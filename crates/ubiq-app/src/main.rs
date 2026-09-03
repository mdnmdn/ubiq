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
use ubiq_host::settings::Settings;
use ubiq_host::store::file::{
    FilePreferenceStore, FileProjectStore, FileSettingsStore, FileTaskStore,
};
use ubiq_host::work::Work;
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
    // Nothing is read here: a project's tasks are opened the first time a window asks for
    // them, unlike the catalogue, which every window needs to draw a picker at boot.
    let work = Work::open(Box::new(FileTaskStore::new(root.path.clone())));
    let settings = Settings::open(Box::new(FileSettingsStore::new(root.path.clone())));

    let (hub, host) = bus::hub();
    coordinator::start(host, root, projects, work, settings, pending);

    // The three ways a path reaches Ubiq from outside its own window — `ubiq <path>` on the
    // command line, a Finder or dock-icon open, a drop on the app icon while it is running — all
    // funnel through here, so whichever window answers sees exactly the same thing.
    let (path_tx, path_rx) = flume::unbounded::<PathBuf>();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for path in argv_paths(std::env::args().skip(1), &cwd) {
        path_tx.send(path).ok();
    }

    // `on_open_urls` and `on_reopen` are `Application`'s, not `App`'s, so they have to go on
    // before `run` — and both take `&self`, so this cannot be one chain with `run`, which takes
    // `self`.
    let app = application().with_assets(gpui_component_assets::Assets);

    app.on_open_urls(move |urls| {
        for url in &urls {
            if let Some(path) = path_from_file_url(url) {
                path_tx.send(path).ok();
            }
        }
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

        cx.on_action(|_: &Quit, cx| cx.quit());
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
            while let Ok(first) = path_rx.recv_async().await {
                let mut batch = vec![first];
                while let Ok(next) = path_rx.try_recv() {
                    batch.push(next);
                }
                cx.update(|cx| app::deliver_paths_to_a_window(batch, cx));
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
