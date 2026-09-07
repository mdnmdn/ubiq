//! Assistance: the one backend that writes a line of prose, and the choice of which one.
//!
//! Everything vendor-specific lives behind [`Assist`], in a module selected once at compile time,
//! so no caller carries a `#[cfg]` and no framework's own words reach the bus — the interface is
//! told an [`AssistReason`], never a sentence a platform wrote. The prompts themselves live in
//! [`subject`], which is what keeps them on the host's side of the bus.

use std::sync::Arc;

use ubiq_proto::assist::{AssistLimits, AssistProvider, AssistReason};
use ubiq_proto::messages::Message;

#[cfg(all(target_os = "macos", feature = "assist-apple"))]
mod apple;
#[cfg(not(all(target_os = "macos", feature = "assist-apple")))]
mod stub;
pub mod subject;

#[cfg(all(target_os = "macos", feature = "assist-apple"))]
use apple::new_backend;
#[cfg(not(all(target_os = "macos", feature = "assist-apple")))]
use stub::new_backend;

// ── The backend interface ───────────────────────────────────────────

/// A backend that can produce a short text.
///
/// Blocking on purpose: the host has no async runtime — it is threads and `flume` — and one
/// blocking signature suits a Swift FFI hop and an HTTP POST equally, which is what makes this
/// trait ready for an API provider without changing.
pub trait Assist: Send + Sync {
    /// `None` when it can generate right now.
    fn availability(&self) -> Option<Unavailable>;
    fn limits(&self) -> AssistLimits;
    fn generate(&self, request: Request) -> Result<String, String>;
}

/// Everything one generation needs. Built in [`subject`], consumed by a backend.
///
/// A build without a platform backend rejects before reading any of it, which is why the fields
/// can look unused there.
#[allow(dead_code)]
pub struct Request {
    pub instructions: String,
    pub prompt: String,
    pub max_tokens: Option<u32>,
}

/// Why a backend cannot generate right now.
pub struct Unavailable {
    pub reason: AssistReason,
    pub detail: Option<String>,
}

/// The states an OS-level model reports, kept apart from `AssistReason` so the mapping stays
/// unit-testable in a build compiled without that backend.
///
/// Only a platform backend constructs these; a stub build reaches them from tests alone.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsUnavailable {
    DeviceNotEligible,
    NotEnabled,
    ModelNotReady,
    OsTooOld,
    Unknown,
}

/// The one crossing from an OS's vocabulary into the contract's.
pub fn reason_for(state: OsUnavailable) -> AssistReason {
    match state {
        OsUnavailable::DeviceNotEligible => AssistReason::DeviceNotEligible,
        OsUnavailable::NotEnabled => AssistReason::NotEnabled,
        OsUnavailable::ModelNotReady => AssistReason::ModelNotReady,
        OsUnavailable::OsTooOld => AssistReason::UnsupportedOs,
        OsUnavailable::Unknown => AssistReason::Unavailable,
    }
}

/// The one place the rejection message for an unavailable backend is built.
pub fn unavailable_message(state: &Unavailable) -> String {
    match &state.detail {
        Some(detail) => format!("assist unavailable: {} — {detail}", state.reason.code()),
        None => format!("assist unavailable: {}", state.reason.code()),
    }
}

// ── Choosing one ────────────────────────────────────────────────────

/// The setting's own answer, given without a model and without a device: whether assistance is
/// switched on is a config fact, so it is settled before any FFI call.
struct OffBackend;

impl Assist for OffBackend {
    fn availability(&self) -> Option<Unavailable> {
        Some(Unavailable {
            reason: AssistReason::DisabledBySetting,
            detail: Some("assistance is switched off in settings".into()),
        })
    }

    fn limits(&self) -> AssistLimits {
        AssistLimits {
            label: "Off".into(),
            context_tokens: 0,
        }
    }

    fn generate(&self, _request: Request) -> Result<String, String> {
        Err(unavailable_message(
            &self
                .availability()
                .expect("the off backend is never available"),
        ))
    }
}

/// The one place a provider is chosen. A future API provider is one more arm here and nothing else.
pub fn select(provider: AssistProvider) -> Arc<dyn Assist> {
    match provider {
        AssistProvider::Off => Arc::new(OffBackend),
        AssistProvider::OnDevice => new_backend(),
    }
}

/// Answer `GetAssist`. A free function over `&dyn Assist` so it is testable against a fake
/// backend with no window and no bus.
pub fn describe(assist: &dyn Assist) -> Message {
    match assist.availability() {
        Some(unavailable) => Message::Assist {
            available: false,
            reason: Some(unavailable.reason),
            detail: unavailable.detail,
            limits: None,
        },
        None => Message::Assist {
            available: true,
            reason: None,
            detail: None,
            limits: Some(assist.limits()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A backend with no machine behind it, so `describe` is exercised in both directions.
    struct Fake(Option<AssistReason>);

    impl Assist for Fake {
        fn availability(&self) -> Option<Unavailable> {
            self.0.map(|reason| Unavailable {
                reason,
                detail: Some("because".into()),
            })
        }

        fn limits(&self) -> AssistLimits {
            AssistLimits {
                label: "Fake model".into(),
                context_tokens: 128,
            }
        }

        fn generate(&self, _request: Request) -> Result<String, String> {
            Ok("fine".into())
        }
    }

    #[test]
    fn every_os_state_maps_onto_a_reason() {
        for (state, reason) in [
            (
                OsUnavailable::DeviceNotEligible,
                AssistReason::DeviceNotEligible,
            ),
            (OsUnavailable::NotEnabled, AssistReason::NotEnabled),
            (OsUnavailable::ModelNotReady, AssistReason::ModelNotReady),
            (OsUnavailable::OsTooOld, AssistReason::UnsupportedOs),
            (OsUnavailable::Unknown, AssistReason::Unavailable),
        ] {
            assert_eq!(reason_for(state), reason);
        }
    }

    #[test]
    fn the_off_provider_answers_from_the_setting_alone() {
        let message = describe(&*select(AssistProvider::Off));
        let Message::Assist {
            available, reason, ..
        } = message
        else {
            panic!("describe answers with an Assist message");
        };
        assert!(!available);
        assert_eq!(reason, Some(AssistReason::DisabledBySetting));
    }

    #[test]
    fn an_available_backend_is_described_with_its_limits() {
        let Message::Assist {
            available,
            reason,
            limits,
            ..
        } = describe(&Fake(None))
        else {
            panic!("describe answers with an Assist message");
        };
        assert!(available);
        assert_eq!(reason, None);
        assert_eq!(
            limits.expect("an available backend has limits").label,
            "Fake model"
        );
    }

    #[test]
    fn an_unavailable_backend_is_described_without_limits() {
        let Message::Assist {
            available,
            reason,
            detail,
            limits,
        } = describe(&Fake(Some(AssistReason::ModelNotReady)))
        else {
            panic!("describe answers with an Assist message");
        };
        assert!(!available);
        assert_eq!(reason, Some(AssistReason::ModelNotReady));
        assert_eq!(detail.as_deref(), Some("because"));
        assert!(limits.is_none());
    }
}
