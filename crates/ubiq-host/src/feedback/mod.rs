//! Sending the user's report outward: what a destination is, and which one this host resolves.
//!
//! **A destination is compiled in as a default, and overridable.** Which service a build reports
//! to is first a property of the build — `option_env!`, on [`crate::connectors::providers`]'s
//! discipline: reading the *environment* at run time would let a user's shell decide where their
//! words and their screenshot are posted. On top of that default the user may set an override from
//! the settings page — the intake URL in the host settings file ([`HostSettings::feedback_url`]),
//! the key in the OS keychain ([`keychain`]) — and an override, when set, wins per field. Clearing
//! one returns that field to the compiled default. Nothing resolved is ever sent to the interface
//! as a key: [`FeedbackSettings`] carries whether one is set, where from, and a prefix.
//!
//! [`HostSettings::feedback_url`]: ubiq_proto::settings::HostSettings::feedback_url
//!
//! **A destination is a trait, not a match.** [`Destination`] is the whole surface: one call that
//! takes a report and answers with a receipt. [`api::Api`] is the first implementation — one POST
//! to the resolved URL with the resolved key — and [`issues::Issues`] the second, which opens an
//! issue on a repository host (compiled in only). Adding a third is a file here and a row in
//! [`select`], nothing else.
//!
//! **Everything blocking runs on one worker thread, never the coordinator's.** [`Feedback`] is one
//! thread over a queue, on [`crate::quota`]'s pattern: the keychain read at start, a report's POST,
//! a settings change and a connection test are all commands on it, first in, first answered. That
//! order is also what makes the offer honest: a `QueryFeedback` is answered after the start-up
//! resolve, and after any change queued before it.
//!
//! Nothing here logs the report's body, and nothing logs a key.

#[cfg(feature = "listener")]
pub mod api;
#[cfg(feature = "listener")]
pub mod issues;
#[cfg(feature = "listener")]
pub mod keychain;

use std::sync::Arc;
use std::thread;

use ubiq_proto::bus::Mailbox;
use ubiq_proto::feedback::{
    FeedbackChannel, FeedbackError, FeedbackOffer, FeedbackReceipt, FeedbackReport,
    FeedbackSettings, FeedbackSettingsChange, FeedbackSource, FeedbackTestResult,
};
use ubiq_proto::messages::Message;

use crate::settings::Settings;

/// The compiled defaults: where reports go, and what buys the right to post one. A release build
/// takes them from CI (`vars.UBIQ_FEEDBACK_URL`, `secrets.UBIQ_FEEDBACK_API_KEY`); an unset CI
/// value arrives as an empty string, which [`compiled`] reads as no default.
pub const API_URL: Option<&str> = option_env!("UBIQ_FEEDBACK_URL");
pub const API_KEY: Option<&str> = option_env!("UBIQ_FEEDBACK_API_KEY");

/// The repository issues are opened on, `owner/name`, and the token that may open one.
/// Compiled in only; no override.
pub const ISSUES_REPO: Option<&str> = option_env!("UBIQ_FEEDBACK_GITHUB_REPO");
pub const ISSUES_TOKEN: Option<&str> = option_env!("UBIQ_FEEDBACK_GITHUB_TOKEN");

/// What both halves put in the `User-Agent` of a feedback call.
pub const USER_AGENT: &str = concat!("ubiq/", env!("CARGO_PKG_VERSION"));

/// How much of a key the interface is shown: enough to tell two apart (`ubiq_ik_` plus a few),
/// never enough to post with.
pub const KEY_PREFIX_CHARS: usize = 12;

/// Somewhere a report can land.
///
/// One method and no state machine: a report is composed by the interface, sent once, and either
/// lands or does not. An implementation blocks — it is called on [`Feedback`]'s thread, never on
/// the coordinator's.
pub trait Destination: Send + Sync {
    /// Which destination this is, for the sentence the modal puts under its send button.
    fn channel(&self) -> FeedbackChannel;

