//! The API-provider backend: three wire protocols behind one [`Assist`].
//!
//! One type rather than three, because what differs between an OpenAI-compatible chat completion,
//! Anthropic's messages API and Gemini's generate-content is a URL, a header and where the text
//! sits in a JSON object — and a backend each would triple the paths a suggestion can take to
//! reach a pane. The three differences live in [`endpoint`], [`headers`] and [`delta`], and every
//! one of them is a `match` on [`AiProviderKind`] with three arms and no default.
//!
//! **Streaming is the normal path here, not an addition.** Every one of the three answers Server-
//! Sent Events, so [`ApiBackend::stream`] is where the work is and `generate` is the arm that
//! throws the pieces away. The sink returning `false` is how a cancelled suggestion stops a
//! response mid-body: the reader stops reading and the connection is dropped, which is the only
//! cancellation an HTTP request has.
//!
//! Nothing here reads the settings file and nothing here reaches the secret store. A backend is
//! constructed with the record and the key it will use, so the half that holds credentials decides
//! once whether this provider may be called at all.

use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use ubiq_proto::assist::{AiModel, AiProvider, AiProviderKind, AssistLimits, AssistReason};

use super::{Assist, Request, Unavailable};

/// What a provider is assumed to hold when nothing has said otherwise.
///
/// Deliberately small. `limits().context_tokens` is what [`super::subject`] cuts a diff against,
/// and cutting more than necessary costs a suggestion some detail, while cutting less than the
/// model accepts costs the request itself. A cached model list that named a window is passed in
/// and used instead — this is the floor for a provider nobody has listed yet.
const CONTEXT_TOKENS: u32 = 16_384;

/// The response ceiling for a subject that named none. Anthropic's API requires the field, so
/// there has to be a number for the case where the caller had no opinion.
const MAX_TOKENS: u32 = 1024;

/// How long to wait for a connection, and then for each read once one is open.
///
/// Two timeouts rather than one on the whole call: a total budget would cut a long answer that is
/// arriving perfectly well, and a stall is what wants catching. The suggestion's own deadline in
/// the coordinator is the ceiling on the whole thing.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// The name the provider sees.
const USER_AGENT: &str = concat!("ubiq/", env!("CARGO_PKG_VERSION"));

// ── The backend ─────────────────────────────────────────────────────

/// One configured provider, ready to call: the record, the key, and the window it is believed to
/// hold. No handle and no connection — an agent is built per request, the way the connector
/// family's is.
pub struct ApiBackend {
    provider: AiProvider,
    key: String,
    context_tokens: u32,
}

/// The backend for one provider.
///
/// `context_tokens` comes from the cached model list when there is one; `None` takes
/// [`CONTEXT_TOKENS`].
pub fn backend(provider: AiProvider, key: String, context_tokens: Option<u32>) -> Arc<dyn Assist> {
    Arc::new(ApiBackend {
        provider,
        key,
        context_tokens: context_tokens.unwrap_or(CONTEXT_TOKENS),
    })
}

impl Assist for ApiBackend {
    /// Answered without a network call, and that is the point: whether this provider *could* be
    /// called is a fact about the record and the keychain, and asking the provider itself would
    /// spend a request to draw a settings row.
    fn availability(&self) -> Option<Unavailable> {
        let missing = if self.key.trim().is_empty() {
            Some("no key is stored for this provider")
        } else if self.provider.fast_model.trim().is_empty() {
            Some("this provider has no model configured")
        } else {
            None
        };
        missing.map(|detail| Unavailable {
            reason: AssistReason::Unavailable,
            detail: Some(detail.to_string()),
        })
    }

    fn limits(&self) -> AssistLimits {
        AssistLimits {
            label: format!("{} · {}", self.provider.name, self.provider.fast_model),
            context_tokens: self.context_tokens,
        }
    }

    /// The whole answer, by streaming one and keeping the pieces. There is no second request
    /// shape: a provider that streams is asked to stream, and a caller with nowhere to put a
    /// first token simply has no sink.
    fn generate(&self, request: Request) -> Result<String, String> {
        self.stream(request, &mut |_| true)
    }

