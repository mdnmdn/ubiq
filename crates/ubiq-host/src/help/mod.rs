//! Ubiq's own documentation, found once and unpacked into the shared workarea.
//!
//! A help bundle is built by `_tools/helpbundle.py`, ships as one `UBIQBND1` file (the same
//! container [`crate::web_assets::archive`] writes for a vendor bundle — [`archive`] here is the
//! reader for it, ported from the UI crate's because this crate may not depend on that one) and,
//! unlike a vendor bundle, is never fetched: it either shipped beside this build or it did not, and
//! nothing here touches a network.
//!
//! ```text
//! resolution, first hit wins:
//!   UBIQ_HELP_BUNDLE                                   an explicit path, for tests and authors
//!   <exe dir>/../Resources/help.bundle                 inside a macOS .app
//!   <exe dir>/help.bundle                               beside the binary, every other platform
//!   target/help/help.bundle, walking up from the exe    the development case
//!   nothing                                            HelpUnavailable
//!
//! <shared workarea>/help/<version>/           the unpacked root, plain files from here on
//! <shared workarea>/help/<version>.part/      where an extraction in progress writes
//! ```
//!
//! **The rename is the only proof of done**, on [`crate::web_assets`]'s own discipline: every entry
//! lands under a sibling `<version>.part/` directory, and only the whole directory's rename over
//! `<version>/` says the unpack finished. A process killed mid-extract leaves a `.part` directory
//! and no finished one, exactly as unfinished as it looks, and the next `ensure` overwrites it.
//!
//! **A version already on disk is used as-is, with no re-extract** — [`Help::ensure`] only opens
//! the bundle far enough to read `catalog.json` and learn the version before deciding whether
//! there is any unpacking left to do. Older version directories are removed, but only once a new
//! extract has actually succeeded: a bundle that fails to unpack must never take the working
//! version down with it.
//!
//! **Extraction runs inline, on whichever thread calls [`Help::ensure`] — the coordinator's.**
//! [`crate::web_assets`] earns its own worker thread and a watcher list because it is a network
//! fetch of up to tens of megabytes across hundreds of files, joined by however many windows ask
//! while it is in flight. A help bundle has none of that shape: it is already local (no network),
//! it is a few dozen small markdown files typically well under a megabyte altogether, and inflating
//! them is sub-millisecond work. [`Help`] does it at most once per process — the outcome is cached
//! in `state` and every later `ensure` re-answers from it — so the one time it runs, it is a
//! bounded, one-off cost on the coordinator's thread, not a recurring one, and a thread plus a
//! channel to hand that single result back would be bookkeeping with nothing to buy.
//!
//! **Never an error reply.** A missing or unreadable bundle answers
//! [`ubiq_proto::messages::Message::HelpUnavailable`] with a sentence fit to show a user — help
//! that was never built is the expected state for a long time, not a fault.

pub mod archive;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use ubiq_proto::bus::Mailbox;
use ubiq_proto::help::HelpCatalog;
use ubiq_proto::messages::Message;

/// The directory under the shared workarea every version lands beside.
const HELP_DIR: &str = "help";

/// The env var an explicit bundle path is read from — for tests, and for a content author
/// iterating on a bundle without installing it anywhere.
const ENV_BUNDLE: &str = "UBIQ_HELP_BUNDLE";

/// The sentence shown when this build carries no help content at all.
const NO_BUNDLE: &str =
    "No help content is bundled with this build. Run `just help-bundle` to build it.";

/// What [`Help::ensure`] answers with once it knows, held so a second ask costs nothing.
#[derive(Clone)]
enum Outcome {
    Ready { root: String, catalog: HelpCatalog },
    Unavailable { reason: String },
}

impl Outcome {
    fn into_message(self) -> Message {
        match self {
            Outcome::Ready { root, catalog } => Message::HelpReady {
                root,
                catalog: Box::new(catalog),
            },
            Outcome::Unavailable { reason } => Message::HelpUnavailable { reason },
        }
    }
}

/// The help family, as the coordinator holds it: where the shared workarea is, and the answer
/// found there, once.
pub struct Help {
    shared: PathBuf,
    state: Mutex<Option<Outcome>>,
}

impl Help {
    pub fn new(shared: PathBuf) -> Self {
        Self {
            shared,
            state: Mutex::new(None),
        }
    }

    /// Make sure the documentation is unpacked, and answer the asker.
    ///
    /// The first call resolves a bundle, extracts it if needed, and parses its catalogue; every
    /// call after that re-answers from what was found, without touching disk again. There is no
    /// third outcome: whatever went wrong on the way, the answer is
    /// [`Message::HelpUnavailable`] and never a bare error.
    pub fn ensure(&self, asker: &Mailbox) {
        asker.send(self.resolve().into_message());
    }

