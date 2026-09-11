//! The vendor bundles a web panel needs, fetched once and kept in the shared workarea.
//!
//! A web panel's component is 25 MiB of somebody else's JavaScript. Baking that into the
//! executable would inflate every build and every release for a feature most sessions never open,
//! so it is downloaded instead — and downloading is network plus disk, which is this half's.
//! **The interface asks; the host fetches, verifies, unpacks and answers when it is there**, and
//! the interface then serves those bytes off its own loopback origin.
//!
//! ```text
//! <shared workarea>/web/<app>/<version>/…            the mirror, cache path = URL path
//! <shared workarea>/web/<app>/<version>/importmap.json  the map the chrome loads
//! <shared workarea>/web/<app>/<version>/.complete    written last, and the only proof of done
//! ```
//!
//! **Versioned by directory, not invalidated by policy.** [`manifest::VERSION`] is pinned in
//! source, so a build that pins a new one looks in a directory that does not exist, fetches, and
//! leaves the old one alone. There is no TTL and no staleness check: a complete directory is
//! answered immediately and nothing is fetched.
//!
//! **Complete means the marker is there.** A process killed mid-fetch leaves a directory full of
//! real, verified files and no marker, and that must not read as done — the marker is written
//! after the last entry is verified and never before.
//!
//! **Every file is verified against the manifest before it is written.** A hash that does not
//! match fails the whole fetch and caches nothing: a downloader that trusts a CDN to have served
//! the right bytes is a supply-chain hole with a nice interface.
//!
//! **Failure is a downgrade.** No network and nothing cached answers
//! [`Message::WebBundleFailed`] with a sentence, and the feature that wanted the bundle says why
//! it is unavailable. Nothing else is disturbed.

pub mod manifest;

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use ubiq_proto::bus::{ClientId, Mailbox};
use ubiq_proto::messages::Message;

/// The floor between two progress messages, for [`clone`](crate::repos::clone)'s reason: 554 files
/// finishing out of order is more per-file events than an interface can draw or a user can read.
const THROTTLE: Duration = Duration::from_millis(250);

/// How many downloads are in the air at once.
///
/// 554 serial round trips is minutes of latency and 554 threads is a denial of service against a
/// CDN; a handful of connections is what a browser would have opened anyway.
const PARALLEL: usize = 6;

/// The ceiling on one downloaded file, well above the largest thing in the manifest (a 12.7 MB CJK
/// font). A response with no end is not a bundle.
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// The file that says a bundle directory is finished. Dotted, so the interface's own server — which
/// refuses dotfiles — can never serve it.
const MARKER: &str = ".complete";

/// Where the import map lands inside the bundle directory, so the chrome can fetch it by a name it
/// knows without the host telling it one.
pub const IMPORT_MAP_FILE: &str = "importmap.json";

/// One bundle this host knows how to fetch: its name, its pinned version, and every file in it.
///
/// Everything here comes from a generated manifest. It is a struct rather than a set of constants
/// so a second tenant is a second value, and so a test can point the fetcher at a fixture.
pub struct Bundle {
    pub app: &'static str,
    pub version: &'static str,
    pub files: &'static [manifest::Entry],
    pub import_map: &'static str,
}

/// Excalidraw, the first and so far only tenant.
pub const EXCALIDRAW: Bundle = Bundle {
    app: manifest::APP,
    version: manifest::VERSION,
    files: manifest::FILES,
    import_map: manifest::IMPORT_MAP,
};

/// How a file's bytes are obtained.
///
/// A function pointer and not a trait object, because there is exactly one thing to do and one
/// reason to replace it: a test must not reach a CDN. The seam is this and nothing else — hashing,
/// writing, the marker and the throttle are the real code in every run.
pub type Fetch = fn(&str) -> Result<Vec<u8>, String>;

/// The web asset family, as the coordinator holds it.
///
/// It holds no thread and no bytes: one entry per fetch in flight, naming the windows waiting on
/// it, so a second ask joins the first.
pub struct WebAssets {
    /// The directory the host reserved for the interface. Every bundle lands under `web/` here.
    shared: PathBuf,
    bundles: Vec<Bundle>,
    fetch: Fetch,
    /// The fetches in flight, by app. The mailbox list is shared with the worker thread, which is
    /// how a window that asks late is still told.
    running: HashMap<String, Arc<Mutex<Vec<Watcher>>>>,
    finished: flume::Sender<String>,
    done: flume::Receiver<String>,
}

