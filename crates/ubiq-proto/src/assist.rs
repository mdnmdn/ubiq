//! The assist contract: whether a suggestion can be asked for, what answers it, and what is named.
//!
//! Assistance is Ubiq writing a line of prose for the user — a commit message today — and the
//! whole of what crosses the bus is a subject's id and the text that came back. No prompt travels:
//! the prompt is the host's, so the interface cannot drift from it and a log of the bus never
//! carries one. The assist family's message variants live in [`crate::messages`]; this module owns
//! the types that travel inside them.

use serde::{Deserialize, Serialize};

use crate::ids::ProjectId;

/// Why assistance cannot run.
///
/// A closed set rather than a sentence, because the interface renders a reason and never a
/// framework's own words — a message about one platform's model can never surface on another, and
/// a string from a vendor SDK would be exactly how that happens. [`AssistReason::code`] is what
/// the interface matches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssistReason {
    /// This build was not compiled for a platform with a backend behind it.
    UnsupportedPlatform,
    /// A backend exists and the user has turned it off — [`AssistProvider::Off`].
    DisabledBySetting,
    /// The operating system is too old for the backend this platform would use.
    UnsupportedOs,
    /// The machine itself cannot run the model, whatever the OS says.
    DeviceNotEligible,
    /// The platform's own assistance feature has not been switched on by the user.
    NotEnabled,
    /// Everything is in place and the model is still being fetched or prepared.
    ModelNotReady,
    /// Available in principle, unusable right now, for a reason the backend did not decompose.
    Unavailable,
}

impl AssistReason {
    /// The stable string this reason is rendered from.
    ///
    /// The same words the serde form uses, spelled out here so the interface has a value to match
    /// on without round-tripping the reason through JSON.
    pub fn code(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "unsupported-platform",
            Self::DisabledBySetting => "disabled-by-setting",
            Self::UnsupportedOs => "unsupported-os",
            Self::DeviceNotEligible => "device-not-eligible",
            Self::NotEnabled => "not-enabled",
            Self::ModelNotReady => "model-not-ready",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Which backend answers a suggestion.
///
/// [`AssistProvider::Off`] is the default and is today's behaviour, where Ubiq calls no model at
/// all — so upgrading changes nothing until the user picks otherwise. A configured API provider
/// appends a variant here rather than growing a second setting beside this one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssistProvider {
    /// No model is called. Assistance reports itself unavailable with
    /// [`AssistReason::DisabledBySetting`].
    #[default]
    Off,
    /// The platform's own on-device model, running locally with nothing leaving the machine.
    OnDevice,
}

/// What the available backend is, and what it can hold.
///
/// The interface shows `label` and never composes one from the provider name, because which model
/// a platform actually resolved is not a fact the interface holds. The host truncates a subject's
/// material against `context_tokens` before the model sees it, so a large diff is cut where the
/// half that knows the budget can cut it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistLimits {
    /// What the row says — the backend's own name for what is running.
    pub label: String,
    /// How much material the backend will accept, in tokens.
    pub context_tokens: u32,
}

/// What is being named.
///
/// Ids only — never prompt text, never a transcript. The host reads the subject and gathers its
/// own material, which is what keeps the prompt on one side of the bus. One variant per call site,
/// added with that call site rather than ahead of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SuggestSubject {
    /// A commit message for what is staged in this project's repository.
    CommitMessage { project_id: ProjectId },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_reason_renders_its_documented_code() {
        for (reason, code) in [
            (AssistReason::UnsupportedPlatform, "unsupported-platform"),
            (AssistReason::DisabledBySetting, "disabled-by-setting"),
            (AssistReason::UnsupportedOs, "unsupported-os"),
            (AssistReason::DeviceNotEligible, "device-not-eligible"),
            (AssistReason::NotEnabled, "not-enabled"),
            (AssistReason::ModelNotReady, "model-not-ready"),
            (AssistReason::Unavailable, "unavailable"),
        ] {
            assert_eq!(reason.code(), code);
            // The rendered code and the wire form are the same string, so a reason logged from
            // the bus tape and one drawn on screen cannot read differently.
            assert_eq!(
                serde_json::to_string(&reason).expect("a reason serialises"),
                format!("\"{code}\"")
            );
        }
    }
}