    /// Post one report. `Ok` carries whatever the service called it; `Err`'s chain becomes the
    /// sentence the user reads, so it names what was being done and never carries a key.
    fn send(&self, report: &FeedbackReport) -> anyhow::Result<FeedbackReceipt>;
}

/// A compiled value, or `None` when it was not compiled in or was compiled in empty.
fn compiled(value: Option<&'static str>) -> Option<&'static str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// One field of the destination as it resolves: the value, and where it came from.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Field {
    pub value: Option<String>,
    pub source: FeedbackSource,
}

/// A `Field` may hold a key, so its `Debug` never prints the value.
impl std::fmt::Debug for Field {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Field")
            .field("value", &self.value.as_ref().map(|_| "<redacted>"))
            .field("source", &self.source)
            .finish()
    }
}

/// Override first, compiled default second, nothing third.
fn field(overridden: Option<&str>, default: Option<&'static str>) -> Field {
    let overridden = overridden.map(str::trim).filter(|value| !value.is_empty());
    match (overridden, compiled(default)) {
        (Some(value), _) => Field {
            value: Some(value.to_string()),
            source: FeedbackSource::Override,
        },
        (None, Some(value)) => Field {
            value: Some(value.to_string()),
            source: FeedbackSource::Compiled,
        },
        (None, None) => Field::default(),
    }
}

/// The API destination's two fields, as they resolve against the overrides given.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    pub url: Field,
    pub key: Field,
}

/// Redacted through [`Field`]'s `Debug`: the key is never printed.
impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolved")
            .field("url", &self.url)
            .field("key", &self.key)
            .finish()
    }
}

impl Resolved {
    pub fn new(url_override: Option<&str>, key_override: Option<&str>) -> Self {
        Self {
            url: field(url_override, API_URL),
            key: field(key_override, API_KEY),
        }
    }

    /// What the interface may be told. Never the key.
    pub fn settings(&self, error: Option<String>) -> FeedbackSettings {
        FeedbackSettings {
            url: self.url.value.clone(),
            url_source: self.url.source,
            key_set: self.key.value.is_some(),
            key_source: self.key.source,
            key_prefix: self.key.value.as_deref().map(prefix),
            error,
        }
    }
}

/// A key's first [`KEY_PREFIX_CHARS`] characters, marked as cut when it was.
pub fn prefix(key: &str) -> String {
    let mut shown: String = key.chars().take(KEY_PREFIX_CHARS).collect();
    if key.chars().count() > KEY_PREFIX_CHARS {
        shown.push('\u{2026}');
    }
    shown
}

/// The service's test endpoint, from the intake URL: `…/api/feedback` becomes
/// `…/api/feedback/test`, and a bare origin gets the whole path.
pub fn test_url(intake: &str) -> String {
    let intake = intake.trim().trim_end_matches('/');
    if intake.ends_with("/api/feedback") {
        format!("{intake}/test")
    } else {
        format!("{intake}/api/feedback/test")
    }
}

/// The destination the resolved fields make, if they make one.
///
/// The API wins where both are available: it is the one that takes the screenshot. With neither
/// this returns `None`, which is the state a source build is in and not a failure.
pub fn select(resolved: &Resolved) -> Option<Box<dyn Destination>> {
    #[cfg(feature = "listener")]
    {
        if let (Some(url), Some(key)) = (&resolved.url.value, &resolved.key.value) {
            return Some(Box::new(api::Api::new(url.clone(), key.clone())));
        }
        if let (Some(repo), Some(token)) = (compiled(ISSUES_REPO), compiled(ISSUES_TOKEN)) {
            return Some(Box::new(issues::Issues::new(repo, token)));
        }
    }
    #[cfg(not(feature = "listener"))]
    let _ = resolved;
    None
}

