//! Ubiq updating itself: a signed feed, a verified download, and a detached helper that swaps.
//!
//! **The feed is compiled in, and signed.** [`FEED`] and [`PUBKEY`] are `option_env!` values, on
//! [`crate::feedback`]'s discipline: the environment at run time never decides where code comes
//! from. A build without a key, a build whose version is not semver (`dev-…`) and a platform the
//! release pipeline does not produce are all [`UpdateStatus::Disabled`], with a sentence.
//!
//! **One worker thread.** [`Updater`] owns a thread over a command queue: the check, the download
//! and the spawn of the helper all block, and none may run on the coordinator's thread. Every state
//! change is broadcast as `UpdateState` through an `Everyone` mailbox, so each window agrees.
//!
//! **Never downgrade, never apply unverified.** A release is offered only if its version is
//! strictly greater than the running one (`feed::pick`); a download is kept only if its size and
//! SHA-256 match the signed manifest, and deleted otherwise.
//!
//! **Install on quit by default.** `ApplyUpdate { relaunch: false }` starts the helper, which waits
//! for this process to exit on its own; `relaunch: true` is the same helper plus a restart, and the
//! interface quits the app.

pub mod apply;
pub mod feed;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use semver::Version;
use sha2::{Digest, Sha256};
use ubiq_proto::bus::Mailbox;
use ubiq_proto::messages::Message;
use ubiq_proto::update::{ApplyMode, ChannelRelease, UpdateChannel, UpdateSettings, UpdateStatus};

/// The version this build was released as; `dev-…` or absent for a source build.
pub const VERSION: Option<&str> = option_env!("UBIQ_VERSION");
/// The minisign public key (the base64 line) manifests must be signed by.
pub const PUBKEY: Option<&str> = option_env!("UBIQ_UPDATE_PUBKEY");
/// Where the signed manifests live.
pub const FEED: &str = match option_env!("UBIQ_UPDATE_FEED") {
    Some(feed) => feed,
    None => "https://github.com/mdnmdn/ubiq/releases/download/channels",
};
/// What the feed sees in a `User-Agent`.
pub const USER_AGENT: &str = concat!("ubiq-updater/", env!("CARGO_PKG_VERSION"));

/// First check after start, and the gap between scheduled checks.
const FIRST_CHECK: Duration = Duration::from_secs(30);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// Everything about the build and the machine the updater decides on. Injected so tests can
/// stand in for a release.
#[derive(Clone)]
pub struct Env {
    pub version: Option<String>,
    pub pubkey: String,
    pub feed: String,
    pub platform: Option<&'static str>,
    pub fetch: feed::Fetch,
    pub mode: ApplyMode,
}

impl Env {
    /// The running build's own.
    pub fn build() -> Self {
        Self {
            version: VERSION.map(|v| v.trim_start_matches('v').to_string()),
            pubkey: PUBKEY.unwrap_or_default().to_string(),
            feed: FEED.to_string(),
            platform: apply::platform_key(),
            fetch: feed::http,
            mode: std::env::current_exe().map_or(ApplyMode::Manual, |exe| apply::mode_of(&exe)),
        }
    }

    /// The running version, or the sentence for why updates are off.
    fn current(&self) -> Result<Version, String> {
        if self.platform.is_none() {
            return Err("updates are not offered on this platform".to_string());
        }
        let version = self.version.as_deref().unwrap_or("");
        let parsed = Version::parse(version)
            .map_err(|_| "this is a development build, which does not update itself".to_string())?;
        if self.pubkey.trim().is_empty() {
            return Err("this build has no update signing key".to_string());
        }
        Ok(parsed)
    }
}

enum Cmd {
    Check,
    Download,
    Apply(bool),
    Save(UpdateSettings),
    /// Read every channel's manifest; the answer goes to this mailbox alone.
    Releases(Mailbox),
}

struct State {
    status: UpdateStatus,
    settings: UpdateSettings,
    found: Option<feed::Release>,
    ready: Option<PathBuf>,
}

/// A std mutex that ignores poison: a panicked worker must not take the status line with it.
struct Shared<T>(std::sync::Mutex<T>);

