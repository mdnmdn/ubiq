//! The application: resolve where things are written, start the one host, open the first window.
//!
//! This is the only crate that names both halves, and it names the host exactly once — to start it
//! and hand the interface the other end of the bus. Everything else lives in the libraries.
//!
//! [`run`] is the whole boot, and [`Boot`] is its only input, so a second edition composing these
//! same crates cannot re-implement or skip a step of it — it hands in a different value. The base
//! is `Boot::default()`, which is not a reduced configuration but the thing that ships: a base
//! behaviour that only works because something was registered is a bug `just verify` catches by
//! never registering anything.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};

use gpui::{App, KeyBinding, actions};
use gpui_platform::application;

use ubiq::app;
use ubiq_host::config::{self, ConfigRoot};
use ubiq_host::coordinator;
use ubiq_host::projects::Projects;
use ubiq_host::settings::Settings;
use ubiq_host::store::file::{
    FilePreferenceStore, FileProjectStore, FileSettingsStore, FileTaskStore,
};
use ubiq_host::store::{PreferenceStore, ProjectStore, SettingsStore, TaskStore};
use ubiq_host::work::Work;
use ubiq_proto::bus;
use ubiq_proto::log;

pub mod handoff;

actions!(ubiq, [Quit]);

/// Where the four subsystems keep what they own.
///
/// Each is already a boxed trait with a file and a memory implementation, so this struct adds no
/// indirection — it only gathers the four `Box::new` calls into one place a caller can replace.
/// An edition that encrypts at rest hands in a decorator over the file stores and changes nothing
/// else.
pub struct Stores {
    pub projects: Box<dyn ProjectStore>,
    pub preferences: Box<dyn PreferenceStore>,
    pub tasks: Box<dyn TaskStore>,
    pub settings: Box<dyn SettingsStore>,
}

impl Stores {
    /// The file stores under a resolved config root. What the base ships.
    pub fn files(root: &Path) -> Self {
        Stores {
            projects: Box::new(FileProjectStore::new(root.join("projects.toml"))),
            preferences: Box::new(FilePreferenceStore::new(root.to_path_buf())),
            tasks: Box::new(FileTaskStore::new(root.to_path_buf())),
            settings: Box::new(FileSettingsStore::new(root.to_path_buf())),
        }
    }
}

/// Everything one edition decides before the first window.
///
/// The config root is resolved inside [`run`], from this process's own arguments, so the stores
/// arrive as a function of it rather than as values — which is also what lets an edition wrap one
/// with a key it fetched during the same boot.
pub struct Boot {
    pub stores: Box<dyn FnOnce(&Path) -> Stores>,
}

impl Default for Boot {
    fn default() -> Self {
        Boot {
            stores: Box::new(Stores::files),
        }
    }
}

