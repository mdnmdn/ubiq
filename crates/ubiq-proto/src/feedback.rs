//! A report the user sends outward: what is wrong, what it looks like, and where it lands.
//!
//! Feedback is the one thing Ubiq says to a service that is neither the user's harness nor the
//! user's repository host. Which service a build reports to is chosen when it is compiled — the
//! host reads `option_env!` values on [`ProviderId`](crate::connectors::ProviderId)'s discipline —
//! and those compiled values are **defaults**: the user may override the intake URL and the key
//! at run time ([`FeedbackSettings`]), field by field, and clearing an override returns that field
//! to the compiled default. A build with neither a default nor an override offers no destination.
//!
//! **The key never reaches the interface.** It goes in once, in a
//! [`Secret`](crate::messages::Secret), on its way to the OS keychain; what comes back is whether
//! one is set, where it came from, and a short prefix to recognise it by.
//!
//! The screenshot rides the report as PNG bytes rather than a path: the interface photographs its
//! own window and the host has no reason to learn where that picture would have been written, and
//! on a remote host there is no such place.

use serde::{Deserialize, Serialize};

use crate::ids::FeedbackId;

/// What a report is about. The set is the user's vocabulary, not a service's — a destination maps
/// it onto its own labels (a GitHub issue turns it into a label, an API into a field).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackKind {
    /// Something is broken.
    #[default]
    Bug,
    /// Something is missing.
    Feature,
    /// Something exists and costs more than it should — time, tokens, watts.
    EcoFeature,
}

impl FeedbackKind {
    /// Every kind, in the order the dropdown offers them.
    pub const ALL: &'static [Self] = &[Self::Bug, Self::Feature, Self::EcoFeature];

    /// What the control says.
    pub fn label(self) -> &'static str {
        match self {
            Self::Bug => "Bug",
            Self::Feature => "Feature",
            Self::EcoFeature => "Eco feature",
        }
    }

    /// The wire and API spelling — what a destination puts in a field or a label.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Feature => "feature",
            Self::EcoFeature => "eco-feature",
        }
    }
}

/// Where a build's reports go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackChannel {
    /// No destination: none compiled in and none configured. The control is drawn and refuses —
    /// the honest state for a source build, and why [`FeedbackOffer::enabled`] exists.
    #[default]
    None,
    /// Ubiq's own intake API: one POST, one API key.
    Api,
    /// An issue opened on a repository host.
    Issues,
}

impl FeedbackChannel {
    /// What the modal says under the send button, so the user knows where their words go.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No feedback destination is configured",
            Self::Api => "Sent to the Ubiq feedback service",
            Self::Issues => "Opened as an issue on the project's tracker",
        }
    }
}

/// Whether this build can send feedback at all, and where to. Answers
/// [`Message::QueryFeedback`](crate::messages::Message).
///
/// The interface asks once at boot and draws the send button disabled while `enabled` is false.
/// The host broadcasts a fresh one whenever the destination's settings change, so the button
/// follows an override without a restart.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackOffer {
    /// Whether a destination is configured and complete. False when the effective URL or the
    /// effective key is missing, and false in a build compiled without the `listener` feature.
    pub enabled: bool,
    /// Which destination, for the sentence under the button.
    pub channel: FeedbackChannel,
}

/// One report, as the user wrote it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackReport {
    /// Minted by the interface, so a reply naming a report it is no longer holding is discarded
    /// by id rather than drawn over a modal the user has closed.
    pub report_id: FeedbackId,
    /// One line. Never empty — the interface refuses to send a blank one.
    pub title: String,
    pub kind: FeedbackKind,
    /// The body, as typed. May be empty; a title alone is still a report.
    pub description: String,
    /// The window as it was the moment the control was pressed, PNG encoded. `None` where the
    /// platform offers no capture, where the user turned it off, or where the capture failed.
    #[serde(default, with = "serde_bytes")]
    pub screenshot: Option<Vec<u8>>,
    /// The build that is reporting: `ubiq::version::FULL`. The first thing anybody triaging asks.
    pub version: String,
    /// `target_os`/`target_arch` of the interface. The second thing they ask.
    pub platform: String,
}

/// What became of a report. Answers [`Message::SendFeedback`](crate::messages::Message) exactly
/// once, to the client that sent it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackReceipt {
    pub report_id: FeedbackId,
    /// What the destination called it — an issue number, a ticket id — when it says one.
    pub reference: Option<String>,
    /// Where the user can go and look, when the destination returns a link.
    pub url: Option<String>,
}

/// Why a report did not land. A sentence the user reads: never a key, never a raw body.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeedbackError {
    pub report_id: FeedbackId,
    pub error: String,
}

/// Where one field of the destination's settings comes from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSource {
    /// Neither compiled in nor overridden.
    #[default]
    None,
    /// The value this build was compiled with.
    Compiled,
    /// The user's override, which wins over the compiled value.
    Override,
}

impl FeedbackSource {
    /// What the badge beside a value says.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "not set",
            Self::Compiled => "compiled",
            Self::Override => "override",
        }
    }
}

/// The destination's settings as the interface may see them. Answers
/// [`Message::QueryFeedbackSettings`](crate::messages::Message) and is broadcast after every
/// [`Message::SetFeedbackSettings`](crate::messages::Message).
///
/// **Never the key.** `key_prefix` is its first few characters, enough to tell two keys apart and
/// not enough to post with.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackSettings {
    /// The intake URL reports are posted to, as it resolves now: the override if one is set, the
    /// compiled value otherwise.
    pub url: Option<String>,
    pub url_source: FeedbackSource,
    /// Whether a key resolves at all, and from where.
    pub key_set: bool,
    pub key_source: FeedbackSource,
    /// The effective key's first characters, for display only.
    pub key_prefix: Option<String>,
    /// Why the last change was not (wholly) kept — a keychain that refused, a settings file that
    /// could not be written. `None` when it was.
    pub error: Option<String>,
}

/// Whether `url` may be an intake override: `https://`, or plain `http://` only to `localhost` or
/// `127.0.0.1` (any port). The key travels in this request, so anything else is refused.
pub fn validate_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if url.starts_with("https://") && url.len() > "https://".len() {
        return Ok(());
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or("");
        let host = host.rsplit_once(':').map_or(host, |(host, _)| host);
        if host == "localhost" || host == "127.0.0.1" {
            return Ok(());
        }
    }
    Err("the URL must start with https:// (http:// only for localhost or 127.0.0.1)".to_string())
}

/// One change to the destination's settings. A `Some` field sets that override; a `clear_*` flag
/// drops it, returning the field to the compiled default. A clear wins over a value sent beside it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackSettingsChange {
    pub url: Option<String>,
    /// Filed in the OS keychain and never sent back.
    pub key: Option<crate::messages::Secret>,
    pub clear_url: bool,
    pub clear_key: bool,
}

/// What a connection test found. Answers [`Message::TestFeedback`](crate::messages::Message), to
/// the asking client alone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackTestResult {
    /// The service accepted the key.
    pub ok: bool,
    /// The name the service has the key under, when it said one.
    pub key_name: Option<String>,
    /// Why it did not work, as a sentence. Never a key, never a raw body.
    pub error: Option<String>,
}