/// What the interface is told before it draws the control.
pub fn offer(destination: Option<&dyn Destination>) -> FeedbackOffer {
    match destination {
        Some(destination) => FeedbackOffer {
            enabled: true,
            channel: destination.channel(),
        },
        None => FeedbackOffer {
            enabled: false,
            channel: FeedbackChannel::None,
        },
    }
}

/// One report on its way out, and the window waiting to hear whether it landed.
pub struct Job {
    pub report: Box<FeedbackReport>,
    /// The window that sent it. It hears back whether the post worked or not.
    pub reply_to: Mailbox,
}

/// What the worker is asked to do, in the order it was asked.
enum Command {
    Send(Job),
    /// Answer with the offer as it resolves now.
    Offer(Mailbox),
    /// Answer with the settings as they resolve now.
    Query(Mailbox),
    /// Apply a change, then tell every window the new settings and the new offer.
    Set(Box<FeedbackSettingsChange>),
    /// Check the effective URL and key against the test endpoint.
    Test(Mailbox),
}

/// The thread that owns the destination.
///
/// One thread over an unbounded queue, on [`crate::quota::Quota`]'s pattern and for its reason:
/// the post, the test and the keychain are all blocking, and anything blocking on the
/// coordinator's thread stalls every pane's keystrokes. No ticker and no retry — a report that
/// failed is said to have failed, and the user still has their words in the modal.
pub struct Feedback {
    commands: flume::Sender<Command>,
}

impl Feedback {
    /// Start the worker. It resolves the destination first — the URL override from `settings`,
    /// the key override from the keychain — and broadcasts on `everyone` after every change. It
    /// ends when the coordinator that holds this drops it.
    pub fn start(settings: Arc<Settings>, everyone: Mailbox) -> Self {
        let (commands, queue) = flume::unbounded::<Command>();
        thread::Builder::new()
            .name("ubiq-feedback".to_string())
            .spawn(move || Worker::new(settings, everyone).run(queue))
            .expect("the feedback thread");
        Self { commands }
    }

    /// Answer `reply_to` with whether a report can be sent, and where to.
    pub fn offer(&self, reply_to: Mailbox) {
        self.command(Command::Offer(reply_to));
    }

    /// Answer `reply_to` with the destination's settings, key withheld.
    pub fn query(&self, reply_to: Mailbox) {
        self.command(Command::Query(reply_to));
    }

    /// Change the overrides. Every window hears the outcome.
    pub fn set(&self, change: FeedbackSettingsChange) {
        self.command(Command::Set(Box::new(change)));
    }

    /// Test the connection and answer `reply_to`.
    pub fn test(&self, reply_to: Mailbox) {
        self.command(Command::Test(reply_to));
    }

    /// Queue one report. Never blocks — the queue is unbounded, on the bus's own rule.
    pub fn submit(&self, job: Job) {
        self.command(Command::Send(job));
    }

    fn command(&self, command: Command) {
        if self.commands.send(command).is_err() {
            tracing::error!("the feedback thread has gone; a request was dropped");
        }
    }
}

/// The worker's state: the overrides as stored, and what they resolve to.
struct Worker {
    settings: Arc<Settings>,
    everyone: Mailbox,
    url_override: Option<String>,
    key_override: Option<String>,
    /// Why the keychain could not be read, while that is unknown. The key override is then
    /// neither set nor absent: it is retried on the next command and said in the settings.
    key_unknown: Option<String>,
    resolved: Resolved,
    destination: Option<Box<dyn Destination>>,
}

impl Worker {
    fn new(settings: Arc<Settings>, everyone: Mailbox) -> Self {
        let url_override = settings.host().feedback_url;
        let mut worker = Self {
            settings,
            everyone,
            url_override,
            key_override: None,
            key_unknown: None,
            resolved: Resolved::default(),
            destination: None,
        };
        worker.load_key_override();
        worker.resolve();
        worker
    }