/// The boot, entire: the console, the config root, the stores, the one host, the path intake,
/// the component library and the palette, and the first window.
pub fn run(boot: Boot) {
    // Before the window, before the host: anything either says on the way up belongs in the
    // console with everything else.
    log::install();

    // Before any thread exists — the coordinator's, the GPUI event loop's, anything else this
    // process spawns — because mutating the environment is only sound while nothing else might
    // read it concurrently. This is the earliest point in the whole boot, which is exactly why it
    // lives here rather than in the host itself: `coordinator::start` below already spawns a
    // thread. See `ubiq_host::shells::repair_path` for what this actually fixes (a desktop
    // launcher's thin `PATH` breaking every bare-name harness spawn) and why it is safe here.
    ubiq_host::shells::repair_path();

    let root = match resolve_root() {
        Ok(root) => root,
        // A broken bootstrap must not fall back to the user's real catalogue and credentials, so
        // there is nothing to do but say so and stop.
        Err(error) => {
            eprintln!("ubiq: {error:#}");
            std::process::exit(2);
        }
    };
    tracing::info!(
        "config root {} ({})",
        root.path.display(),
        if root.is_default() {
            "the default"
        } else {
            "not the default"
        }
    );

    if let Err(error) = std::fs::create_dir_all(&root.path) {
        // Not fatal: the session runs, and the catalogue says once that it is not durable.
        tracing::error!("could not create {}: {error}", root.path.display());
    }

    // The three ways a path reaches Ubiq from outside its own window — `ubiq <path>` on the
    // command line, a Finder or dock-icon open, a drop on the app icon while it is running — all
    // funnel through one channel, so whichever window answers sees exactly the same thing.
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let paths = argv_paths(std::env::args().skip(1), &cwd);

    // Before anything is opened or started: a second `ubiq` under the same config root is a second
    // *process*, not a second application. It gives its paths to the one already running and is
    // done here — the shell it was typed in gets its prompt straight back.
    let listener = match handoff::claim(&root.path, &paths) {
        handoff::Handoff::Delivered => return,
        handoff::Handoff::Owner(listener) => listener,
    };

    let stores = (boot.stores)(&root.path);

    let (projects, pending) =
        Projects::open(root.path.clone(), stores.projects, stores.preferences);

    // One host, started before the first window and outliving every one of them. The catalogue it
    // owns is process-wide, so a host per window would race the store and disagree about what
    // exists.
    // Nothing is read here: a project's tasks are opened the first time a window asks for
    // them, unlike the catalogue, which every window needs to draw a picker at boot.
    let work = Work::open(stores.tasks);
    let settings = Settings::open(stores.settings);

    let (hub, host) = bus::hub();
    coordinator::start(host, root, projects, work, settings, pending);

    // After the coordinator, so a client that attaches in the same breath has something to talk
    // to, and before the first window, so a `--serve` run that never opens one still serves.
    if let Some(bind) = serve_bind(std::env::args().skip(1)) {
        match ubiq_host::remote::serve(hub.clone(), bind) {
            Ok(serving) => announce(&serving),
            // The user asked for a server. Falling through into a window would leave them with
            // something that looks like it worked and is not listening to anything.
            Err(error) => {
                eprintln!("ubiq: could not listen on {bind}: {error}");
                std::process::exit(2);
            }
        }
    }

    // One batch per arrival, because an arrival with no path in it is a bare `ubiq` asking for the
    // window's attention and has to reach the loop below all the same.
    let (path_tx, path_rx) = flume::unbounded::<Vec<PathBuf>>();
    path_tx.send(paths).ok();
    if let Some(listener) = listener {
        handoff::serve(listener, path_tx.clone());
    }

    // `on_open_urls` and `on_reopen` are `Application`'s, not `App`'s, so they have to go on
    // before `run` — and both take `&self`, so this cannot be one chain with `run`, which takes
    // `self`.
    let app = application().with_assets(gpui_component_assets::Assets);

    app.on_open_urls(move |urls| {
        path_tx
            .send(
                urls.iter()
                    .filter_map(|url| path_from_file_url(url))
                    .collect(),
            )
            .ok();
    });

    // The dock icon, clicked with no window open: the same door a cold launch walks through.
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            app::open_first_window(cx);
        }
    });

    app.run(|cx: &mut App| {
        gpui_component::init(cx);
        // Fills the highlight-query gap gpui-component leaves for Swift and C#.
        ubiq::ui::editor::register_extra_languages();
        // Sets both palettes at once: Ubiq's tokens and the component library's theme.
        theme_boot(cx);
        // Before any window: `open_project_window` takes the window's connection from here.
        app::BusHub::install(hub, cx);

        // Unsaved work is the interface's to answer for, so ⌘Q is a question first: the window
        // holding it raises the same dialog its close button does, and quits on the yes.
        cx.on_action(|_: &Quit, cx| {
            if app::quit_requested(cx) {
                cx.quit();
            }
        });
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, None),
        ]);
        // Quit is the application's; everything else a window answers to belongs to the
        // interface, which binds it beside the action it dispatches.
        app::install_key_bindings(cx);

        // Nothing is open yet: the window asks the host what exists and points itself at the
        // most recently opened, or at nothing when the catalogue is empty.
        app::open_first_window(cx);

        // Whatever is already queued — argv paths from this launch, or a Finder open that beat
        // the window to existing — is drained here, and every later arrival takes the same path.
        // `deliver_paths_to_a_window` owns what happens to them; this just says when.
        cx.spawn(async move |cx| {
            while let Ok(batch) = path_rx.recv_async().await {
                cx.update(|cx| {
                    // Whoever asked is in another process, or in Finder: the window they meant has
                    // to come forward, whether or not a path came with the ask.
                    cx.activate(true);
                    app::deliver_paths_to_a_window(batch, cx)
                });
            }
        })
        .detach();

        // A closed window leaves the registry, and the application ends with its last one —
        // not with any particular one.
        cx.on_window_closed(|cx, window_id| {
            app::window_closed(window_id, cx);
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.activate(true);
    });
}

/// Positional arguments as paths — `ubiq .`, `ubiq some/file` — reaching the same place a Finder
/// open would. `--config-root` and its value are the only two tokens spoken for; anything else
/// that looks like a flag is left alone, on the same trust the platform's own arguments get.
///
/// Relative paths are made absolute against `cwd`: the interface hands them straight to
/// `deliver_paths`, which never sees the launch directory to resolve them against itself.
fn argv_paths(mut args: impl Iterator<Item = String>, cwd: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    while let Some(arg) = args.next() {
        if arg == "--config-root" {
            args.next(); // the flag's value, not ours
            continue;
        }
        if arg.starts_with("--config-root=") || arg.starts_with('-') {
            continue;
        }
        paths.push(cwd.join(arg));
    }
    paths
}

