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