    fn load_key_override(&mut self) {
        match load_key() {
            Ok(key) => {
                self.key_override = key;
                self.key_unknown = None;
            }
            Err(error) => self.key_unknown = Some(error),
        }
    }

    /// Try the keychain again while its state is unknown, and re-resolve if that settled it.
    fn retry_unknown(&mut self) {
        if self.key_unknown.is_some() {
            self.load_key_override();
            self.resolve();
        }
    }

    fn resolve(&mut self) {
        self.resolved = Resolved::new(self.url_override.as_deref(), self.key_override.as_deref());
        self.destination = select(&self.resolved);
    }

    fn run(mut self, queue: flume::Receiver<Command>) {
        while let Ok(command) = queue.recv() {
            if !matches!(command, Command::Test(_)) {
                self.retry_unknown();
            }
            match command {
                Command::Send(job) => {
                    job.reply_to.send(answer(self.destination.as_deref(), &job));
                }
                Command::Offer(reply_to) => {
                    reply_to.send(Message::FeedbackOffered {
                        offer: offer(self.destination.as_deref()),
                    });
                }
                Command::Query(reply_to) => {
                    reply_to.send(Message::FeedbackSettingsChanged {
                        settings: self.resolved.settings(self.key_unknown.clone()),
                    });
                }
                Command::Set(change) => {
                    let applied = self.apply(*change);
                    self.resolve();
                    let error = match (applied, self.key_unknown.clone()) {
                        (Some(applied), Some(unknown)) => Some(format!("{applied}; {unknown}")),
                        (applied, unknown) => applied.or(unknown),
                    };
                    self.everyone.send(Message::FeedbackSettingsChanged {
                        settings: self.resolved.settings(error),
                    });
                    self.everyone.send(Message::FeedbackOffered {
                        offer: offer(self.destination.as_deref()),
                    });
                }
                Command::Test(reply_to) => {
                    reply_to.send(Message::FeedbackTested {
                        result: test(&self.resolved),
                    });
                }
            }
        }
    }

    /// Write the change down: the URL to the host settings file, the key to the keychain. A field
    /// whose write failed keeps its old value, and the failure is the sentence returned.
    fn apply(&mut self, change: FeedbackSettingsChange) -> Option<String> {
        let mut errors = Vec::new();

        let url = if change.clear_url {
            Some(None)
        } else {
            change
                .url
                .map(|url| Some(url.trim().to_string()).filter(|url| !url.is_empty()))
        };
        if let Some(url) = url {
            if let Some(Err(error)) = url.as_deref().map(ubiq_proto::feedback::validate_url) {
                errors.push(error);
            } else {
                match self
                    .settings
                    .update_host(|settings| settings.feedback_url = url.clone())
                {
                    Ok(_) => self.url_override = url,
                    Err(error) => errors.push(format!("the URL was not saved: {error}")),
                }
            }
        }

        let key = if change.clear_key {
            Some(None)
        } else {
            change
                .key
                .map(|key| Some(key.expose().trim().to_string()).filter(|key| !key.is_empty()))
        };
        if let Some(key) = key {
            let written = match &key {
                Some(key) => save_key(key),
                None => delete_key(),
            };
            match written {
                Ok(()) => {
                    self.key_override = key;
                    // The write settled what the keychain holds, whatever the read could not say.
                    self.key_unknown = None;
                }
                Err(error) => errors.push(error),
            }
        }

        (!errors.is_empty()).then(|| errors.join("; "))
    }
}

#[cfg(feature = "listener")]
fn load_key() -> Result<Option<String>, String> {
    keychain::load()
}

#[cfg(feature = "listener")]
fn save_key(key: &str) -> Result<(), String> {
    keychain::save(key)
}

#[cfg(feature = "listener")]
fn delete_key() -> Result<(), String> {
    keychain::delete()
}

#[cfg(not(feature = "listener"))]
fn load_key() -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(not(feature = "listener"))]
fn save_key(_key: &str) -> Result<(), String> {
    Err("this build keeps no feedback key".to_string())
}

