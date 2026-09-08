//! Assistance: the one backend that writes a line of prose, and the choice of which one.
//!
//! Everything vendor-specific lives behind [`Assist`], in a module selected once at compile time,
//! so no caller carries a `#[cfg]` and no framework's own words reach the bus — the interface is
//! told an [`AssistReason`], never a sentence a platform wrote. The prompts themselves live in
//! [`subject`], which is what keeps them on the host's side of the bus.
//!
//! - `api`: the API providers — OpenAI-compatible, Anthropic, Gemini — behind one backend
//! - `providers`: the configured provider records, their keys and their cached model lists
//! - `subject`: what each subject asks for, and the only prompts in the application

use std::sync::Arc;

use ubiq_proto::assist::{AiProvider, AssistLimits, AssistProvider, AssistReason, ModelRole};
use ubiq_proto::ids::AiProviderId;
use ubiq_proto::messages::Message;

mod api;
pub mod providers;

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

    /// Generate, handing text to `sink` as it arrives, and answer the whole of it.
    ///
    /// The default is the honest answer for a backend that cannot stream: generate, hand the
    /// answer over in one piece, and return it. So a caller always gets the same final text and a
    /// backend that streams is the only one that draws differently — which is what lets the
    /// coordinator forward chunks without asking who is behind them.
    ///
    /// **`sink` returning `false` means stop.** That is the whole cancellation story: a
    /// suggestion the user gave up on, or one past its deadline, stops the backend reading rather
    /// than letting it finish into nowhere. Whatever arrived before the refusal is still returned,
    /// and the caller is the half that decides a partial answer is worth nothing.
    fn stream(
        &self,
        request: Request,
        sink: &mut dyn FnMut(&str) -> bool,
    ) -> Result<String, String> {
        let text = self.generate(request)?;
        sink(&text);
        Ok(text)
    }
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
    /// Which of a provider's two models this wants. Meaningless to a backend with one model —
    /// the on-device one ignores it — and the whole choice for an API provider.
    pub role: ModelRole,
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

/// Where an API provider's key and its listed context window come from.
///
/// A trait rather than two arguments so [`select`] stays the one place a backend is chosen without
/// this module knowing that a keychain or a model cache exist. The coordinator implements it once,
/// over the secret store and the cache it already holds.
pub trait ApiKeys {
    /// The key filed under this provider, or `None` when there is none. A backend is still built
    /// for `None`: it reports itself unavailable with a detail naming the missing key, which is a
    /// row the user can act on rather than a provider that silently is not there.
    fn key(&self, provider: AiProviderId) -> Option<String>;
    /// What a cached model list said this provider's chosen model holds, if anything has listed
    /// it. `None` takes the backend's own conservative floor.
    fn context_tokens(&self, provider: AiProviderId) -> Option<u32>;
}

/// The one place a provider is chosen.
///
/// `providers` is [`ubiq_proto::settings::HostSettings::ai_providers`] and `keys` is where their
/// material lives, so every arm of this match is settled from what the host already holds and
/// nothing here does I/O beyond reading a key.
pub fn select(
    provider: &AssistProvider,
    providers: &[AiProvider],
    keys: &dyn ApiKeys,
) -> Arc<dyn Assist> {
    match provider {
        AssistProvider::Off => Arc::new(OffBackend),
        AssistProvider::OnDevice => new_backend(),
        AssistProvider::Api { provider_id } => {
            match providers.iter().find(|held| held.id == *provider_id) {
                Some(record) => api::backend(
                    record.clone(),
                    keys.key(*provider_id).unwrap_or_default(),
                    keys.context_tokens(*provider_id),
                ),
                // The setting names a provider that has been deleted. Answered rather than
                // panicked over, and answered as unavailable rather than as off: the user did
                // choose a provider, and it is gone.
                None => Arc::new(GoneBackend),
            }
        }
    }
}

/// The setting names a provider no record answers to.
///
/// A backend rather than an `Option` at the call site, so `self.assist` is always something and
/// every caller reads one answer for "cannot generate right now" whatever the reason.
struct GoneBackend;

impl Assist for GoneBackend {
    fn availability(&self) -> Option<Unavailable> {
        Some(Unavailable {
            reason: AssistReason::Unavailable,
            detail: Some("the provider this was pointed at no longer exists".into()),
        })
    }

    fn limits(&self) -> AssistLimits {
        AssistLimits {
            label: "Missing provider".into(),
            context_tokens: 0,
        }
    }

    fn generate(&self, _request: Request) -> Result<String, String> {
        Err(unavailable_message(
            &self
                .availability()
                .expect("a missing provider is never available"),
        ))
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

    /// Nothing filed and nothing listed — the answer for every provider a test does not set up.
    struct NoKeys;

    impl ApiKeys for NoKeys {
        fn key(&self, _provider: AiProviderId) -> Option<String> {
            None
        }

        fn context_tokens(&self, _provider: AiProviderId) -> Option<u32> {
            None
        }
    }

    #[test]
    fn the_off_provider_answers_from_the_setting_alone() {
        let message = describe(&*select(&AssistProvider::Off, &[], &NoKeys));
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
    fn a_setting_naming_a_deleted_provider_is_unavailable_rather_than_off() {
        // The distinction matters on screen: "switched off" is a choice the user made and
        // "no longer exists" is one they have to repair.
        let provider = AssistProvider::Api {
            provider_id: AiProviderId::generate(),
        };
        let Message::Assist {
            available, reason, ..
        } = describe(&*select(&provider, &[], &NoKeys))
        else {
            panic!("describe answers with an Assist message");
        };
        assert!(!available);
        assert_eq!(reason, Some(AssistReason::Unavailable));
    }

    #[test]
    fn a_backend_that_cannot_stream_still_answers_a_streaming_caller() {
        // The default `stream` is what lets the coordinator forward chunks without asking which
        // backend is behind them: one chunk, then the same whole text `generate` would have given.
        let mut chunks: Vec<String> = Vec::new();
        let whole = Fake(None)
            .stream(
                Request {
                    instructions: String::new(),
                    prompt: String::new(),
                    max_tokens: None,
                    role: ModelRole::Fast,
                },
                &mut |chunk| {
                    chunks.push(chunk.to_string());
                    true
                },
            )
            .expect("the fake backend generates");
        assert_eq!(whole, "fine");
        assert_eq!(chunks, vec!["fine".to_string()]);
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