/// The port a `--serve` with no address of its own lands on.
const SERVE_PORT: u16 = 7420;

/// Where `--serve` should listen, or `None` when it was not asked for.
///
/// `--serve` alone binds every interface, which is the point of the flag; `--serve=<addr>` narrows
/// it, and `--serve=127.0.0.1:7420` is how a tunnel-only setup is spelled. The value is attached
/// with `=` and never separated by a space: `argv_paths` skips a token because it starts with `-`
/// and has no way to know the next one belongs to it, so a separated value would be read as a
/// project path. `--config-root` can afford the separated form because it is consumed there by
/// name; a second such flag is a parser, and this codebase has decided it does not want one.
fn serve_bind(args: impl Iterator<Item = String>) -> Option<SocketAddr> {
    for arg in args {
        if arg == "--serve" {
            return Some(SocketAddr::from((Ipv4Addr::UNSPECIFIED, SERVE_PORT)));
        }
        if let Some(value) = arg.strip_prefix("--serve=") {
            return Some(parse_bind(value).unwrap_or_else(|| {
                eprintln!("ubiq: --serve={value} is not an address or a port");
                std::process::exit(2);
            }));
        }
    }
    None
}

/// A bare port, an address with a port, or an address on the default port.
fn parse_bind(value: &str) -> Option<SocketAddr> {
    if let Ok(port) = value.parse::<u16>() {
        return Some(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)));
    }
    if let Ok(addr) = value.parse::<SocketAddr>() {
        return Some(addr);
    }
    value
        .parse::<IpAddr>()
        .ok()
        .map(|ip| SocketAddr::new(ip, SERVE_PORT))
}

/// What to print as `<ip>` in the connection string.
///
/// Bound to one interface, that address is the answer. Bound to all of them, `0.0.0.0` is useless
/// to paste, so ask the routing table which source address it would use to leave this machine —
/// a connected UDP socket assigns one without sending a packet or needing a reachable peer, which
/// is why the address it is pointed at does not matter and no traffic is generated. A machine with
/// no route out has nothing to advertise, and the bind address is printed unchanged.
fn advertised_ip(addr: SocketAddr) -> IpAddr {
    if !addr.ip().is_unspecified() {
        return addr.ip();
    }
    UdpSocket::bind(("0.0.0.0", 0))
        .and_then(|socket| {
            socket.connect(("192.0.2.1", 53))?;
            socket.local_addr()
        })
        .map(|local| local.ip())
        .unwrap_or_else(|_| addr.ip())
}

/// The banner: where it is listening, the token, and the string to paste into a UI.
///
/// Printed to stdout rather than logged, because it is the whole output of a `--serve` run and
/// must not depend on a log filter. The warning is here because the flag hands a shell on this
/// machine to whoever holds the token — said plainly, once, rather than buried in a document.
fn announce(serving: &ubiq_host::remote::Serving) {
    let addr = serving.addr;
    let ip = advertised_ip(addr);
    println!("ubiq host listening on {addr}");
    println!();
    println!("  token  {}", serving.token);
    println!("  connect  http://{ip}:{}?token={}", addr.port(), serving.token);
    println!();
    println!("Anyone who reaches this port with that token gets a terminal on this machine.");
    println!("The connection is not encrypted — tunnel it if the network is not trusted.");
}

/// A `file://` URL as Finder or a dock-icon drop hands it over, decoded back to a path. macOS
/// marks a folder with a trailing slash; the path itself never wants one. Anything not `file://`
/// is not ours — Ubiq registers no URL scheme of its own.
fn path_from_file_url(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let decoded = percent_encoding::percent_decode_str(rest).decode_utf8_lossy();
    Some(PathBuf::from(decoded.strip_suffix('/').unwrap_or(&decoded)))
}

fn theme_boot(cx: &mut App) {
    ubiq::theme::set_mode(app::boot_theme(), cx);
}

/// Where everything is written down: `--config-root`, then `UBIQ_CONFIG_DIR`, then the nearest
/// `ubiq.toml`, then `~/.config/ubiq`.
///
/// One flag does not earn a command-line parser, and Ubiq has no other. Anything unrecognised is
/// left alone: the platform passes arguments of its own.
fn resolve_root() -> anyhow::Result<ConfigRoot> {
    let flag = config_root_flag(std::env::args().skip(1));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    config::resolve_from_env(flag.as_deref(), &cwd)
}