#[cfg(not(feature = "listener"))]
fn delete_key() -> Result<(), String> {
    Ok(())
}

/// Ask the service whether the effective URL and key would be accepted.
fn test(resolved: &Resolved) -> FeedbackTestResult {
    let failed = |error: &str| FeedbackTestResult {
        ok: false,
        key_name: None,
        error: Some(error.to_string()),
    };
    let Some(url) = resolved.url.value.as_deref() else {
        return failed("no feedback URL is configured");
    };
    let Some(key) = resolved.key.value.as_deref() else {
        return failed("no feedback key is configured");
    };
    #[cfg(feature = "listener")]
    {
        api::test(&test_url(url), key)
    }
    #[cfg(not(feature = "listener"))]
    {
        let _ = (url, key);
        failed("this build has no network client")
    }
}

/// Post once, and say what became of it.
fn answer(destination: Option<&dyn Destination>, job: &Job) -> Message {
    let report_id = job.report.report_id;
    let Some(destination) = destination else {
        return Message::FeedbackFailed {
            failure: FeedbackError {
                report_id,
                error: "no feedback destination is configured".to_string(),
            },
        };
    };
    match destination.send(&job.report) {
        Ok(receipt) => Message::FeedbackSent { receipt },
        Err(error) => {
            // The sentence the user reads. `anyhow`'s chain names what was being done and what
            // failed; nothing key-shaped ever reaches it.
            let error = format!("{error:#}");
            tracing::warn!("feedback could not be sent: {error}");
            Message::FeedbackFailed {
                failure: FeedbackError { report_id, error },
            }
        }
    }
}

/// Standard base64, for a screenshot that has to travel inside a JSON body.
///
/// Written here rather than taken as a dependency: it is fifteen lines, it is called once per
/// report, and the crate that would provide it draws nothing else in.
#[cfg_attr(not(feature = "listener"), expect(dead_code))]
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = chunk.iter().enumerate().fold(0u32, |bits, (at, byte)| {
            bits | u32::from(*byte) << (16 - 8 * at)
        });
        for at in 0..=chunk.len() {
            out.push(ALPHABET[(bits >> (18 - 6 * at)) as usize & 0x3f] as char);
        }
        for _ in chunk.len()..3 {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn a_build_with_no_destination_offers_nothing() {
        let offer = offer(None);
        assert!(!offer.enabled);
        assert_eq!(offer.channel, FeedbackChannel::None);
    }

    #[test]
    fn an_override_wins_and_a_blank_one_is_none() {
        let resolved = Resolved::new(
            Some(" https://x/api/feedback "),
            Some("ubiq_ik_abcdefghijk"),
        );
        assert_eq!(resolved.url.source, FeedbackSource::Override);
        assert_eq!(
            resolved.url.value.as_deref(),
            Some("https://x/api/feedback")
        );
        assert_eq!(resolved.key.source, FeedbackSource::Override);
        // A blank override is no override: the field falls through to whatever was compiled in.
        assert_eq!(Resolved::new(Some("  "), None).url, field(None, API_URL));
    }

    #[test]
    fn the_interface_sees_a_prefix_never_the_key() {
        let resolved = Resolved::new(Some("https://x"), Some("ubiq_ik_abcdefghijk"));
        let settings = resolved.settings(None);
        assert!(settings.key_set);
        assert_eq!(settings.key_prefix.as_deref(), Some("ubiq_ik_abcd\u{2026}"));
        assert_eq!(prefix("short"), "short");
    }

    #[test]
    fn the_test_endpoint_hangs_off_the_intake() {
        assert_eq!(
            test_url("https://x/api/feedback"),
            "https://x/api/feedback/test"
        );
        assert_eq!(
            test_url("https://x/api/feedback/"),
            "https://x/api/feedback/test"
        );
        assert_eq!(test_url("https://x"), "https://x/api/feedback/test");
    }
}