    fn stream(
        &self,
        request: Request,
        sink: &mut dyn FnMut(&str) -> bool,
    ) -> Result<String, String> {
        let model = self.provider.model_for(request.role).to_string();
        let url = endpoint(&self.provider, Call::Generate { model: &model });
        let body = body(self.provider.kind, &model, &request);

        let mut post = agent().post(&url).set("User-Agent", USER_AGENT);
        for (name, value) in headers(self.provider.kind, &self.key) {
            post = post.set(name, &value);
        }
        let response = post.send_json(body).map_err(refusal)?;

        let mut reader = BufReader::new(response.into_reader());
        let mut line = String::new();
        let mut whole = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) => {
                    // A stream that died halfway is a failure even though text arrived: the
                    // caller is holding a fragment, and a fragment presented as a suggestion is
                    // worse than a sentence saying what went wrong.
                    return Err(format!("the answer stopped early: {error}"));
                }
            }
            // An SSE frame is `field: value`, and the only field with an answer in it is `data`.
            // `event:`, `id:`, comments and the blank line between frames are all skipped here,
            // which is what makes one reader serve three providers.
            let Some(payload) = line.trim_end().strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            // OpenAI-compatible servers end with a sentinel that is not JSON. The other two end
            // by closing the body, so both endings are covered.
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            if let Some(error) = wire_error(&value) {
                return Err(error);
            }
            let text = delta(self.provider.kind, &value);
            if text.is_empty() {
                continue;
            }
            whole.push_str(&text);
            // The sink refusing means the suggestion was cancelled or timed out. Stopping here
            // drops the connection, which is the only way to stop a provider generating.
            if !sink(&text) {
                break;
            }
        }

        if whole.trim().is_empty() {
            return Err("the model answered with nothing".to_string());
        }
        Ok(whole)
    }
}

// ── The three differences ───────────────────────────────────────────

/// Which request is being addressed. The two calls a provider gets asked for.
enum Call<'a> {
    /// The one generation. Gemini names the model in the path, so it is a parameter here rather
    /// than a body field for all three.
    Generate { model: &'a str },
    /// The model list.
    Models,
}

/// The endpoint a provider's own base URL resolves to.
///
/// A user's `base_url` replaces the vendor's host and path prefix and nothing else, so a local
/// Ollama at `http://localhost:11434/v1` and OpenAI itself take the same suffix. The trailing
/// slash is trimmed here so the two spellings of one endpoint cannot both exist.
fn endpoint(provider: &AiProvider, call: Call<'_>) -> String {
    let base = provider
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .unwrap_or_else(|| default_base(provider.kind))
        .trim_end_matches('/')
        .to_string();
    match (provider.kind, call) {
        (AiProviderKind::OpenAiCompatible, Call::Generate { .. }) => {
            format!("{base}/chat/completions")
        }
        (AiProviderKind::OpenAiCompatible, Call::Models) => format!("{base}/models"),
        (AiProviderKind::Anthropic, Call::Generate { .. }) => format!("{base}/v1/messages"),
        (AiProviderKind::Anthropic, Call::Models) => format!("{base}/v1/models?limit=200"),
        // `alt=sse` is what turns Gemini's streaming answer from a JSON array into the same
        // Server-Sent Events the other two speak, which is the whole reason one reader works.
        (AiProviderKind::Gemini, Call::Generate { model }) => {
            format!("{base}/v1beta/models/{model}:streamGenerateContent?alt=sse")
        }
        (AiProviderKind::Gemini, Call::Models) => format!("{base}/v1beta/models?pageSize=1000"),
    }
}

/// Where a kind's requests go when the user named no endpoint.
///
/// **The only place a vendor host is spelled in this application.** The contract names none, and
/// the interface is never told one — a row on a settings page shows what the user typed or
/// nothing at all.
fn default_base(kind: AiProviderKind) -> &'static str {
    match kind {
        AiProviderKind::OpenAiCompatible => "https://api.openai.com/v1",
        AiProviderKind::Anthropic => "https://api.anthropic.com",
        AiProviderKind::Gemini => "https://generativelanguage.googleapis.com",
    }
}