impl<T> Shared<T> {
    fn new(value: T) -> Self {
        Self(std::sync::Mutex::new(value))
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The handle the coordinator holds.
pub struct Updater {
    commands: flume::Sender<Cmd>,
    state: Arc<Shared<State>>,
    version: String,
}

impl Updater {
    /// Start the worker over the running build.
    pub fn start(root: PathBuf, everyone: Mailbox) -> Self {
        Self::start_with(Env::build(), root, everyone)
    }

    pub fn start_with(env: Env, root: PathBuf, everyone: Mailbox) -> Self {
        let settings = load_settings(&root);
        let status = match env.current() {
            Ok(_) => UpdateStatus::Idle,
            Err(reason) => UpdateStatus::Disabled { reason },
        };
        let version = env.version.clone().unwrap_or_else(|| "dev".to_string());
        let state = Arc::new(Shared::new(State {
            status,
            settings,
            found: None,
            ready: None,
        }));
        let (commands, queue) = flume::unbounded();
        let worker = Worker {
            env,
            root,
            everyone,
            state: state.clone(),
            version: version.clone(),
        };
        std::thread::Builder::new()
            .name("ubiq-updates".to_string())
            .spawn(move || worker.run(queue))
            .expect("the updates thread");
        Self {
            commands,
            state,
            version,
        }
    }

    /// The `UpdateState` message for the asking client.
    pub fn snapshot(&self) -> Message {
        message(&self.state, &self.version)
    }

    /// Answer `Releases` to `asker`: what each channel offers, whatever is running. A disabled
    /// build says why at once, for every channel; otherwise the worker reads the feed, so the
    /// caller's thread never waits on the network.
    pub fn releases(&self, asker: Mailbox) {
        let disabled = match &self.state.lock().status {
            UpdateStatus::Disabled { reason } => Some(reason.clone()),
            _ => None,
        };
        match disabled {
            Some(reason) => {
                let channels = UpdateChannel::ALL
                    .iter()
                    .map(|&channel| ChannelRelease {
                        channel,
                        release: None,
                        error: Some(reason.clone()),
                    })
                    .collect();
                asker.send(Message::Releases { channels });
            }
            None => {
                let _ = self.commands.send(Cmd::Releases(asker));
            }
        }
    }

    pub fn check(&self) {
        let _ = self.commands.send(Cmd::Check);
    }
    pub fn download(&self) {
        let _ = self.commands.send(Cmd::Download);
    }
    pub fn apply(&self, relaunch: bool) {
        let _ = self.commands.send(Cmd::Apply(relaunch));
    }
    pub fn save(&self, settings: UpdateSettings) {
        let _ = self.commands.send(Cmd::Save(settings));
    }
}

struct Worker {
    env: Env,
    root: PathBuf,
    everyone: Mailbox,
    state: Arc<Shared<State>>,
    version: String,
}

impl Worker {
    fn run(self, queue: flume::Receiver<Cmd>) {
        let mut next = self
            .state
            .lock()
            .settings
            .auto_check
            .then(|| Instant::now() + FIRST_CHECK);
        loop {
            let wait = next.map_or(Duration::from_secs(3600), |at| {
                at.saturating_duration_since(Instant::now())
            });
            match queue.recv_timeout(wait) {
                Ok(Cmd::Check) => self.check(),
                Ok(Cmd::Download) => self.download(),
                Ok(Cmd::Apply(relaunch)) => self.apply(relaunch),
                Ok(Cmd::Releases(asker)) => self.releases(&asker),
                Ok(Cmd::Save(settings)) => {
                    let old = std::mem::replace(&mut self.state.lock().settings, settings);
                    save_settings(&self.root, &settings);
                    if old.channel != settings.channel {
                        // What was found belongs to the old channel's view of the feed.
                        let mut state = self.state.lock();
                        state.found = None;
                        if matches!(state.status, UpdateStatus::Available { .. }) {
                            state.status = UpdateStatus::Idle;
                        }
                    }
                    next = match (settings.auto_check, next) {
                        (true, None) => Some(Instant::now() + FIRST_CHECK),
                        (false, _) => None,
                        (true, kept) => kept,
                    };
                    self.broadcast();
                    if old.channel != settings.channel && settings.auto_check {
                        self.check();
                    }
                }
                Err(flume::RecvTimeoutError::Timeout) => {
                    if next.is_some() {
                        self.check();
                        next = Some(Instant::now() + EVERY);
                    }
                }
                Err(flume::RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn broadcast(&self) {
        self.everyone.send(message(&self.state, &self.version));
    }

    fn set(&self, status: UpdateStatus) {
        self.state.lock().status = status;
        self.broadcast();
    }

    fn fail(&self, message: String) {
        tracing::warn!("update: {message}");
        self.set(UpdateStatus::Failed { message });
    }

    /// `true` when the updater is switched off, after saying so again.
    fn disabled(&self) -> bool {
        if matches!(self.state.lock().status, UpdateStatus::Disabled { .. }) {
            self.broadcast();
            return true;
        }
        false
    }

    fn check(&self) {
        if self.disabled() {
            return;
        }
        let Ok(current) = self.env.current() else {
            return;
        };
        let Some(platform) = self.env.platform else {
            return;
        };
        self.set(UpdateStatus::Checking);
        let channel = self.state.lock().settings.channel;
        match feed::newest(
            self.env.fetch,
            &self.env.feed,
            &self.env.pubkey,
            channel,
            &current,
            platform,
        ) {
            Err(message) => self.fail(message),
            Ok(None) => {
                let mut state = self.state.lock();
                state.found = None;
                drop(state);
                self.set(UpdateStatus::UpToDate {
                    checked: chrono::Utc::now().to_rfc3339(),
                });
            }
            Ok(Some(release)) => {
                let info = release.info.clone();
                {
                    let mut state = self.state.lock();
                    // A fresh find of what is already downloaded keeps the download.
                    if state.found.as_ref().map(|r| &r.version) != Some(&release.version) {
                        state.ready = None;
                    }
                    state.found = Some(release);
                }
                if let Some(path) = self.state.lock().ready.clone()
                    && path.is_file()
                {
                    self.set(UpdateStatus::Ready { info });
                    return;
                }
                self.set(UpdateStatus::Available {
                    info,
                    apply: self.env.mode,
                });
                if self.state.lock().settings.auto_download && self.env.mode != ApplyMode::Manual {
                    self.download();
                }
            }
        }
    }

    /// Every channel's newest release for this platform, told to `asker` and nobody else. Touches
    /// no status: this is a look at the feed, not a check.
    fn releases(&self, asker: &Mailbox) {
        // Always answered, so the asker never waits on a question nobody will take up.
        let Some(platform) = self.env.platform else {
            let channels = UpdateChannel::ALL
                .iter()
                .map(|&channel| ChannelRelease {
                    channel,
                    release: None,
                    error: Some("No releases are published for this platform.".to_string()),
                })
                .collect();
            asker.send(Message::Releases { channels });
            return;
        };
        let channels = feed::channels(self.env.fetch, &self.env.feed, &self.env.pubkey, platform);
        asker.send(Message::Releases { channels });
    }

    fn download(&self) {
        if self.disabled() {
            return;
        }
        let Some(release) = self.state.lock().found.clone() else {
            return self
                .fail("there is no update to download; check for updates first".to_string());
        };
        let dir = self.root.join("updates").join(&release.info.version);
        let info = release.info.clone();
        let total = release.asset.size;
        let progress = |received: u64| {
            self.set(UpdateStatus::Downloading {
                info: info.clone(),
                received,
                total,
            });
        };
        progress(0);
        match fetch_asset(&release.asset, &dir, progress) {
            Ok(path) => {
                self.state.lock().ready = Some(path);
                self.set(UpdateStatus::Ready { info });
            }
            Err(message) => self.fail(message),
        }
    }

    fn apply(&self, relaunch: bool) {
        if self.disabled() {
            return;
        }
        let (info, path) = {
            let state = self.state.lock();
            match (&state.found, &state.ready) {
                (Some(release), Some(path)) => (release.info.clone(), path.clone()),
                _ => {
                    drop(state);
                    return self.fail("the update has not been downloaded yet".to_string());
                }
            }
        };
        if self.env.mode == ApplyMode::Manual {
            return self.fail(
                "this copy of Ubiq cannot update itself; download the new version and install it yourself"
                    .to_string(),
            );
        }
        let stage = path.parent().unwrap_or(&self.root).join("stage");
        match apply::spawn(self.env.mode, &path, &stage, relaunch) {
            Ok(()) => self.set(UpdateStatus::Applying { info }),
            Err(message) => self.fail(message),
        }
    }
}

fn message(state: &Shared<State>, version: &str) -> Message {
    let state = state.lock();
    Message::UpdateState {
        status: state.status.clone(),
        settings: state.settings,
        current_version: version.to_string(),
    }
}

/// Stream `asset` into `dir`, verifying its size and SHA-256; a mismatch deletes the file.
fn fetch_asset(asset: &feed::Asset, dir: &Path, progress: impl Fn(u64)) -> Result<PathBuf, String> {
    feed::require_https(&asset.url)?;
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("could not prepare the download: {error}"))?;
    let name: String = asset
        .url
        .rsplit('/')
        .next()
        .unwrap_or("update")
        .split(['?', '#'])
        .next()
        .unwrap_or("update")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    let name = if name.is_empty() || name.starts_with('.') {
        "update".to_string()
    } else {
        name
    };
    let target = dir.join(&name);
    let part = dir.join(format!("{name}.part"));
    let response = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(30))
        .timeout_read(Duration::from_secs(60))
        .build()
        .get(&asset.url)
        .set("User-Agent", USER_AGENT)
        .call()
        .map_err(|error| format!("the update could not be downloaded: {error}"))?;
    let outcome = stream(response.into_reader(), &part, asset, progress);
    match outcome {
        Ok(()) => {
            std::fs::rename(&part, &target)
                .map_err(|error| format!("could not keep the downloaded update: {error}"))?;
            Ok(target)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&part);
            Err(error)
        }
    }
}

fn stream(
    mut from: impl Read,
    part: &Path,
    asset: &feed::Asset,
    progress: impl Fn(u64),
) -> Result<(), String> {
    let mut file = std::fs::File::create(part)
        .map_err(|error| format!("could not write the download: {error}"))?;
    let mut hash = Sha256::new();
    let mut received = 0u64;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut last = Instant::now();
    loop {
        let read = from
            .read(&mut buffer)
            .map_err(|error| format!("the download was interrupted: {error}"))?;
        if read == 0 {
            break;
        }
        received += read as u64;
        if received > asset.size {
            return Err("the download is larger than the feed said; it was discarded".to_string());
        }
        hash.update(&buffer[..read]);
        file.write_all(&buffer[..read])
            .map_err(|error| format!("could not write the download: {error}"))?;
        if last.elapsed() > Duration::from_millis(250) {
            last = Instant::now();
            progress(received);
        }
    }
    file.flush()
        .map_err(|error| format!("could not write the download: {error}"))?;
    verify_digest(received, &hash.finalize(), asset)
}

/// Size and digest against the signed manifest's claim.
fn verify_digest(received: u64, digest: &[u8], asset: &feed::Asset) -> Result<(), String> {
    if received != asset.size {
        return Err("the download is incomplete; it was discarded".to_string());
    }
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    if !hex.eq_ignore_ascii_case(asset.sha256.trim()) {
        return Err("the download does not match its checksum; it was discarded".to_string());
    }
    Ok(())
}

const SETTINGS_FILE: &str = "updates.toml";

fn load_settings(root: &Path) -> UpdateSettings {
    std::fs::read_to_string(root.join(SETTINGS_FILE))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_settings(root: &Path, settings: &UpdateSettings) {
    let Ok(text) = toml::to_string(settings) else {
        return;
    };
    if let Err(error) = crate::atomic::write_atomic(&root.join(SETTINGS_FILE), text.as_bytes()) {
        tracing::warn!("update settings could not be saved: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(version: Option<&str>, key: &str, platform: Option<&'static str>) -> Env {
        fn none(_: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(None)
        }
        Env {
            version: version.map(String::from),
            pubkey: key.to_string(),
            feed: "https://f".to_string(),
            platform,
            fetch: none,
            mode: ApplyMode::Manual,
        }
    }

    #[test]
    fn a_build_is_disabled_with_a_reason() {
        let mac = Some("macos-aarch64");
        assert!(env(Some("1.0.0"), "k", mac).current().is_ok());
        assert!(
            env(Some("dev-abc"), "k", mac)
                .current()
                .unwrap_err()
                .contains("development")
        );
        assert!(env(None, "k", mac).current().is_err());
        assert!(
            env(Some("1.0.0"), " ", mac)
                .current()
                .unwrap_err()
                .contains("key")
        );
        assert!(
            env(Some("1.0.0"), "k", None)
                .current()
                .unwrap_err()
                .contains("platform")
        );
    }

    #[test]
    fn a_download_is_kept_only_if_size_and_digest_match() {
        let body = b"hello update";
        let hex: String = Sha256::digest(body)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let asset = feed::Asset {
            url: "https://e/a.zip".into(),
            sha256: hex.clone(),
            size: body.len() as u64,
        };
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        assert!(stream(&body[..], &part, &asset, |_| {}).is_ok());
        let wrong = feed::Asset {
            sha256: "00".into(),
            ..asset.clone()
        };
        assert!(
            stream(&body[..], &part, &wrong, |_| {})
                .unwrap_err()
                .contains("checksum")
        );
        let short = feed::Asset {
            size: 3,
            ..asset.clone()
        };
        assert!(
            stream(&body[..], &part, &short, |_| {})
                .unwrap_err()
                .contains("larger")
        );
        let long = feed::Asset { size: 99, ..asset };
        assert!(
            stream(&body[..], &part, &long, |_| {})
                .unwrap_err()
                .contains("incomplete")
        );
    }

    #[test]
    fn settings_round_trip_and_default_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_settings(dir.path()), UpdateSettings::default());
        let settings = UpdateSettings {
            channel: UpdateChannel::Nightly,
            auto_check: false,
            auto_download: true,
        };
        save_settings(dir.path(), &settings);
        assert_eq!(load_settings(dir.path()), settings);
    }

    #[test]
    fn a_disabled_updater_answers_with_its_reason() {
        let (_hub, host) = ubiq_proto::bus::hub();
        let updater = Updater::start_with(
            env(Some("dev-1"), "k", Some("macos-aarch64")),
            tempfile::tempdir().unwrap().keep(),
            host.mailbox(ubiq_proto::bus::To::Everyone),
        );
        match updater.snapshot() {
            Message::UpdateState {
                status: UpdateStatus::Disabled { reason },
                current_version,
                ..
            } => {
                assert!(reason.contains("development"));
                assert_eq!(current_version, "dev-1");
            }
            other => panic!("{other:?}"),
        }
    }

    /// Start an updater over `env` and ask it for the releases as one client.
    fn ask_releases(env: Env) -> Vec<ChannelRelease> {
        let (hub, host) = ubiq_proto::bus::hub();
        let asker = hub.connect();
        let updater = Updater::start_with(
            env,
            tempfile::tempdir().unwrap().keep(),
            host.mailbox(ubiq_proto::bus::To::Everyone),
        );
        updater.releases(host.mailbox(ubiq_proto::bus::To::Client(asker.id())));
        match asker
            .from_host()
            .recv_timeout(Duration::from_secs(5))
            .expect("an answer")
        {
            Message::Releases { channels } => channels,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_disabled_build_lists_every_channel_with_its_reason() {
        let channels = ask_releases(env(Some("dev-1"), "k", Some("macos-aarch64")));
        let order: Vec<_> = channels.iter().map(|c| c.channel).collect();
        assert_eq!(order, UpdateChannel::ALL);
        for channel in channels {
            assert!(channel.release.is_none());
            assert!(channel.error.unwrap().contains("development"));
        }
    }

    #[test]
    fn releases_are_read_on_the_worker_and_answered_to_the_asker() {
        // The fake feed has nothing anywhere: every channel is empty, none is an error.
        let channels = ask_releases(env(Some("1.0.0"), "k", Some("macos-aarch64")));
        assert_eq!(channels.len(), UpdateChannel::ALL.len());
        assert!(
            channels
                .iter()
                .all(|c| c.release.is_none() && c.error.is_none())
        );
    }
}