    /// The catalogue and the root its pages sit under, resolved the same way [`Self::ensure`]
    /// does and from the same cache — `None` when this build has no documentation to show.
    ///
    /// What the `ubiq-help` MCP tools read: they need the data itself, not a message addressed to
    /// a window.
    pub fn ready(&self) -> Option<(String, HelpCatalog)> {
        match self.resolve() {
            Outcome::Ready { root, catalog } => Some((root, catalog)),
            Outcome::Unavailable { .. } => None,
        }
    }

    /// Why there is nothing to show, when there is nothing — the sentence [`Self::ready`]'s
    /// `None` doesn't carry.
    pub fn unavailable_reason(&self) -> Option<String> {
        match self.resolve() {
            Outcome::Ready { .. } => None,
            Outcome::Unavailable { reason } => Some(reason),
        }
    }

    fn resolve(&self) -> Outcome {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(outcome) = state.as_ref() {
            return outcome.clone();
        }
        let outcome = build(&self.shared);
        *state = Some(outcome.clone());
        outcome
    }
}

/// Resolve a bundle and get it unpacked, or say why there is nothing to show.
fn build(shared: &Path) -> Outcome {
    let Some(bundle_path) = resolve_bundle_path() else {
        return Outcome::Unavailable {
            reason: NO_BUNDLE.to_string(),
        };
    };
    match unpack(&bundle_path, &shared.join(HELP_DIR)) {
        Ok((root, catalog)) => Outcome::Ready { root, catalog },
        Err(reason) => Outcome::Unavailable { reason },
    }
}

/// First hit wins: an explicit override, then beside the binary (the macOS `.app` shape and the
/// plain one), then walking up from the executable for the development build's `target/help/`.
fn resolve_bundle_path() -> Option<PathBuf> {
    let env = std::env::var(ENV_BUNDLE).ok();
    let exe = std::env::current_exe().ok()?;
    resolve_bundle_from(env.as_deref(), &exe)
}

/// The pure half of [`resolve_bundle_path`], taking the env var and the executable path as
/// arguments so a test can drive it against a fixture rather than the real process environment.
fn resolve_bundle_from(env_override: Option<&str>, exe: &Path) -> Option<PathBuf> {
    if let Some(path) = env_override {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }

    let dir = exe.parent()?;

    let macos_bundle = dir.join("../Resources/help.bundle");
    if macos_bundle.is_file() {
        return Some(macos_bundle);
    }

    let beside = dir.join("help.bundle");
    if beside.is_file() {
        return Some(beside);
    }

    let mut ancestor = Some(dir);
    while let Some(candidate_dir) = ancestor {
        let candidate = candidate_dir.join("target/help/help.bundle");
        if candidate.is_file() {
            return Some(candidate);
        }
        ancestor = candidate_dir.parent();
    }

    None
}

/// Get the bundle's catalogue and, if the version it names is not already unpacked, extract it.
fn unpack(bundle_path: &Path, help_dir: &Path) -> Result<(String, HelpCatalog), String> {
    let mut reader = archive::Reader::open(bundle_path)?;
    let catalog_bytes = reader
        .read("catalog.json")
        .ok_or_else(|| "the help bundle is missing its catalog.json".to_string())?;
    let catalog: HelpCatalog = serde_json::from_slice(&catalog_bytes)
        .map_err(|error| format!("the help bundle's catalog.json does not parse: {error}"))?;

    let version_dir = help_dir.join(&catalog.version);
    if version_dir.is_dir() {
        return Ok((version_dir.display().to_string(), catalog));
    }

    extract(&mut reader, help_dir, &catalog.version)?;
    remove_other_versions(help_dir, &catalog.version);
    Ok((version_dir.display().to_string(), catalog))
}

/// Every entry into `<help_dir>/<version>.part/`, then the rename that is the only proof it is
/// done.
fn extract(reader: &mut archive::Reader, help_dir: &Path, version: &str) -> Result<(), String> {
    let part_dir = help_dir.join(format!("{version}.part"));
    // A leftover from a process killed mid-extract is never trusted, only overwritten.
    let _ = std::fs::remove_dir_all(&part_dir);
    std::fs::create_dir_all(&part_dir)
        .map_err(|error| format!("{}: {error}", part_dir.display()))?;

    let paths: Vec<String> = reader.entries().map(str::to_string).collect();
    for path in &paths {
        let bytes = reader
            .read(path)
            .ok_or_else(|| format!("{path}: could not be read back out of the bundle"))?;
        let dest = join_entry(&part_dir, path);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        std::fs::write(&dest, &bytes).map_err(|error| format!("{}: {error}", dest.display()))?;
    }

    let version_dir = help_dir.join(version);
    std::fs::rename(&part_dir, &version_dir)
        .map_err(|error| format!("{}: {error}", version_dir.display()))
}