/// How a kind carries its key, and whatever else it insists on.
///
/// Three different answers to one question, which is why this is a function and not a constant:
/// a bearer token, a bare header, and a Google API key header. Anthropic also pins the API
/// version, and that version is the contract this code was written against — a provider that
/// changed shape under a later one would break silently without it.
fn headers(kind: AiProviderKind, key: &str) -> Vec<(&'static str, String)> {
    match kind {
        AiProviderKind::OpenAiCompatible => {
            vec![("Authorization", format!("Bearer {key}"))]
        }
        AiProviderKind::Anthropic => vec![
            ("x-api-key", key.to_string()),
            ("anthropic-version", "2023-06-01".to_string()),
        ],
        AiProviderKind::Gemini => vec![("x-goog-api-key", key.to_string())],
    }
}

/// The request body, in each kind's own shape.
///
/// The instructions and the prompt are one thing to the host and two things to every provider —
/// a system field and a user turn — so this is where [`Request`] is taken apart. `stream` is
/// always on: there is no non-streaming path here.
fn body(kind: AiProviderKind, model: &str, request: &Request) -> Value {
    let max_tokens = request.max_tokens.unwrap_or(MAX_TOKENS);
    match kind {
        AiProviderKind::OpenAiCompatible => json!({
            "model": model,
            "stream": true,
            "max_tokens": max_tokens,
            "messages": [
                {"role": "system", "content": request.instructions},
                {"role": "user", "content": request.prompt},
            ],
        }),
        AiProviderKind::Anthropic => json!({
            "model": model,
            "stream": true,
            // Required rather than optional on this API, which is why `MAX_TOKENS` exists.
            "max_tokens": max_tokens,
            "system": request.instructions,
            "messages": [{"role": "user", "content": request.prompt}],
        }),
        AiProviderKind::Gemini => json!({
            "systemInstruction": {"parts": [{"text": request.instructions}]},
            "contents": [{"role": "user", "parts": [{"text": request.prompt}]}],
            "generationConfig": {"maxOutputTokens": max_tokens},
        }),
    }
}

/// The text in one Server-Sent Event, wherever that kind keeps it.
///
/// Every frame that is not text — a role announcement, a usage report, a keep-alive, a block
/// boundary — reads as an empty string here rather than as an error, because a stream is mostly
/// those and a reader that objected to them could not read one.
fn delta(kind: AiProviderKind, value: &Value) -> String {
    match kind {
        AiProviderKind::OpenAiCompatible => value["choices"][0]["delta"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        // The typed events are the whole shape of this stream: only `content_block_delta` carries
        // prose, and only its `text_delta` variant carries prose that is not a tool argument.
        AiProviderKind::Anthropic => {
            if value["type"] == "content_block_delta" && value["delta"]["type"] == "text_delta" {
                value["delta"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            } else {
                String::new()
            }
        }
        // A Gemini chunk is a whole candidate with however many parts it felt like sending, so
        // the parts are joined rather than indexed.
        AiProviderKind::Gemini => value["candidates"][0]["content"]["parts"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|part| part["text"].as_str())
                    .collect::<String>()
            })
            .unwrap_or_default(),
    }
}

/// A provider saying no inside a 200 response.
///
/// All three can do it — a rate limit reached mid-stream, a content refusal, a model that went
/// away — and none of them closes the connection to say so. Without this the stream would simply
/// end and the answer would be "the model answered with nothing", which names the wrong problem.
fn wire_error(value: &Value) -> Option<String> {
    let message = value["error"]["message"]
        .as_str()
        .or_else(|| value["error"].as_str())?;
    Some(message.to_string())
}

// ── The model list ──────────────────────────────────────────────────

/// The models a provider says it has.
///
/// Blocking, like everything else here, and called from a worker thread. The order is the
/// provider's own — for two of the three that is newest first, and imposing an order of Ubiq's
/// own would hide that.
pub fn models(provider: &AiProvider, key: &str) -> Result<Vec<AiModel>, String> {
    let url = endpoint(provider, Call::Models);
    let mut get = agent()
        .get(&url)
        .set("Accept", "application/json")
        .set("User-Agent", USER_AGENT);
    for (name, value) in headers(provider.kind, key) {
        get = get.set(name, &value);
    }
    let value: Value = get
        .call()
        .map_err(refusal)?
        .into_json()
        .map_err(|error| format!("the model list did not parse: {error}"))?;
    let models = read_models(provider.kind, &value);
    if models.is_empty() {
        return Err("the provider listed no models".to_string());
    }
    Ok(models)
}