/// One window waiting on a bundle.
struct Watcher {
    client: ClientId,
    mailbox: Mailbox,
}

impl WebAssets {
    /// The real thing: the bundles this build pins, fetched over the network.
    pub fn new(shared: PathBuf) -> Self {
        Self::with(shared, vec![EXCALIDRAW], download)
    }

    /// The same, over a stated set of bundles and a stated fetcher. The one seam a test needs.
    pub fn with(shared: PathBuf, bundles: Vec<Bundle>, fetch: Fetch) -> Self {
        let (finished, done) = flume::unbounded();
        Self {
            shared,
            bundles,
            fetch,
            running: HashMap::new(),
            finished,
            done,
        }
    }

    /// Where a bundle's files live once they are there.
    pub fn directory(&self, bundle: &Bundle) -> PathBuf {
        self.shared
            .join("web")
            .join(bundle.app)
            .join(bundle.version)
    }

    /// Make sure a bundle is on disk, and answer the asker.
    ///
    /// Three outcomes and no fourth: it is already complete and the answer is immediate; a fetch is
    /// already running and this window joins it; or a thread starts. Nothing here touches the
    /// network — the complete check is one `stat`.
    pub fn ensure(&mut self, client: ClientId, app: &str, asker: Mailbox) {
        // Reaped where a fetch is started, on [`crate::repos::Repos::clone`]'s rule: a finished
        // fetch's row must never outlive it, or the next ask would join a thread that has gone and
        // never be answered.
        self.reap();
        let Some(bundle) = self.bundles.iter().find(|bundle| bundle.app == app) else {
            asker.send(Message::WebBundleFailed {
                app: app.to_string(),
                error: format!("no web bundle named {app:?}"),
            });
            return;
        };
        let directory = self
            .shared
            .join("web")
            .join(bundle.app)
            .join(bundle.version);

        if complete(&directory) {
            asker.send(Message::WebBundleReady {
                app: app.to_string(),
                version: bundle.version.to_string(),
                path: directory.to_string_lossy().into_owned(),
            });
            return;
        }

        // Joining rather than starting a second is the whole reason this map exists: two panels
        // opened at once would otherwise race each other over the same 554 files.
        if let Some(watchers) = self.running.get(app) {
            if let Ok(mut watchers) = watchers.lock() {
                watchers.push(Watcher {
                    client,
                    mailbox: asker,
                });
            }
            return;
        }

        let watchers = Arc::new(Mutex::new(vec![Watcher {
            client,
            mailbox: asker,
        }]));
        self.running.insert(app.to_string(), watchers.clone());
        spawn(Job {
            app: app.to_string(),
            version: bundle.version.to_string(),
            files: bundle.files,
            import_map: bundle.import_map,
            directory,
            watchers,
            fetch: self.fetch,
            done: self.finished.clone(),
        });
    }

    /// A window went. It stops being told about a fetch, which carries on for whoever is left.
    pub fn client_gone(&mut self, client: ClientId) {
        for watchers in self.running.values() {
            if let Ok(mut watchers) = watchers.lock() {
                watchers.retain(|watcher| watcher.client != client);
            }
        }
    }

    /// The fetches that ended since this was last asked, so the next ask starts a new one rather
    /// than joining a thread that has gone.
    pub fn reap(&mut self) {
        for app in self.done.try_iter().collect::<Vec<_>>() {
            self.running.remove(&app);
        }
    }
}

/// Whether a bundle directory is finished, which is the marker being there and nothing else.
fn complete(directory: &Path) -> bool {
    directory.join(MARKER).is_file()
}

/// One fetch, addressed and equipped. Everything it needs, so the thread borrows nothing.
struct Job {
    app: String,
    version: String,
    files: &'static [manifest::Entry],
    import_map: &'static str,
    directory: PathBuf,
    /// The windows waiting, shared with the coordinator so one that joins late is still told.
    watchers: Arc<Mutex<Vec<Watcher>>>,
    fetch: Fetch,
    /// Where the end of this fetch is posted, so the coordinator can forget it.
    done: flume::Sender<String>,
}

