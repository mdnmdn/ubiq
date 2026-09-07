//! The backend for a build with no on-device model: every platform other than macOS, and macOS
//! built without `assist-apple`.
//!
//! This is not a failure mode. It is the supported way to build Ubiq without an AI toolchain — no
//! Swift compiler, no macOS 26 SDK — so every call site has to read correctly against it: the
//! host still answers `GetAssist` honestly, and a `Suggest` still gets a `SuggestError` rather
//! than silence.

use std::sync::Arc;

use ubiq_proto::assist::{AssistLimits, AssistReason};

use super::{Assist, Request, Unavailable, unavailable_message};

pub(crate) fn new_backend() -> Arc<dyn Assist> {
    Arc::new(StubBackend)
}

struct StubBackend;

fn unsupported() -> Unavailable {
    Unavailable {
        reason: AssistReason::UnsupportedPlatform,
        detail: Some("this build has no on-device model behind it".into()),
    }
}

impl Assist for StubBackend {
    fn availability(&self) -> Option<Unavailable> {
        Some(unsupported())
    }

    fn limits(&self) -> AssistLimits {
        AssistLimits {
            label: "On-device model".into(),
            context_tokens: 0,
        }
    }

    fn generate(&self, _request: Request) -> Result<String, String> {
        Err(unavailable_message(&unsupported()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stub_is_never_available_and_says_why() {
        let backend = new_backend();
        let state = backend.availability().expect("the stub is never available");
        assert_eq!(state.reason, AssistReason::UnsupportedPlatform);
        assert_eq!(
            backend
                .generate(Request {
                    instructions: "write a line".into(),
                    prompt: "a change".into(),
                    max_tokens: None,
                })
                .unwrap_err(),
            "assist unavailable: unsupported-platform — this build has no on-device model behind it"
        );
    }
}
