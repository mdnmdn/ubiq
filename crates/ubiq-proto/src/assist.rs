//! The assist contract: whether a suggestion can be asked for, what answers it, and what is named.
//!
//! Assistance is Ubiq writing a line of prose for the user — a commit message today — and the
//! whole of what crosses the bus is a subject's id and the text that came back. No prompt travels:
//! the prompt is the host's, so the interface cannot drift from it and a log of the bus never
//! carries one. The assist family's message variants live in [`crate::messages`]; this module owns
//! the types that travel inside them.
//!
//! An API provider is configured here too — [`AiProvider`] and the [`AiModel`] list a provider is
//! asked for — and one rule governs the whole of it: **a record carries no key.** The key crosses
//! once, in a [`crate::messages::Secret`], on its way to the OS secret store, and every record
//! afterwards says only whether one is filed ([`AiProviderInfo::has_key`]). That is the same shape
//! a connector's token has, for the same reason.

use serde::{Deserialize, Serialize};

use crate::ids::{AiProviderId, ProjectId};

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
/// appends a variant here rather than growing a second setting beside this one, which is why this
/// enum is no longer `Copy`: [`AssistProvider::Api`] names one of the configured providers by id,
/// and the record it names lives in [`crate::settings::HostSettings::ai_providers`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AssistProvider {
    /// No model is called. Assistance reports itself unavailable with
    /// [`AssistReason::DisabledBySetting`].
    #[default]
    Off,
    /// The platform's own on-device model, running locally with nothing leaving the machine.
    OnDevice,
    /// A configured API provider, by id. An id no record answers to is
    /// [`AssistReason::Unavailable`] — the same answer a deleted provider earns, since a record
    /// and its key go together.
    Api { provider_id: AiProviderId },
}

impl AssistProvider {
    /// Whether this setting names that provider.
    ///
    /// The comparison every caller wants, spelled once so none of them builds an `Api` variant
    /// just to compare against one.
    pub fn names(&self, provider: AiProviderId) -> bool {
        matches!(self, Self::Api { provider_id } if *provider_id == provider)
    }
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

// ── API providers ────────────────────────────────────────────

/// Which wire protocol a provider speaks.
///
/// Three, because three is what the API landscape actually is: an OpenAI-compatible chat
/// completion, Anthropic's messages API, and Gemini's generate-content. Everything else a
/// deployment varies — the host, the path prefix, the deployment name — is a base URL, not a
/// fourth kind, which is why [`AiProviderKind::OpenAiCompatible`] covers a local Ollama, a
/// vLLM server and OpenAI itself without a variant each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiProviderKind {
    /// `POST /chat/completions`, bearer key, SSE `data:` frames. OpenAI and everything that
    /// copied it.
    ///
    /// Spelled out rather than left to `kebab-case`, which reads the `Ai` as its own word and
    /// renders `open-ai-compatible` — a second spelling of the one string [`Self::code`] is
    /// there to keep single.
    #[serde(rename = "openai-compatible")]
    OpenAiCompatible,
    /// `POST /v1/messages`, `x-api-key` header, SSE with typed events.
    Anthropic,
    /// `POST /v1beta/models/{model}:streamGenerateContent`, key as a query parameter.
    Gemini,
}

impl AiProviderKind {
    /// The stable string this kind is rendered and filed under.
    ///
    /// The same words the serde form uses, spelled out so a credential's key and a row's label
    /// need no round trip through JSON to agree.
    pub fn code(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "openai-compatible",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
        }
    }

    /// What the row says before the user renames it.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI-compatible",
            Self::Anthropic => "Anthropic",
            Self::Gemini => "Gemini",
        }
    }

    /// Every kind, in the order a picker offers them.
    pub const ALL: [Self; 3] = [Self::OpenAiCompatible, Self::Anthropic, Self::Gemini];
}