/// A fetch that does not start is answered, and not only logged: every window waiting on this
/// bundle is holding a loader open, and a `Fetching` that is never resolved is a panel that loads
/// for ever. The coordinator is told the fetch ended for the same reason `run` tells it.
fn spawn(job: Job) {
    let name = format!("ubiq-web-{}", job.app);
    // Kept out of the closure so the answer below can be sent after the move.
    let (app, watchers, done) = (job.app.clone(), job.watchers.clone(), job.done.clone());
    if let Err(error) = thread::Builder::new().name(name).spawn(move || run(job)) {
        tracing::error!("a web bundle fetch did not start: {error}");
        let _ = done.send(app.clone());
        say(
            &watchers,
            Message::WebBundleFailed {
                app,
                error: "the download could not be started".to_string(),
            },
        );
    }
}

/// The order of the last two lines is deliberate. The coordinator is told this fetch has ended
/// *before* the answer goes out, so a window asking again at that moment starts a fresh fetch —
/// which finds the finished directory and answers immediately — rather than joining a thread that
/// is about to end and hearing nothing.
fn run(job: Job) {
    let outcome = fetch_all(&job);
    let _ = job.done.send(job.app.clone());
    match outcome {
        Ok(()) => {
            tracing::info!(
                "web bundle {} {} is at {}",
                job.app,
                job.version,
                job.directory.display()
            );
            tell(
                &job,
                Message::WebBundleReady {
                    app: job.app.clone(),
                    version: job.version.clone(),
                    path: job.directory.to_string_lossy().into_owned(),
                },
            );
        }
        Err(error) => {
            tracing::warn!("web bundle {} is unavailable: {error}", job.app);
            tell(
                &job,
                Message::WebBundleFailed {
                    app: job.app.clone(),
                    error,
                },
            );
        }
    }
}