/// One kind's model list, out of its own answer's shape.
///
/// Split from [`models`] so the three shapes are testable against a fixture with no network and
/// no key.
fn read_models(kind: AiProviderKind, value: &Value) -> Vec<AiModel> {
    match kind {
        AiProviderKind::OpenAiCompatible => value["data"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| row["id"].as_str())
                    .map(|id| AiModel {
                        id: id.to_string(),
                        // This API volunteers no display name and no window, so the id is both
                        // the value and the label. Rows read as bare ids because they are.
                        label: id.to_string(),
                        context_tokens: None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        AiProviderKind::Anthropic => value["data"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        let id = row["id"].as_str()?;
                        Some(AiModel {
                            id: id.to_string(),
                            label: row["display_name"].as_str().unwrap_or(id).to_string(),
                            context_tokens: row["max_input_tokens"].as_u64().map(clamp_tokens),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        // Gemini names a model `models/gemini-…` in its listing and `gemini-…` in a request path,
        // so the prefix is dropped here and one spelling reaches everything else. Models that
        // cannot answer a generation at all — embedders, rankers — are dropped with it.
        AiProviderKind::Gemini => value["models"]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .filter(|row| {
                        row["supportedGenerationMethods"]
                            .as_array()
                            .is_none_or(|methods| {
                                methods.iter().any(|method| method == "generateContent")
                            })
                    })
                    .filter_map(|row| {
                        let name = row["name"].as_str()?;
                        // `strip_prefix` rather than `trim_start_matches`, which strips a repeated
                        // prefix: one `models/` comes off, and a model actually called
                        // `models/models/x` keeps the name it has.
                        let id = name.strip_prefix("models/").unwrap_or(name);
                        Some(AiModel {
                            id: id.to_string(),
                            label: row["displayName"].as_str().unwrap_or(id).to_string(),
                            context_tokens: row["inputTokenLimit"].as_u64().map(clamp_tokens),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A window as a `u32`, since a provider reports one as an unbounded integer and
/// [`AssistLimits`] holds 32 bits.
fn clamp_tokens(tokens: u64) -> u32 {
    tokens.min(u64::from(u32::MAX)) as u32
}

// ── HTTP ────────────────────────────────────────────────────────────

/// One request's client.
///
/// Built per request rather than pooled, on the connector family's reasoning: a pooled agent
/// outlives whatever it was built with, and a handshake costs more than a builder does.
///
/// **The machine's trust rules and nothing else.** There is no pin here and no certificate
/// question to ask, unlike a connector: an assist provider has no confirmation flow, so a
/// certificate the machine refuses is an error the user reads rather than a question they answer.
/// The crypto provider and the root store are the connector family's, named rather than
/// reinherited so there is one rustls and one set of anchors in the process.
fn agent() -> ureq::Agent {
    let config = rustls::ClientConfig::builder_with_provider(crate::connectors::tls::provider())
        .with_safe_default_protocol_versions()
        // The provider names both TLS versions; a set it cannot serve would be a build error.
        .expect("the tls versions ring supports")
        .with_root_certificates(crate::connectors::tls::roots())
        .with_no_client_auth();
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .tls_config(Arc::new(config))
        .build()
}

/// What a refused request means, in one sentence a settings page can show.
///
/// A provider says why in a JSON body, and every one of the three uses `error.message` for it, so
/// the body is worth reading before falling back to the status line. Truncated, because a refusal
/// can carry a page of prose and this is going into a row.
fn refusal(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => {
            let body = response.into_string().unwrap_or_default();
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .as_ref()
                .and_then(wire_error)
                .unwrap_or_else(|| body.trim().chars().take(300).collect::<String>());
            match detail.trim() {
                "" => format!("the provider refused the request: HTTP {code}"),
                detail => format!("the provider refused the request: HTTP {code}: {detail}"),
            }
        }
        ureq::Error::Transport(transport) => {
            format!("the provider could not be reached: {transport}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::assist::ModelRole;

    fn provider(kind: AiProviderKind, base_url: Option<&str>) -> AiProvider {
        AiProvider {
            id: ubiq_proto::ids::AiProviderId::generate(),
            kind,
            name: "Somewhere".into(),
            base_url: base_url.map(str::to_string),
            fast_model: "quick".into(),
            smart_model: Some("clever".into()),
        }
    }

    #[test]
    fn a_provider_with_no_endpoint_takes_its_kinds_own() {
        let url = endpoint(
            &provider(AiProviderKind::OpenAiCompatible, None),
            Call::Generate { model: "quick" },
        );
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn a_trailing_slash_does_not_double() {
        let url = endpoint(
            &provider(
                AiProviderKind::OpenAiCompatible,
                Some("http://box:11434/v1/"),
            ),
            Call::Models,
        );
        assert_eq!(url, "http://box:11434/v1/models");
    }

    #[test]
    fn geminis_model_goes_in_the_path_and_the_stream_is_asked_for() {
        let url = endpoint(
            &provider(AiProviderKind::Gemini, None),
            Call::Generate { model: "gemini-x" },
        );
        assert!(
            url.ends_with("/v1beta/models/gemini-x:streamGenerateContent?alt=sse"),
            "{url}"
        );
    }

    #[test]
    fn a_blank_endpoint_is_no_endpoint() {
        // An interface that sends an empty field rather than omitting one must not produce a URL
        // that begins with the suffix alone.
        let url = endpoint(
            &provider(AiProviderKind::Anthropic, Some("   ")),
            Call::Generate { model: "any" },
        );
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn a_role_chooses_the_model_and_smart_falls_back() {
        let mut record = provider(AiProviderKind::Anthropic, None);
        assert_eq!(record.model_for(ModelRole::Fast), "quick");
        assert_eq!(record.model_for(ModelRole::Smart), "clever");
        record.smart_model = None;
        assert_eq!(record.model_for(ModelRole::Smart), "quick");
    }

    #[test]
    fn each_kind_finds_its_text_and_ignores_every_other_frame() {
        let openai = json!({"choices": [{"delta": {"content": "one"}}]});
        assert_eq!(delta(AiProviderKind::OpenAiCompatible, &openai), "one");
        assert_eq!(
            delta(
                AiProviderKind::OpenAiCompatible,
                &json!({"choices": [{"delta": {"role": "assistant"}}]})
            ),
            ""
        );

        let anthropic = json!({
            "type": "content_block_delta",
            "delta": {"type": "text_delta", "text": "two"},
        });
        assert_eq!(delta(AiProviderKind::Anthropic, &anthropic), "two");
        assert_eq!(
            delta(AiProviderKind::Anthropic, &json!({"type": "message_stop"})),
            ""
        );

        let gemini =
            json!({"candidates": [{"content": {"parts": [{"text": "th"}, {"text": "ree"}]}}]});
        assert_eq!(delta(AiProviderKind::Gemini, &gemini), "three");
        assert_eq!(
            delta(AiProviderKind::Gemini, &json!({"usageMetadata": {}})),
            ""
        );
    }

    #[test]
    fn a_refusal_inside_a_stream_is_found() {
        let value = json!({"type": "error", "error": {"message": "over the limit"}});
        assert_eq!(wire_error(&value).as_deref(), Some("over the limit"));
        assert!(wire_error(&json!({"type": "ping"})).is_none());
    }

    #[test]
    fn each_kinds_model_list_reads_back() {
        let openai = json!({"data": [{"id": "gpt-x"}, {"id": "gpt-y"}]});
        let models = read_models(AiProviderKind::OpenAiCompatible, &openai);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-x");
        assert_eq!(models[0].label, "gpt-x");

        let anthropic = json!({"data": [
            {"id": "claude-x", "display_name": "Claude X", "max_input_tokens": 200_000},
        ]});
        let models = read_models(AiProviderKind::Anthropic, &anthropic);
        assert_eq!(models[0].id, "claude-x");
        assert_eq!(models[0].label, "Claude X");
        assert_eq!(models[0].context_tokens, Some(200_000));

        let gemini = json!({"models": [
            {
                "name": "models/gemini-x",
                "displayName": "Gemini X",
                "inputTokenLimit": 1_048_576,
                "supportedGenerationMethods": ["generateContent"],
            },
            {
                "name": "models/embedder",
                "displayName": "Embedder",
                "supportedGenerationMethods": ["embedContent"],
            },
        ]});
        let models = read_models(AiProviderKind::Gemini, &gemini);
        // The embedder is dropped: it cannot answer a generation, so it cannot be a choice.
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gemini-x");
        assert_eq!(models[0].context_tokens, Some(1_048_576));
    }

    #[test]
    fn a_provider_without_a_key_or_a_model_says_which() {
        let no_key = ApiBackend {
            provider: provider(AiProviderKind::Anthropic, None),
            key: String::new(),
            context_tokens: CONTEXT_TOKENS,
        };
        let unavailable = no_key.availability().expect("no key is unavailable");
        assert_eq!(unavailable.reason, AssistReason::Unavailable);
        assert!(
            unavailable
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("key")),
            "the detail names the missing thing"
        );

        let mut record = provider(AiProviderKind::Anthropic, None);
        record.fast_model = String::new();
        let no_model = ApiBackend {
            provider: record,
            key: "k".into(),
            context_tokens: CONTEXT_TOKENS,
        };
        assert!(
            no_model
                .availability()
                .and_then(|unavailable| unavailable.detail)
                .is_some_and(|detail| detail.contains("model"))
        );
    }

    #[test]
    fn a_configured_provider_is_available_and_labels_itself() {
        let ready = ApiBackend {
            provider: provider(AiProviderKind::Gemini, None),
            key: "k".into(),
            context_tokens: 4_096,
        };
        assert!(ready.availability().is_none());
        let limits = ready.limits();
        assert_eq!(limits.label, "Somewhere · quick");
        assert_eq!(limits.context_tokens, 4_096);
    }

    #[test]
    fn a_body_carries_the_instructions_where_its_kind_keeps_them() {
        let request = Request {
            instructions: "be brief".into(),
            prompt: "name this".into(),
            max_tokens: Some(32),
            role: ModelRole::Fast,
        };
        let openai = body(AiProviderKind::OpenAiCompatible, "m", &request);
        assert_eq!(openai["messages"][0]["content"], json!("be brief"));
        assert_eq!(openai["stream"], json!(true));

        let anthropic = body(AiProviderKind::Anthropic, "m", &request);
        assert_eq!(anthropic["system"], json!("be brief"));
        assert_eq!(anthropic["max_tokens"], json!(32));

        let gemini = body(AiProviderKind::Gemini, "m", &request);
        assert_eq!(
            gemini["systemInstruction"]["parts"][0]["text"],
            json!("be brief")
        );
        assert_eq!(gemini["generationConfig"]["maxOutputTokens"], json!(32));
    }

    #[test]
    fn a_subject_with_no_ceiling_still_gets_one() {
        // Anthropic's API refuses a request without `max_tokens`, so there is no such thing as
        // "no opinion" on the wire.
        let request = Request {
            instructions: String::new(),
            prompt: String::new(),
            max_tokens: None,
            role: ModelRole::Fast,
        };
        assert_eq!(
            body(AiProviderKind::Anthropic, "m", &request)["max_tokens"],
            json!(MAX_TOKENS)
        );
    }

    #[test]
    fn a_key_travels_in_each_kinds_own_header() {
        assert_eq!(
            headers(AiProviderKind::OpenAiCompatible, "k"),
            vec![("Authorization", "Bearer k".to_string())]
        );
        let anthropic = headers(AiProviderKind::Anthropic, "k");
        assert!(anthropic.contains(&("x-api-key", "k".to_string())));
        assert!(
            anthropic
                .iter()
                .any(|(name, _)| *name == "anthropic-version")
        );
        assert_eq!(
            headers(AiProviderKind::Gemini, "k"),
            vec![("x-goog-api-key", "k".to_string())]
        );
    }
}