/// Which of a provider's two models a request wants.
///
/// Two roles rather than a model per call site, because the choice a user makes is about cost and
/// latency and not about a subject: the fast model names a commit, and the smart one is there for
/// the subjects that will not fit in it. A provider need not configure a smart model at all —
/// [`AiProvider::model_for`] falls back to the fast one, so a subject asking for smart always has
/// something to run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelRole {
    /// The cheap, quick model. Every subject uses this unless it says otherwise.
    #[default]
    Fast,
    /// The capable, slower model, when one is configured.
    Smart,
}

impl ModelRole {
    pub fn code(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Smart => "smart",
        }
    }
}

/// One configured API provider, as it is stored and as the interface is shown it.
///
/// **No key.** The key is in the OS secret store under this record's id, and the only thing said
/// about it here is whether it is there. `base_url` is `None` for the kind's own endpoint, which
/// the host resolves — the contract names no URL, the same way it names no path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProvider {
    /// Minted by the host when the record is written, because a provider exists only once its key
    /// is filed — an abandoned dialog leaves no id behind. Stable across a rename: the name is the
    /// user's, this is what the setting and the stored key reference.
    pub id: AiProviderId,
    pub kind: AiProviderKind,
    /// What the user called it. Several providers of one kind are ordinary — two OpenAI-compatible
    /// endpoints, or one key per project — so the name is what tells them apart on screen.
    pub name: String,
    /// The endpoint, when it is not the kind's own. Stored as the user typed it; the host
    /// normalises a trailing slash rather than making the interface do it.
    #[serde(default)]
    pub base_url: Option<String>,
    /// The model every subject uses unless it asks for the other one.
    pub fast_model: String,
    /// The capable model, when the user picked one.
    #[serde(default)]
    pub smart_model: Option<String>,
}

impl AiProvider {
    /// Which model answers a role. Smart falls back to fast, because a provider without a smart
    /// model must still answer a subject that asked for one.
    pub fn model_for(&self, role: ModelRole) -> &str {
        match role {
            ModelRole::Fast => &self.fast_model,
            ModelRole::Smart => self.smart_model.as_deref().unwrap_or(&self.fast_model),
        }
    }
}

/// What an add or an edit carries: everything on the record except the id the host mints.
///
/// A draft rather than the record itself, so an interface cannot invent an id and a host cannot be
/// asked to trust one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderDraft {
    pub kind: AiProviderKind,
    pub name: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub fast_model: String,
    #[serde(default)]
    pub smart_model: Option<String>,
}

/// One provider as the interface is told about it: the record, plus what the host read out of the
/// secret store without calling anyone.
///
/// **Which provider is selected is not here.** That is `HostSettings.assist`, which every window
/// already mirrors, and [`AssistProvider::names`] is how a row asks — the same reasoning that
/// keeps a pinned certificate off [`crate::messages::ConnectionInfo`]. A copy here would need
/// re-broadcasting whenever the setting moved, and would be a second answer to a question the
/// interface can already answer without asking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiProviderInfo {
    pub provider: AiProvider,
    /// Whether a key is filed under this record's id. Never the key, and never its length.
    pub has_key: bool,
}

/// One model a provider says it has.
///
/// `id` is what a request names; `label` is what the row says, which for two of the three kinds is
/// the provider's own display name and for the third is the id again. `context_tokens` is present
/// when the provider volunteers it, and is what an [`AssistLimits`] is cut against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiModel {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub context_tokens: Option<u32>,
}

/// A provider's model list as the host holds it: the answer, and when it was obtained.
///
/// Cached because a picker that costs a network round trip per keystroke is a picker nobody
/// filters, and because the list changes on the provider's schedule and not on the user's. The
/// host serves this list from its cache and calls the provider only when asked to refresh or when
/// there is nothing cached at all — [`crate::messages::Message::ListAiModels`] carries that
/// choice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiModelList {
    pub provider_id: AiProviderId,
    pub models: Vec<AiModel>,
    /// Epoch milliseconds. What the row under the picker reads, so a stale list says so rather
    /// than looking fresh.
    pub fetched_at_ms: i64,
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
    /// A test of one configured provider, which is the one subject that names its own backend
    /// instead of using the selected one: a user checking a key they just typed is asking about
    /// *that* provider, not about whichever one the setting points at. The prompt is the host's
    /// like every other subject's, and the answer streams so the modal shows a first token rather
    /// than a spinner.
    ProviderCheck {
        provider_id: AiProviderId,
        role: ModelRole,
    },
}

