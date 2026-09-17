//! A report the user sends outward: what is wrong, what it looks like, and where it lands.
//!
//! Feedback is the one thing Ubiq says to a service that is neither the user's harness nor the
//! user's repository host — so the *destination* is not a setting. Which service a build reports
//! to is a property of the build, chosen when it is compiled and stated to the interface as
//! [`FeedbackChannel`]; the host resolves it from `option_env!` values on
//! [`ProviderId`](crate::connectors::ProviderId)'s discipline, and a build compiled without them
//! offers no destination at all.
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

/// Where a build's reports go. Compiled in, never configured at run time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackChannel {
    /// This build was compiled without a destination. The control is drawn and refuses — the
    /// honest state for a source build, and why [`FeedbackOffer::enabled`] exists.
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
            Self::None => "This build has no feedback destination",
            Self::Api => "Sent to the Ubiq feedback service",
            Self::Issues => "Opened as an issue on the project's tracker",
        }
    }
}

/// Whether this build can send feedback at all, and where to. Answers
/// [`Message::QueryFeedback`](crate::messages::Message).
///
/// The interface asks once at boot and draws the send button disabled while `enabled` is false —
/// the missing key is a fact about the binary, so it cannot change while the process runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedbackOffer {
    /// Whether a destination is configured and complete. False when the URL or the key is
    /// missing, and false in a build compiled without the `listener` feature.
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