fn config_root_flag(args: impl Iterator<Item = String>) -> Option<PathBuf> {
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--config-root=") {
            return Some(PathBuf::from(value));
        }
        if arg == "--config-root" {
            return args
                .next()
                .map(PathBuf::from)
                .filter(|p| p != Path::new(""));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of `Stores`: an edition substitutes one without touching the boot. If this stops
    /// compiling, tier 0 of `inbox-editions` has stopped being reachable.
    #[test]
    fn an_edition_can_hand_in_stores_of_its_own() {
        use ubiq_host::store::memory::{
            MemoryPreferenceStore, MemoryProjectStore, MemorySettingsStore, MemoryTaskStore,
        };

        let boot = Boot {
            stores: Box::new(|_root| Stores {
                projects: Box::new(MemoryProjectStore::new()),
                preferences: Box::new(MemoryPreferenceStore::new()),
                tasks: Box::new(MemoryTaskStore::new()),
                settings: Box::new(MemorySettingsStore::new()),
            }),
        };

        // The base is the default, not one configuration of several.
        let base = Boot::default();
        for stores in [
            (boot.stores)(Path::new("/nowhere")),
            (base.stores)(Path::new("/nowhere")),
        ] {
            let (projects, pending) = ubiq_host::projects::Projects::open(
                PathBuf::from("/nowhere"),
                stores.projects,
                stores.preferences,
            );
            drop((projects, pending, stores.tasks, stores.settings));
        }
    }

    fn argv(args: &[&str]) -> impl Iterator<Item = String> {
        args.iter().map(|a| a.to_string()).collect::<Vec<_>>().into_iter()
    }

    /// Not asked for is the default, and every other flag stays somebody else's.
    #[test]
    fn without_the_flag_nothing_listens() {
        assert_eq!(serve_bind(argv(&[])), None);
        assert_eq!(serve_bind(argv(&["--config-root", "/tmp/x", "."])), None);
    }

    /// The bare flag is the one that binds every interface — that is what the flag is for.
    #[test]
    fn the_bare_flag_binds_every_interface_on_the_default_port() {
        let bind = serve_bind(argv(&["--serve"])).expect("a bind");
        assert!(bind.ip().is_unspecified());
        assert_eq!(bind.port(), SERVE_PORT);
    }

    /// The three shapes a value may take, including the loopback one a tunnel-only setup uses.
    #[test]
    fn a_value_may_be_a_port_an_address_or_both() {
        assert_eq!(
            serve_bind(argv(&["--serve=9000"])).map(|b| b.port()),
            Some(9000)
        );
        let loopback = serve_bind(argv(&["--serve=127.0.0.1:7420"])).expect("a bind");
        assert!(loopback.ip().is_loopback());
        assert_eq!(loopback.port(), 7420);
        let bare_ip = serve_bind(argv(&["--serve=127.0.0.1"])).expect("a bind");
        assert!(bare_ip.ip().is_loopback());
        assert_eq!(bare_ip.port(), SERVE_PORT);
    }

    /// The reason the value is attached with `=`: a separated one would be read as a project path,
    /// and a path silently becoming a bind address is worse than the flag not taking that form.
    #[test]
    fn the_flag_and_its_attached_value_are_never_mistaken_for_paths() {
        let cwd = Path::new("/work");
        assert!(argv_paths(argv(&["--serve"]), cwd).is_empty());
        assert!(argv_paths(argv(&["--serve=0.0.0.0:7420"]), cwd).is_empty());
        assert_eq!(
            argv_paths(argv(&["--serve", "."]), cwd),
            vec![PathBuf::from("/work/.")]
        );
    }

    /// An unspecified bind is not something anyone can paste, so the string carries a real one.
    #[test]
    fn the_advertised_address_is_never_the_unspecified_one() {
        let advertised = advertised_ip(SocketAddr::from((Ipv4Addr::UNSPECIFIED, SERVE_PORT)));
        // A sandbox with no route out has nothing better to offer, and says so by handing the
        // bind address back rather than by inventing one.
        assert!(!advertised.is_unspecified() || advertised == IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        // Bound to one interface, that interface is the answer, with no guessing at all.
        let pinned = SocketAddr::from((Ipv4Addr::LOCALHOST, 1234));
        assert_eq!(advertised_ip(pinned), IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn positional_arguments_become_paths_and_config_root_is_skipped() {
        let cwd = Path::new("/work");
        let args = [
            "--config-root",
            "/elsewhere",
            "relative/dir",
            "--config-root=/also/elsewhere",
            "/already/absolute",
            "-x",
        ]
        .into_iter()
        .map(String::from);

        assert_eq!(
            argv_paths(args, cwd),
            vec![
                PathBuf::from("/work/relative/dir"),
                PathBuf::from("/already/absolute"),
            ]
        );
    }

    #[test]
    fn a_file_url_decodes_and_loses_its_trailing_slash() {
        assert_eq!(
            path_from_file_url("file:///Users/me/My%20Project/"),
            Some(PathBuf::from("/Users/me/My Project"))
        );
        assert_eq!(path_from_file_url("https://example.com"), None);
    }
}