/// Turn a forward-slashed archive path into a real one, component by component — the entries a
/// bundle carries are never trusted to be a single valid path segment on every platform.
fn join_entry(dir: &Path, entry: &str) -> PathBuf {
    let mut path = dir.to_path_buf();
    for part in entry.split('/') {
        path.push(part);
    }
    path
}

/// Remove every version directory under `help_dir` except `keep`, called only after a fresh
/// extract of `keep` has succeeded — never before, so a failed extract leaves the working version
/// standing.
fn remove_other_versions(help_dir: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(help_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_str() == Some(keep) {
            continue;
        }
        if entry.path().is_dir() {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use ubiq_proto::bus::{self, To};

    /// `UBIQ_HELP_BUNDLE` is one process-global variable; anything that sets it holds this for
    /// the duration, so two tests racing to set and clear it cannot interleave.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn write_help_bundle(path: &Path, version: &str, pages: &[(&str, &str)]) {
        let catalog = serde_json::json!({
            "version": version,
            "namespace": "ubiq",
            "pages": pages.iter().map(|(id, path)| serde_json::json!({
                "id": id, "path": path, "title": id, "summary": "",
            })).collect::<Vec<_>>(),
            "nav": pages.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            "context": {},
            "redirects": {},
        });
        let catalog_bytes = serde_json::to_vec(&catalog).unwrap();
        let mut entries: Vec<(&str, &[u8])> = vec![("catalog.json", &catalog_bytes)];
        for &(_, page_path) in pages {
            entries.push((page_path, b"# a help page" as &[u8]));
        }
        archive::fixture::write_bundle(path, &entries);
    }

    // ── resolution order ────────────────────────────────────────────

    #[test]
    fn an_explicit_env_path_wins_over_everything_else() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("somewhere.bundle");
        std::fs::write(&bundle, b"anything").unwrap();
        let exe = dir.path().join("does-not-matter/ubiq");

        assert_eq!(
            resolve_bundle_from(Some(bundle.to_str().unwrap()), &exe),
            Some(bundle)
        );
    }

    #[test]
    fn an_explicit_env_path_that_is_not_a_file_resolves_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.bundle");
        let exe = dir.path().join("ubiq");
        assert_eq!(
            resolve_bundle_from(Some(missing.to_str().unwrap()), &exe),
            None
        );
    }

    #[test]
    fn a_macos_app_bundle_is_found_relative_to_the_executable() {
        let dir = tempfile::tempdir().unwrap();
        let macos = dir.path().join("Ubiq.app/Contents/MacOS");
        let resources = dir.path().join("Ubiq.app/Contents/Resources");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::create_dir_all(&resources).unwrap();
        let bundle = resources.join("help.bundle");
        std::fs::write(&bundle, b"anything").unwrap();
        let exe = macos.join("ubiq");

        let found = resolve_bundle_from(None, &exe).unwrap();
        assert!(found.is_file());
    }

    #[test]
    fn a_bundle_beside_the_binary_is_found_when_there_is_no_app_shape() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("help.bundle");
        std::fs::write(&bundle, b"anything").unwrap();
        let exe = dir.path().join("ubiq");

        assert_eq!(resolve_bundle_from(None, &exe), Some(bundle));
    }

    #[test]
    fn target_help_is_found_by_walking_up_from_the_executable() {
        let dir = tempfile::tempdir().unwrap();
        let target_help = dir.path().join("target/help");
        std::fs::create_dir_all(&target_help).unwrap();
        let bundle = target_help.join("help.bundle");
        std::fs::write(&bundle, b"anything").unwrap();
        // As deep as a debug binary actually sits: <root>/target/debug/ubiq-app.
        let exe = dir.path().join("target/debug/ubiq-app");

        assert_eq!(resolve_bundle_from(None, &exe), Some(bundle));
    }

    #[test]
    fn nothing_found_anywhere_resolves_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("ubiq");
        assert_eq!(resolve_bundle_from(None, &exe), None);
    }

    // ── Help::ensure ────────────────────────────────────────────────

    fn window() -> (bus::Hub, bus::HostEnd, bus::Client) {
        let (hub, host) = bus::hub();
        let client = hub.connect();
        (hub, host, client)
    }

    #[test]
    fn a_missing_bundle_answers_unavailable_never_an_error() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_hub, host, client) = window();
        let shared = tempfile::tempdir().unwrap();
        // An explicit override that names nothing on disk wins over every other resolution step,
        // so this is deterministic regardless of whatever this build's own `target/help/` holds —
        // and it is a real one, in this very tree.
        let missing = shared.path().join("nope.bundle");
        unsafe {
            std::env::set_var(ENV_BUNDLE, &missing);
        }
        let help = Help::new(shared.path().to_path_buf());
        help.ensure(&host.mailbox(To::Client(client.id())));
        unsafe {
            std::env::remove_var(ENV_BUNDLE);
        }

        match client
            .from_host()
            .recv_timeout(std::time::Duration::from_secs(5))
        {
            Ok(Message::HelpUnavailable { reason }) => {
                assert!(reason.contains("help-bundle"), "{reason}");
            }
            other => panic!("expected HelpUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn a_bundle_named_by_env_is_unpacked_once_and_reused() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_hub, host, client) = window();
        let shared = tempfile::tempdir().unwrap();
        let bundle_dir = tempfile::tempdir().unwrap();
        let bundle = bundle_dir.path().join("help.bundle");
        write_help_bundle(&bundle, "2026.09.1", &[("index", "index.md")]);

        unsafe {
            std::env::set_var(ENV_BUNDLE, &bundle);
        }
        let help = Help::new(shared.path().to_path_buf());
        help.ensure(&host.mailbox(To::Client(client.id())));

        let root = match client
            .from_host()
            .recv_timeout(std::time::Duration::from_secs(5))
        {
            Ok(Message::HelpReady { root, catalog }) => {
                assert_eq!(catalog.version, "2026.09.1");
                assert_eq!(catalog.pages.len(), 1);
                root
            }
            other => panic!("expected HelpReady, got {other:?}"),
        };
        assert!(Path::new(&root).join("index.md").is_file());
        assert!(Path::new(&root).join("catalog.json").is_file());

        // A second ask re-answers from the cache — remove the bundle first, so a re-extract would
        // fail loudly if one were attempted.
        std::fs::remove_file(&bundle).unwrap();
        help.ensure(&host.mailbox(To::Client(client.id())));
        match client
            .from_host()
            .recv_timeout(std::time::Duration::from_secs(5))
        {
            Ok(Message::HelpReady { catalog, .. }) => assert_eq!(catalog.version, "2026.09.1"),
            other => panic!("expected the cached HelpReady, got {other:?}"),
        }

        unsafe {
            std::env::remove_var(ENV_BUNDLE);
        }
    }

    #[test]
    fn extracting_a_second_version_removes_the_first() {
        let shared = tempfile::tempdir().unwrap();
        let help_dir = shared.path().join(HELP_DIR);
        let bundle_dir = tempfile::tempdir().unwrap();

        let first = bundle_dir.path().join("v1.bundle");
        write_help_bundle(&first, "v1", &[("index", "index.md")]);
        let (root_v1, _) = unpack(&first, &help_dir).unwrap();
        assert!(Path::new(&root_v1).is_dir());

        let second = bundle_dir.path().join("v2.bundle");
        write_help_bundle(&second, "v2", &[("index", "index.md")]);
        let (root_v2, _) = unpack(&second, &help_dir).unwrap();

        assert!(Path::new(&root_v2).is_dir());
        assert!(
            !Path::new(&root_v1).exists(),
            "the older version is removed"
        );
    }

    #[test]
    fn a_version_already_on_disk_is_reused_without_touching_the_bundle_again() {
        let shared = tempfile::tempdir().unwrap();
        let help_dir = shared.path().join(HELP_DIR);
        let bundle_dir = tempfile::tempdir().unwrap();
        let bundle = bundle_dir.path().join("v1.bundle");
        write_help_bundle(&bundle, "v1", &[("index", "index.md")]);

        let (root, _) = unpack(&bundle, &help_dir).unwrap();
        // Corrupt the file so a re-extract would surface as a different error or content.
        std::fs::write(Path::new(&root).join("index.md"), b"already there").unwrap();

        let (root_again, catalog_again) = unpack(&bundle, &help_dir).unwrap();
        assert_eq!(root_again, root);
        assert_eq!(catalog_again.version, "v1");
        assert_eq!(
            std::fs::read_to_string(Path::new(&root_again).join("index.md")).unwrap(),
            "already there",
            "the existing directory was used as-is, not re-extracted"
        );
    }
}