impl SuggestSubject {
    /// Which of a provider's models this subject wants. Naming a commit is the fast model's job;
    /// a check exercises whichever model the user asked about.
    pub fn role(&self) -> ModelRole {
        match self {
            Self::CommitMessage { .. } => ModelRole::Fast,
            Self::ProviderCheck { role, .. } => *role,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> AiProvider {
        AiProvider {
            id: AiProviderId::generate(),
            kind: AiProviderKind::OpenAiCompatible,
            name: "work".into(),
            base_url: None,
            fast_model: "quick".into(),
            smart_model: Some("clever".into()),
        }
    }

    #[test]
    fn every_kind_and_role_renders_its_own_wire_string() {
        for (kind, code) in [
            (AiProviderKind::OpenAiCompatible, "openai-compatible"),
            (AiProviderKind::Anthropic, "anthropic"),
            (AiProviderKind::Gemini, "gemini"),
        ] {
            assert_eq!(kind.code(), code);
            // The credential a key is filed under and the row a picker draws are built from these
            // strings, so the rendered spelling and the wire spelling cannot be allowed to drift.
            assert_eq!(
                serde_json::to_string(&kind).expect("a kind serialises"),
                format!("\"{code}\"")
            );
        }
        assert_eq!(AiProviderKind::ALL.len(), 3, "a new kind needs a pill");
        for (role, code) in [(ModelRole::Fast, "fast"), (ModelRole::Smart, "smart")] {
            assert_eq!(role.code(), code);
            assert_eq!(
                serde_json::to_string(&role).expect("a role serialises"),
                format!("\"{code}\"")
            );
        }
    }

    #[test]
    fn a_role_chooses_a_model_and_smart_falls_back_to_fast() {
        // A provider with no smart model must still answer a subject that asked for one, or a
        // call site would have to know which of the two is configured.
        let mut record = provider();
        assert_eq!(record.model_for(ModelRole::Fast), "quick");
        assert_eq!(record.model_for(ModelRole::Smart), "clever");
        record.smart_model = None;
        assert_eq!(record.model_for(ModelRole::Smart), "quick");
    }

    #[test]
    fn a_subject_names_the_role_it_wants() {
        let record = provider();
        assert_eq!(
            SuggestSubject::CommitMessage {
                project_id: ProjectId::generate()
            }
            .role(),
            ModelRole::Fast
        );
        assert_eq!(
            SuggestSubject::ProviderCheck {
                provider_id: record.id,
                role: ModelRole::Smart,
            }
            .role(),
            ModelRole::Smart
        );
    }

    #[test]
    fn a_provider_record_round_trips_and_carries_nothing_that_looks_like_a_key() {
        let record = provider();
        let json = serde_json::to_string(&record).expect("a record serialises");
        // The rule the whole feature rests on, asserted rather than trusted: a key reaches the
        // secret store and never this record, so a settings file the user opens holds none.
        assert!(
            !json.contains("key"),
            "a provider record grew a key: {json}"
        );
        assert_eq!(
            serde_json::from_str::<AiProvider>(&json).expect("a record parses"),
            record
        );
    }

    #[test]
    fn a_selected_api_provider_is_the_setting_itself() {
        // The setting names a provider by id rather than carrying a copy of it, so a rename or a
        // re-model leaves the choice alone.
        let record = provider();
        let selected = AssistProvider::Api {
            provider_id: record.id,
        };
        let json = serde_json::to_string(&selected).expect("a provider serialises");
        assert_eq!(
            serde_json::from_str::<AssistProvider>(&json).expect("a provider parses"),
            selected
        );
        assert_eq!(AssistProvider::default(), AssistProvider::Off);
    }

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
