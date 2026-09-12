//! The vendor bundles a web panel needs, fetched once and kept in the shared workarea.
//!
//! A web panel's component is 25 MiB of somebody else's JavaScript. Baking that into the
//! executable would inflate every build and every release for a feature most sessions never open,
//! so it is downloaded instead — and downloading is network plus disk, which is this half's.
//! **The interface asks; the host fetches, verifies, unpacks and answers when it is there**, and
//! the interface then serves those bytes off its own loopback origin.
//!
//! ```text
//! <shared workarea>/web/<app>/<version>.bundle       one file: every entry, deflated, then an
//!                                                     index and a footer — see `archive.rs`
//! <shared workarea>/web/<app>/<version>.bundle.part  where a fetch in progress writes
//! ```
//!
//! **Versioned by file, not invalidated by policy.** [`manifest::VERSION`] is pinned in source,
//! so a build that pins a new one looks for a file that does not exist, fetches, and leaves the
//! old one alone. There is no TTL and no staleness check: a bundle that is there is answered
//! immediately and nothing is fetched.
//!
//! **The rename is the only proof of done.** Every entry lands in a sibling `.part` file as it is
//! verified; the import map is appended the same way; only then is `.part` renamed over the real
//! name. A process killed mid-fetch leaves a `.part` file and no `.bundle`, which is exactly as
//! unfinished as it looks — there is no resume, the next ask fetches every entry again.
//!
//! **Every file is verified against the manifest before it is written.** A hash that does not
//! match fails the whole fetch and caches nothing: a downloader that trusts a CDN to have served
//! the right bytes is a supply-chain hole with a nice interface.
//!
//! **Failure is a downgrade.** No network and nothing cached answers
//! [`Message::WebBundleFailed`] with a sentence, and the feature that wanted the bundle says why
//! it is unavailable. Nothing else is disturbed.

pub mod archive;
pub mod manifest;
pub mod manifest_drawio;

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

/// Where the import map lands inside the archive, so the chrome can fetch it by a name it knows
/// without the host telling it one.
pub const IMPORT_MAP_FILE: &str = "importmap.json";

/// The suffix a bundle is written under while a fetch is in flight. Renamed away on success, so
/// its presence alone is never mistaken for a finished bundle.
const PART: &str = ".part";

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

/// Excalidraw, the first tenant.
pub const EXCALIDRAW: Bundle = Bundle {
    app: manifest::APP,
    version: manifest::VERSION,
    files: manifest::FILES,
    import_map: manifest::IMPORT_MAP,
};

