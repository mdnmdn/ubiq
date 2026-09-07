//! The macOS backend, built on Apple's FoundationModels framework through the
//! `foundation-models` crate. Compiled only for
//! `all(target_os = "macos", feature = "assist-apple")`.
//!
//! Nothing vendor-specific escapes this file: availability states are mapped onto
//! [`OsUnavailable`] and from there onto the contract's own reasons, and every detail sentence is
//! written here rather than forwarded from the framework — a message naming one platform's model
//! must never surface anywhere else.
//!
//! Availability is *asked*, never inferred. There is no OS-version comparison and no device
//! allow-list here: the framework is the only thing that knows, and it is the thing consulted.

use std::sync::Arc;

use foundation_models::{
    Availability, GenerationOptions, LanguageModelSession, SystemLanguageModel, Unavailability,
};
use ubiq_proto::assist::AssistLimits;

use super::{Assist, OsUnavailable, Request, Unavailable, reason_for};

pub(crate) fn new_backend() -> Arc<dyn Assist> {
    Arc::new(AppleBackend)
}

struct AppleBackend;

/// How much material one session will accept, in tokens.
///
/// The crate exposes no accessor for the session's context window, so this is the framework's
/// documented per-session window written down as a ceiling — a constant this will drift from as
/// the framework changes. Read it at runtime the day the crate offers it; do not guess an API.
const CONTEXT_TOKENS: u32 = 4096;

// ── Availability ────────────────────────────────────────────────────

/// The framework's enum crosses into the host's own vocabulary exactly here.
fn os_state(reason: Unavailability) -> OsUnavailable {
    match reason {
        Unavailability::DeviceNotEligible => OsUnavailable::DeviceNotEligible,
        Unavailability::AppleIntelligenceNotEnabled => OsUnavailable::NotEnabled,
        Unavailability::ModelNotReady => OsUnavailable::ModelNotReady,
        Unavailability::OsTooOld => OsUnavailable::OsTooOld,
        _ => OsUnavailable::Unknown,
    }
}

/// Written here rather than forwarded from the framework, so the sentence the user reads names no
/// vendor and reads the same on a machine that has never had this backend.
fn detail_for(state: OsUnavailable) -> &'static str {
    match state {
        OsUnavailable::DeviceNotEligible => "this device does not support on-device assistance",
        OsUnavailable::NotEnabled => "on-device assistance is turned off in system settings",
        OsUnavailable::ModelNotReady => "the on-device model is still downloading or preparing",
        OsUnavailable::OsTooOld => "this operating system has no on-device model API",
        OsUnavailable::Unknown => "the system reports the on-device model as unavailable",
    }
}

fn unavailable(state: OsUnavailable) -> Unavailable {
    Unavailable {
        reason: reason_for(state),
        detail: Some(detail_for(state).to_string()),
    }
}

// ── Generation ──────────────────────────────────────────────────────

/// A fresh session per request: per-request instructions cannot be changed on an existing session,
/// and the framework refuses concurrent requests on one anyway.
fn open_session(request: &Request) -> Result<LanguageModelSession, String> {
    LanguageModelSession::builder()
        .instructions(request.instructions.as_str())
        .map_err(|e| format!("assist instructions: {}", e.message()))?
        .build()
        .map_err(|e| format!("assist session: {}", e.message()))
}

impl Assist for AppleBackend {
    fn availability(&self) -> Option<Unavailable> {
        match SystemLanguageModel::availability() {
            Availability::Available => None,
            Availability::Unavailable(reason) => Some(unavailable(os_state(reason))),
            _ => Some(unavailable(OsUnavailable::Unknown)),
        }
    }

    fn limits(&self) -> AssistLimits {
        AssistLimits {
            label: "On-device model".into(),
            context_tokens: CONTEXT_TOKENS,
        }
    }

    fn generate(&self, request: Request) -> Result<String, String> {
        let session = open_session(&request)?;
        let mut options = GenerationOptions::new();
        if let Some(max_tokens) = request.max_tokens {
            options = options.with_maximum_response_tokens(max_tokens);
        }
        // A guardrail refusal arrives here as an error like any other, and that is an ordinary
        // outcome rather than a bug: the model declined this material, the suggestion does not
        // happen, and the user still has their own words. Nothing was written either way.
        session
            .respond_with(&request.prompt, options)
            .map_err(|e| format!("assist generate: {}", e.message()))
    }
}

#[cfg(test)]
mod tests {
    use ubiq_proto::git::{GitEntry, GitPathChange};

    use super::*;
    use crate::assist::subject;

    #[test]
    fn unavailability_maps_onto_the_contract_reason_codes() {
        let cases = [
            (Unavailability::DeviceNotEligible, "device-not-eligible"),
            (Unavailability::AppleIntelligenceNotEnabled, "not-enabled"),
            (Unavailability::ModelNotReady, "model-not-ready"),
            (Unavailability::OsTooOld, "unsupported-os"),
            (Unavailability::Unknown, "unavailable"),
        ];
        for (reason, expected) in cases {
            assert_eq!(reason_for(os_state(reason)).code(), expected);
        }
    }

    /// The only check that proves generation itself, rather than the mapping around it.
    ///
    /// Ignored by default because it needs the model on the machine and a second of its time, and
    /// because a model's words are not a thing to assert on. It asserts the two things that would
    /// be broken if the bridge were: that something came back, and that it is short enough to be
    /// a commit subject rather than the essay a mis-wired prompt produces. No call site sends
    /// `Suggest` yet, so until one does this is the whole of the end-to-end proof — run it with
    /// `cargo test -p ubiq-host --features assist-apple -- --ignored`.
    #[test]
    #[ignore = "needs the on-device model, and a second of its time"]
    fn the_model_answers_a_commit_message() {
        let backend = AppleBackend;
        if let Some(state) = backend.availability() {
            eprintln!("skipped: assistance is {}", state.reason.code());
            return;
        }

        let changes = [GitEntry {
            rel_path: "crates/ubiq-host/src/assist/mod.rs".to_string(),
            index: Some(GitPathChange::Added),
            worktree: None,
            conflicted: false,
            ignored: false,
        }];
        let text = backend
            .generate(subject::commit_message(&changes, &backend.limits()))
            .expect("the model answers");

        assert!(!text.trim().is_empty(), "the model answered with nothing");
        assert!(
            text.lines().count() <= 3,
            "a commit subject came back as {} lines: {text}",
            text.lines().count()
        );
    }

    #[test]
    fn availability_details_never_name_a_vendor() {
        let states = [
            OsUnavailable::DeviceNotEligible,
            OsUnavailable::NotEnabled,
            OsUnavailable::ModelNotReady,
            OsUnavailable::OsTooOld,
            OsUnavailable::Unknown,
        ];
        for state in states {
            let detail = detail_for(state).to_lowercase();
            for vendor in ["apple", "mac", "foundation", "siri", "intelligence"] {
                assert!(!detail.contains(vendor), "{detail} leaks \"{vendor}\"");
            }
        }
    }
}
