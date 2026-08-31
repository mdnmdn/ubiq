//! The application: resolve where things are written, start the one host, open the first window.
//!
//! This is the only crate that names both halves, and it names the host exactly once — to start it
//! and hand the interface the other end of the bus. Everything else lives in the libraries.

use std::path::{Path, PathBuf};

use gpui::{App, KeyBinding, actions};
use gpui_platform::application;

use ubiq::app;
use ubiq_host::config::{self, ConfigRoot};
use ubiq_host::coordinator;
use ubiq_host::projects::Projects;
use ubiq_host::store::file::{FilePreferenceStore, FileProjectStore};
use ubiq_proto::bus;
use ubiq_proto::log;

actions!(ubiq, [Quit]);

fn main() {
    // Before the window, before the host: anything either says on the way up belongs in the
    // console with everything else.
    log::install();

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

    let (projects, pending) = Projects::open(
        root.path.clone(),
        Box::new(FileProjectStore::new(root.path.join("projects.toml"))),
        Box::new(FilePreferenceStore::new(root.path.clone())),
    );

    // One host, started before the first window and outliving every one of them. The catalogue it
    // owns is process-wide, so a host per window would race the store and disagree about what
    // exists.
    let (hub, host) = bus::hub();
    coordinator::start(host, root, projects, pending);

    application()
        .with_assets(gpui_component_assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            // Sets both palettes at once: Ubiq's tokens and the component library's theme.
            theme_boot(cx);
            // Before any window: `open_project_window` takes the window's connection from here.
            app::BusHub::install(hub, cx);

            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("ctrl-q", Quit, None),
            ]);

            // Nothing is open yet: the window asks the host what exists and points itself at the
            // most recently opened, or at nothing when the catalogue is empty.
            app::open_first_window(cx);

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