/// draw.io, the second tenant. No import map: it loads through classic `<script>` tags.
pub const DRAWIO: Bundle = Bundle {
    app: manifest_drawio::APP,
    version: manifest_drawio::VERSION,
    files: manifest_drawio::FILES,
    import_map: manifest_drawio::IMPORT_MAP,
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
        Self::with(shared, vec![EXCALIDRAW, DRAWIO], download)
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

    /// Where a bundle's `.bundle` file lives once it is there.
    pub fn archive(&self, bundle: &Bundle) -> PathBuf {
        self.shared
            .join("web")
            .join(bundle.app)
            .join(format!("{}.bundle", bundle.version))
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
        let archive = self.archive(bundle);

        if archive.is_file() {
            asker.send(Message::WebBundleReady {
                app: app.to_string(),
                version: bundle.version.to_string(),
                path: archive.to_string_lossy().into_owned(),
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
            archive,
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

/// One fetch, addressed and equipped. Everything it needs, so the thread borrows nothing.
struct Job {
    app: String,
    version: String,
    files: &'static [manifest::Entry],
    import_map: &'static str,
    /// Where the finished `.bundle` lands. The fetch itself writes a sibling `.part` file and
    /// renames it here on success.
    archive: PathBuf,
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
/// which finds the finished archive and answers immediately — rather than joining a thread that
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
                job.archive.display()
            );
            tell(
                &job,
                Message::WebBundleReady {
                    app: job.app.clone(),
                    version: job.version.clone(),
                    path: job.archive.to_string_lossy().into_owned(),
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

/// Every file, a handful at a time, into the `.part` archive, then the import map, then the
/// rename that turns it into the real `.bundle`.
///
/// There is no resume: a `.part` left by a killed process is not read back, only overwritten by
/// the next fetch. The six download workers only fetch and verify — the single archive writer
/// below is the one thing allowed to touch the `.part` file, so 554 bodies never race a write.
fn fetch_all(job: &Job) -> Result<(), String> {
    let total = job.files.len() as u32;
    let parent = job.archive.parent().unwrap_or_else(|| Path::new("."));
    if let Err(error) = std::fs::create_dir_all(parent) {
        return Err(format!("{}: {error}", parent.display()));
    }
    let part = part_path(&job.archive);

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

    // Bounded at a handful of bodies: 554 file bodies held in memory at once is the thing this
    // archive exists to avoid, so a full channel simply makes a fast worker wait for the writer.
    let (report, reports) =
        flume::bounded::<Result<(&'static manifest::Entry, Vec<u8>), String>>(8);
    let stop = Arc::new(AtomicBool::new(false));

    let workers: Vec<_> = (0..PARALLEL.min(job.files.len().max(1)))
        .map(|_| {
            let (work, report, stop) = (work.clone(), report.clone(), stop.clone());
            let fetch = job.fetch;
            thread::spawn(move || {
                for entry in work.iter() {
                    // The first failure stops the rest: there is no partial bundle worth having,
                    // and a CDN that served one wrong file is not worth 553 more requests.
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let outcome = one(entry, fetch).map(|bytes| (entry, bytes));
                    let failed = outcome.is_err();
                    // A closed receiver means the main loop already bailed (its own write
                    // failed); either way there is nothing left to do.
                    if report.send(outcome).is_err() || failed {
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

    let file = std::fs::File::create(&part).map_err(|error| format!("{}: {error}", part.display()));
    let mut writer = match file {
        Ok(file) => archive::Writer::new(file),
        Err(error) => {
            stop.store(true, Ordering::Relaxed);
            for _ in reports.iter() {}
            for worker in workers {
                let _ = worker.join();
            }
            return Err(error);
        }
    };

    let mut done = 0u32;
    let mut spoke = Instant::now();
    let mut failure = None;
    let mut last = String::new();
    while done < total {
        match reports.recv_timeout(THROTTLE) {
            Ok(Ok((entry, bytes))) => {
                if let Err(error) = writer.write_entry(entry.path, &bytes) {
                    failure = Some(format!("{}: {error}", entry.path));
                    break;
                }
                done += 1;
                last = entry.path.to_string();
                tracing::debug!("web bundle {}: {done}/{total} {}", job.app, entry.path);
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
    // Set before the drain: a worker blocked sending on a full, no-longer-read channel would
    // otherwise deadlock the join below.
    if failure.is_some() {
        stop.store(true, Ordering::Relaxed);
    }
    for _ in reports.iter() {}
    for worker in workers {
        let _ = worker.join();
    }
    if let Some(error) = failure {
        let _ = std::fs::remove_file(&part);
        return Err(error);
    }

    if let Err(error) = writer.write_entry(IMPORT_MAP_FILE, job.import_map.as_bytes()) {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{IMPORT_MAP_FILE}: {error}"));
    }
    if let Err(error) = writer.finish() {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{}: {error}", part.display()));
    }

    // The rename is the one thing that turns a `.part` file into a finished bundle.
    std::fs::rename(&part, &job.archive)
        .map_err(|error| format!("{}: {error}", job.archive.display()))
}

/// The sibling a fetch in progress writes to.
fn part_path(archive: &Path) -> PathBuf {
    let mut name = archive.file_name().unwrap_or_default().to_os_string();
    name.push(PART);
    archive.with_file_name(name)
}

/// One file: fetch it and verify it. Verification is before the caller ever writes a byte, so a
/// file whose hash does not match is never in the archive at all — discarded rather than cached.
fn one(entry: &manifest::Entry, fetch: Fetch) -> Result<Vec<u8>, String> {
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
    Ok(bytes)
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

    /// A minimal reader for the test's own assertions — parses exactly the layout `archive.rs`
    /// writes and nothing more. There is no reader in the production code: the host never reads
    /// one of these back.
    fn read_archive(path: &std::path::Path) -> HashMap<String, Vec<u8>> {
        use std::io::Read as _;

        let bytes = std::fs::read(path).unwrap();
        let len = bytes.len();
        assert_eq!(&bytes[len - 8..], b"UBIQBND1", "the footer's magic");
        let offset = u64::from_le_bytes(bytes[len - 16..len - 8].try_into().unwrap()) as usize;
        let index: Vec<(String, u64, u64, u64)> =
            serde_json::from_slice(&bytes[offset..len - 16]).unwrap();

        let mut out = HashMap::new();
        for (path, start, compressed_len, _uncompressed_len) in index {
            let start = start as usize;
            let end = start + compressed_len as usize;
            let mut data = Vec::new();
            flate2::read::DeflateDecoder::new(&bytes[start..end])
                .read_to_end(&mut data)
                .unwrap();
            out.insert(path, data);
        }
        out
    }

    /// A `.bundle` file is answered where it stands.
    ///
    /// Versioned by file means there is no staleness question to ask, so there is nothing here to
    /// re-check and nothing to fetch — the fetcher panics if it is reached.
    #[test]
    fn a_complete_bundle_is_answered_without_touching_the_network() {
        let shared = tempfile::TempDir::new().unwrap();
        let (_hub, host, client) = window();
        let mut assets = assets(shared.path(), refused);
        let archive = shared.path().join("web").join("fixture").join("v1.bundle");
        std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
        // Its content is never read on this path — only that the file is there.
        std::fs::write(&archive, b"anything").unwrap();

        assets.ensure(
            client.id(),
            "fixture",
            host.mailbox(To::Client(client.id())),
        );

        match answer(&client) {
            Message::WebBundleReady { app, version, path } => {
                assert_eq!(app, "fixture");
                assert_eq!(version, "v1");
                assert_eq!(std::path::Path::new(&path), archive);
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
        let archive = shared.path().join("web").join("fixture").join("v1.bundle");

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
            !archive.is_file(),
            "a fetch that failed verification never becomes a finished bundle"
        );
        assert!(
            !part_path(&archive).exists(),
            "and its .part file is removed rather than left behind"
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

    /// A `.part` file and no rename is a killed process, not a finished bundle — and there is no
    /// resume: the next fetch overwrites it and fetches every entry again.
    #[test]
    fn a_leftover_part_file_is_not_complete_and_is_overwritten() {
        let shared = tempfile::TempDir::new().unwrap();
        let (_hub, host, client) = window();
        let archive = shared.path().join("web").join("fixture").join("v1.bundle");
        std::fs::create_dir_all(archive.parent().unwrap()).unwrap();
        // What a fetch killed half way leaves: a `.part` file, and no `.bundle`. Its content must
        // never be trusted, so it is garbage on purpose.
        std::fs::write(part_path(&archive), b"garbage from a killed fetch").unwrap();

        let mut assets = assets(shared.path(), served);
        assets.ensure(
            client.id(),
            "fixture",
            host.mailbox(To::Client(client.id())),
        );
        loop {
            match answer(&client) {
                Message::WebBundlePending { .. } => continue,
                Message::WebBundleReady { .. } => break,
                other => panic!("a fresh fetch over a stale .part answered {other:?}"),
            }
        }

        assert!(archive.is_file(), "the rename is the proof of done");
        assert!(
            !part_path(&archive).exists(),
            "no .part file is left behind"
        );
        let entries = read_archive(&archive);
        assert_eq!(entries.get("a.js"), Some(&b"alpha".to_vec()));
        assert_eq!(entries.get("sub/b.js"), Some(&b"beta".to_vec()));
        assert_eq!(
            entries.get(IMPORT_MAP_FILE),
            Some(&b"{\"imports\":{}}".to_vec())
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