/// Every file, a handful at a time, then the import map, then the marker.
///
/// The marker is last on purpose: everything above it can be interrupted and resumed, because a
/// file already on disk with the right length and hash is skipped rather than fetched again.
fn fetch_all(job: &Job) -> Result<(), String> {
    let total = job.files.len() as u32;
    if let Err(error) = std::fs::create_dir_all(&job.directory) {
        return Err(format!("{}: {error}", job.directory.display()));
    }

    // The count is known before the first byte, so it goes out before the first byte: a bar that
    // fills from zero for the whole download, rather than one that sweeps for the first quarter
    // second and then jumps. Nothing has landed yet, so there is no file to name.
    tracing::info!("web bundle {} {}: {total} files", job.app, job.version);
    tell(
        job,
        Message::WebBundlePending {
            app: job.app.clone(),
            done: 0,
            total,
            file: String::new(),
        },
    );

    let (queue, work) = flume::unbounded::<&'static manifest::Entry>();
    for entry in job.files {
        let _ = queue.send(entry);
    }
    drop(queue);

    // Each worker reports the cache path of the file it just landed, so a progress report can name
    // it. A failure reports the sentence instead, and takes the whole fetch with it.
    let (report, reports) = flume::unbounded::<Result<&'static str, String>>();
    let stop = Arc::new(AtomicBool::new(false));

    let workers: Vec<_> = (0..PARALLEL.min(job.files.len().max(1)))
        .map(|_| {
            let (work, report, stop) = (work.clone(), report.clone(), stop.clone());
            let (directory, fetch) = (job.directory.clone(), job.fetch);
            thread::spawn(move || {
                for entry in work.iter() {
                    // The first failure stops the rest: there is no partial bundle worth having,
                    // and a CDN that served one wrong file is not worth 553 more requests.
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let outcome = one(&directory, entry, fetch).map(|()| entry.path);
                    let failed = outcome.is_err();
                    let _ = report.send(outcome);
                    if failed {
                        stop.store(true, Ordering::Relaxed);
                        return;
                    }
                }
            })
        })
        .collect();
    // Both ends are held here as well, and the loop below reads "every worker has gone" off the
    // channel disconnecting — which it cannot do while this thread still holds a sender.
    drop(work);
    drop(report);

    let mut done = 0u32;
    let mut spoke = Instant::now();
    let mut failure = None;
    let mut last = String::new();
    while done < total {
        match reports.recv_timeout(THROTTLE) {
            Ok(Ok(path)) => {
                done += 1;
                last = path.to_string();
                tracing::debug!("web bundle {}: {done}/{total} {path}", job.app);
            }
            Ok(Err(error)) => {
                failure = Some(error);
                break;
            }
            // Every worker has ended without finishing the queue: a panic, and nothing left to
            // wait for.
            Err(flume::RecvTimeoutError::Disconnected) => {
                failure = Some("the fetch stopped unexpectedly".to_string());
                break;
            }
            Err(flume::RecvTimeoutError::Timeout) => {}
        }
        if spoke.elapsed() >= THROTTLE {
            spoke = Instant::now();
            tell(
                job,
                Message::WebBundlePending {
                    app: job.app.clone(),
                    done,
                    total,
                    file: last.clone(),
                },
            );
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
    if let Some(error) = failure {
        return Err(error);
    }

    let map = job.directory.join(IMPORT_MAP_FILE);
    crate::atomic::write_atomic(&map, job.import_map.as_bytes())
        .map_err(|error| format!("{}: {error}", map.display()))?;

    // Last, and only now: every entry above is verified, so this is the one byte that turns a
    // directory of files into a bundle.
    let marker = job.directory.join(MARKER);
    crate::atomic::write_atomic(&marker, job.version.as_bytes())
        .map_err(|error| format!("{}: {error}", marker.display()))?;
    Ok(())
}

/// One file: skip it, or fetch it, verify it and write it.
///
/// Verification is before the write and not after, so a file whose hash does not match is never on
/// disk at all — discarded rather than cached.
fn one(directory: &Path, entry: &manifest::Entry, fetch: Fetch) -> Result<(), String> {
    let path = directory.join(entry.path);
    if held(&path, entry) {
        return Ok(());
    }
    let bytes = fetch(entry.url).map_err(|error| format!("{}: {error}", entry.path))?;
    if bytes.len() as u64 != entry.len {
        return Err(format!(
            "{}: the manifest says {} bytes and {} arrived",
            entry.path,
            entry.len,
            bytes.len()
        ));
    }
    if digest(&bytes) != entry.sha256 {
        return Err(format!(
            "{}: sha-256 does not match the manifest; the file was discarded",
            entry.path
        ));
    }
    crate::atomic::write_atomic(&path, &bytes).map_err(|error| format!("{}: {error}", entry.path))
}

/// Whether this file is already here and is the file the manifest names.
///
/// The length is checked from the directory entry first, so a resumed fetch reads the bytes of
/// only the files that could plausibly be right.
fn held(path: &Path, entry: &manifest::Entry) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() == entry.len => match std::fs::read(path) {
            Ok(bytes) => digest(&bytes) == entry.sha256,
            Err(_) => false,
        },
        _ => false,
    }
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Say something to every window still waiting on this bundle.
fn tell(job: &Job, message: Message) {
    say(&job.watchers, message);
}

/// The same, for the one caller that has the watcher list without a [`Job`] to reach it through.
fn say(watchers: &Arc<Mutex<Vec<Watcher>>>, message: Message) {
    let Ok(watchers) = watchers.lock() else {
        return;
    };
    for watcher in watchers.iter() {
        watcher.mailbox.send(message.clone());
    }
}

/// The default fetcher: one GET, capped.
///
/// A plain agent — no pin and no credential. These are public CDN URLs pinned by content hash, so
/// what is verified is the bytes rather than who served them.
fn download(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .build();
    let response = agent
        .get(url)
        .set("User-Agent", "ubiq")
        .call()
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(format!("more than {MAX_FILE} bytes"));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::Duration;

    use ubiq_proto::bus::{self, Client, Hub, To};

    use super::*;

    /// Two files, hashed for real, over a scheme no network answers.
    ///
    /// The manifest the host ships is 554 entries against a CDN; what the fetcher does with one is
    /// the same code either way, so the fixture is small and the seam is [`Fetch`] alone.
    const FILES: &[manifest::Entry] = &[
        manifest::Entry {
            path: "a.js",
            url: "fixture://a",
            sha256: "8ed3f6ad685b959ead7022518e1af76cd816f8e8ec7ccdda1ed4018e8f2223f8",
            len: 5,
        },
        manifest::Entry {
            path: "sub/b.js",
            url: "fixture://b",
            sha256: "f44e64e75f3948e9f73f8dfa94721c4ce8cbb4f265c4790c702b2d41cfbf2753",
            len: 4,
        },
    ];

    fn fixture() -> Bundle {
        Bundle {
            app: "fixture",
            version: "v1",
            files: FILES,
            import_map: "{\"imports\":{}}",
        }
    }

    /// A host end and one attached window, so `ensure` has a real mailbox to answer.
    fn window() -> (Hub, bus::HostEnd, Client) {
        let (hub, host) = bus::hub();
        let client = hub.connect();
        (hub, host, client)
    }

    fn assets(shared: &std::path::Path, fetch: Fetch) -> WebAssets {
        WebAssets::with(shared.to_path_buf(), vec![fixture()], fetch)
    }

    fn answer(client: &Client) -> Message {
        client
            .from_host()
            .recv_timeout(Duration::from_secs(5))
            .expect("the host answers")
    }

    /// The fetcher that must never be called.
    fn refused(_: &str) -> Result<Vec<u8>, String> {
        panic!("the network was touched");
    }

    fn served(url: &str) -> Result<Vec<u8>, String> {
        match url {
            "fixture://a" => Ok(b"alpha".to_vec()),
            "fixture://b" => Ok(b"beta".to_vec()),
            _ => Err("no such file".to_string()),
        }
    }

    /// A directory with the marker in it is answered where it stands.
    ///
    /// Versioned by directory means there is no staleness question to ask, so there is nothing
    /// here to re-check and nothing to fetch — the fetcher panics if it is reached.
    #[test]
    fn a_complete_bundle_is_answered_without_touching_the_network() {
        let shared = tempfile::TempDir::new().unwrap();
        let (_hub, host, client) = window();
        let mut assets = assets(shared.path(), refused);
        let directory = shared.path().join("web").join("fixture").join("v1");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(MARKER), "v1").unwrap();

        assets.ensure(
            client.id(),
            "fixture",
            host.mailbox(To::Client(client.id())),
        );

        match answer(&client) {
            Message::WebBundleReady { app, version, path } => {
                assert_eq!(app, "fixture");
                assert_eq!(version, "v1");
                assert_eq!(std::path::Path::new(&path), directory);
            }
            other => panic!("a complete bundle answered {other:?}"),
        }
    }

    /// A file whose hash is not the manifest's is discarded rather than cached, and takes the
    /// whole fetch with it.
    #[test]
    fn a_hash_that_does_not_match_fails_and_caches_nothing() {
        fn tampered(url: &str) -> Result<Vec<u8>, String> {
            match url {
                "fixture://a" => Ok(b"alpha".to_vec()),
                // The right length and the wrong bytes: exactly what a regenerated CDN bundle
                // looks like, and the case a length check alone would wave through.
                "fixture://b" => Ok(b"beta".to_vec().into_iter().rev().collect()),
                _ => Err("no such file".to_string()),
            }
        }

        let shared = tempfile::TempDir::new().unwrap();
        let (_hub, host, client) = window();
        let mut assets = assets(shared.path(), tampered);
        let directory = shared.path().join("web").join("fixture").join("v1");

        assets.ensure(
            client.id(),
            "fixture",
            host.mailbox(To::Client(client.id())),
        );

        let error = loop {
            match answer(&client) {
                Message::WebBundlePending { .. } => continue,
                Message::WebBundleFailed { error, .. } => break error,
                other => panic!("a tampered bundle answered {other:?}"),
            }
        };
        assert!(
            error.contains("sub/b.js"),
            "the failure names the file: {error}"
        );
        assert!(
            !directory.join("sub/b.js").exists(),
            "a file that failed verification is never written"
        );
        assert!(
            !complete(&directory),
            "and the directory never reads as finished"
        );
    }

    /// Progress is a real count from the first report, and every report after one has landed
    /// names the file — the two things a loader needs to say more than "please wait".
    #[test]
    fn progress_carries_the_count_up_front_and_then_the_file() {
        /// The fixture's bytes, with the second file slow enough that the throttle fires while
        /// the first has landed and the fetch is still running — the state a loader draws.
        fn slow(url: &str) -> Result<Vec<u8>, String> {
            if url.ends_with('b') {
                std::thread::sleep(THROTTLE * 3);
            }
            served(url)
        }

        let shared = tempfile::TempDir::new().unwrap();
        let (_hub, host, client) = window();
        let mut assets = assets(shared.path(), slow);

        assets.ensure(
            client.id(),
            "fixture",
            host.mailbox(To::Client(client.id())),
        );

        // The count is known before the network is touched, so it arrives before any file does.
        match answer(&client) {
            Message::WebBundlePending {
                done, total, file, ..
            } => {
                assert_eq!((done, total), (0, 2));
                assert!(file.is_empty(), "nothing has landed yet: {file:?}");
            }
            other => panic!("the first report was {other:?}"),
        }

        let mut named = None;
        loop {
            match answer(&client) {
                Message::WebBundlePending { done, file, .. } if done > 0 => named = Some(file),
                Message::WebBundlePending { .. } => continue,
                Message::WebBundleReady { .. } => break,
                other => panic!("a fetch in progress answered {other:?}"),
            }
        }
        let named = named.expect("a report after a file landed");
        assert!(
            FILES.iter().any(|entry| entry.path == named),
            "the report names a file of the manifest: {named:?}"
        );
    }

    /// Files on disk and no marker is a killed process, not a finished bundle.
    #[test]
    fn a_directory_without_the_marker_is_not_complete() {
        let shared = tempfile::TempDir::new().unwrap();
        let (_hub, host, client) = window();
        let directory = shared.path().join("web").join("fixture").join("v1");
        std::fs::create_dir_all(&directory).unwrap();
        // Real, verifiable bytes for the first entry, and nothing for the second: what a fetch
        // that was killed half way leaves.
        std::fs::write(directory.join("a.js"), b"alpha").unwrap();

        // With no network, an incomplete directory is a failure and never a Ready.
        let mut offline = assets(shared.path(), |_| Err("no network".to_string()));
        offline.ensure(
            client.id(),
            "fixture",
            host.mailbox(To::Client(client.id())),
        );
        loop {
            match answer(&client) {
                Message::WebBundlePending { .. } => continue,
                Message::WebBundleFailed { .. } => break,
                other => panic!("an unfinished bundle answered {other:?}"),
            }
        }

        // And finishing it is a resume: the file already there is kept, the missing one fetched,
        // and the marker written last.
        let mut online = assets(shared.path(), served);
        online.ensure(
            client.id(),
            "fixture",
            host.mailbox(To::Client(client.id())),
        );
        loop {
            match answer(&client) {
                Message::WebBundlePending { .. } => continue,
                Message::WebBundleReady { .. } => break,
                other => panic!("a finished bundle answered {other:?}"),
            }
        }
        assert!(complete(&directory));
        assert_eq!(
            std::fs::read(directory.join("sub/b.js")).unwrap(),
            b"beta".to_vec()
        );
        assert_eq!(
            std::fs::read_to_string(directory.join(IMPORT_MAP_FILE)).unwrap(),
            "{\"imports\":{}}"
        );
    }

    /// Two windows asking at once is one fetch, and both are told.
    #[test]
    fn a_second_ask_joins_the_fetch_in_flight() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        static RELEASE: AtomicBool = AtomicBool::new(false);

        /// Blocks until the test has had time to ask twice, so the second ask is unambiguously
        /// mid-fetch rather than after it.
        fn held(url: &str) -> Result<Vec<u8>, String> {
            CALLS.fetch_add(1, Ordering::Relaxed);
            let waited = Instant::now();
            while !RELEASE.load(Ordering::Relaxed) && waited.elapsed() < Duration::from_secs(5) {
                thread::sleep(Duration::from_millis(5));
            }
            served(url)
        }

        let shared = tempfile::TempDir::new().unwrap();
        let (hub, host, first) = window();
        let second = hub.connect();
        let mut assets = assets(shared.path(), held);

        assets.ensure(first.id(), "fixture", host.mailbox(To::Client(first.id())));
        assets.ensure(
            second.id(),
            "fixture",
            host.mailbox(To::Client(second.id())),
        );
        RELEASE.store(true, Ordering::Relaxed);

        for client in [&first, &second] {
            loop {
                match answer(client) {
                    Message::WebBundlePending { .. } => continue,
                    Message::WebBundleReady { .. } => break,
                    other => panic!("a joined fetch answered {other:?}"),
                }
            }
        }
        assert_eq!(
            CALLS.load(Ordering::Relaxed),
            FILES.len(),
            "the second ask joined the fetch rather than starting a second one"
        );
    }

    /// An app this host has no manifest for is a named failure, not a silence.
    #[test]
    fn an_unknown_app_is_refused() {
        let shared = tempfile::TempDir::new().unwrap();
        let (_hub, host, client) = window();
        let mut assets = assets(shared.path(), refused);
        assets.ensure(
            client.id(),
            "nothing",
            host.mailbox(To::Client(client.id())),
        );
        match answer(&client) {
            Message::WebBundleFailed { app, error } => {
                assert_eq!(app, "nothing");
                assert!(error.contains("nothing"));
            }
            other => panic!("an unknown app answered {other:?}"),
        }
    }
}
