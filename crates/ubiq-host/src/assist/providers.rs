//! The configured API providers: the records, their keys, and the model lists they were asked for.
//!
//! Everything answerable from a file is answered here, on the coordinator's thread — a list, a
//! write, a delete, whether a key is filed, a cached model list. The one thing that touches a
//! network is a model *refresh*, and that goes on a thread of its own, because the coordinator's
//! thread carries keystrokes and must never wait on a handshake. This is the connector family's
//! shape, deliberately, and for the same reasons.
//!
//! **A record and its key are written together or not at all.** The key is filed under the
//! record's id before the record exists, so a row on a settings page always has material behind
//! it, and a delete removes both. That is why there is no `SetSettings` path into
//! [`ubiq_proto::settings::HostSettings::ai_providers`]: the interface asks for a change and the
//! host makes both halves of it.
//!
//! The keychain is the connector family's — [`crate::connectors::store::Store`], one store for
//! the process, in a namespace of its own. A second store over the same directory would be a
//! second answer to "does this platform have a keychain at all".

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};
use ubiq_proto::assist::{
    AiModel, AiModelList, AiProvider, AiProviderDraft, AiProviderInfo, AssistProvider, ModelRole,
};
use ubiq_proto::bus::Mailbox;
use ubiq_proto::ids::AiProviderId;
use ubiq_proto::messages::{Message, Secret};
use ubiq_proto::settings::{HostSettings, SettingsLayer};

use crate::atomic::write_atomic;
use crate::connectors::store::Store;
use crate::reply::Reply;
use crate::settings::Settings;

use super::{ApiKeys, api};

/// The provider family, as the coordinator holds it.
pub struct Providers {
    settings: Arc<Settings>,
    /// The platform's secret store, shared with the connector family — one keychain, one probe.
    store: Arc<Store>,
    /// The model lists, held behind an `Arc` because a refresh reads and writes them from a
    /// thread of its own.
    cache: Arc<Cache>,
}

impl Providers {
    /// Constructing does no keychain I/O; the cache file is read once, and a file that will not
    /// parse is an empty cache rather than an error — a model list is a convenience, and the
    /// remedy is the refresh button that would have been pressed anyway.
    pub fn open(settings: Arc<Settings>, store: Arc<Store>, root: &Path) -> Self {
        Self {
            settings,
            store,
            cache: Arc::new(Cache::open(root.join("ai-models.json"))),
        }
    }

    /// Every provider, with what the secret store adds to it.
    pub fn list(&self) -> Vec<Reply> {
        vec![Reply::Asker(Message::AiProviders {
            providers: self.infos(&self.settings.host()),
        })]
    }

    /// Write a new provider: the key first, then the record, then its models.
    ///
    /// That order is the whole design. A record whose key could not be filed would draw as a
    /// working provider and fail on use; a key with no record is unreachable but harmless, and is
    /// overwritten by the next attempt under the same id — which cannot happen, because the id is
    /// minted here.
    ///
    /// The model listing that follows is why a provider may be added with no model at all. A
    /// picker needs an id to name and a key to call with, and both exist only once this has
    /// written them — so the first thing a new provider does is fetch what it can run, and the
    /// user picks from a list instead of typing a model name from memory. Until they do, the
    /// provider reports itself unavailable with a detail saying which half is missing.
    pub fn add(&self, draft: AiProviderDraft, key: Secret, asker: Mailbox) -> Vec<Reply> {
        let draft = match checked(draft) {
            Ok(draft) => draft,
            Err(reason) => return vec![error(None, &reason)],
        };
        if key.expose().trim().is_empty() {
            return vec![error(None, "a provider needs a key")];
        }
        // Asked before anything is written, for the reason a flow asks it: this application does
        // not keep an API key in a plaintext file, so a machine with no usable secret store is
        // refused here rather than after a record exists.
        if let Err(reason) = self.store.usable() {
            tracing::warn!("no secure credential store: {reason}");
            return vec![error(
                None,
                "this machine has no secure place to keep a key, so none was stored",
            )];
        }

        let id = AiProviderId::generate();
        if let Err(reason) = self.store.set_ai_key(id, key.expose()) {
            return vec![error(None, &format!("the key was not stored: {reason}"))];
        }
        let record = AiProvider {
            id,
            kind: draft.kind,
            name: draft.name,
            base_url: draft.base_url,
            fast_model: draft.fast_model,
            smart_model: draft.smart_model,
        };
        let replies = self.change(move |host| {
            host.ai_providers.push(record);
            Ok(())
        });
        // The record did not land, so the key it was filed under is unreachable. Removed rather
        // than left, because an orphan in a keychain is exactly what this feature must not leave.
        if replies.iter().any(is_error) {
            let _ = self.store.clear_ai_key(id);
            return replies;
        }
        // Answers on its own thread, so this adds nothing to what the caller is told now.
        let listing = self.models(id, true, asker);
        replies.into_iter().chain(listing).collect()
    }

    /// Change a provider that exists.
    ///
    /// `key: None` leaves the stored key alone, which is what an edit that only renames or
    /// re-models has to do — the interface is never told a key, so it cannot send one back. A key
    /// that *is* sent is written before the record, on [`Self::add`]'s reasoning.
    pub fn update(
        &self,
        provider_id: AiProviderId,
        draft: AiProviderDraft,
        key: Option<Secret>,
    ) -> Vec<Reply> {
        let draft = match checked(draft) {
            Ok(draft) => draft,
            Err(reason) => return vec![error(Some(provider_id), &reason)],
        };
        if !self
            .settings
            .host()
            .ai_providers
            .iter()
            .any(|held| held.id == provider_id)
        {
            return vec![error(Some(provider_id), "no such provider")];
        }
        if let Some(key) = key.as_ref().filter(|key| !key.expose().trim().is_empty())
            && let Err(reason) = self.store.set_ai_key(provider_id, key.expose())
        {
            return vec![error(
                Some(provider_id),
                &format!("the key was not stored: {reason}"),
            )];
        }
        self.change(|host| {
            let Some(held) = host
                .ai_providers
                .iter_mut()
                .find(|held| held.id == provider_id)
            else {
                return Err("no such provider".to_string());
            };
            held.kind = draft.kind;
            held.name = draft.name;
            held.base_url = draft.base_url;
            held.fast_model = draft.fast_model;
            held.smart_model = draft.smart_model;
            Ok(())
        })
    }

    /// Remove a provider, its key and its cached model list, together.
    ///
    /// If the assist setting named it, the setting falls back to
    /// [`AssistProvider::Off`] in the same write: a setting pointing at nothing would report
    /// itself unavailable forever, and switching assistance off is the honest reading of "the
    /// provider I chose is gone".
    pub fn forget(&self, provider_id: AiProviderId) -> Vec<Reply> {
        if let Err(reason) = self.store.clear_ai_key(provider_id) {
            // The record goes anyway, on the connector family's reasoning: a provider whose key
            // could not be removed is still one the user asked to be rid of, and leaving the row
            // would strand it.
            tracing::warn!("an ai provider's key was not deleted: {reason}");
        }
        self.cache.forget(provider_id);
        self.change(|host| {
            host.ai_providers.retain(|held| held.id != provider_id);
            if host.assist.names(provider_id) {
                host.assist = AssistProvider::Off;
            }
            Ok(())
        })
    }

    /// The models a provider says it has.
    ///
    /// `refresh: false` is answered from the cache when there is one and calls the provider only
    /// when there is not, which is what makes a model picker openable without a network. A refresh
    /// is a thread, for the reason every other network call here is one.
    pub fn models(&self, provider_id: AiProviderId, refresh: bool, asker: Mailbox) -> Vec<Reply> {
        let host = self.settings.host();
        let Some(record) = host
            .ai_providers
            .iter()
            .find(|held| held.id == provider_id)
            .cloned()
        else {
            return vec![error(Some(provider_id), "no such provider")];
        };
        if !refresh && let Some(list) = self.cache.get(provider_id) {
            return vec![Reply::Asker(Message::AiModels {
                list,
                refreshed: false,
            })];
        }
        let Some(key) = self.store.ai_key_value(provider_id) else {
            return vec![error(
                Some(provider_id),
                "no key is stored for this provider, so its models cannot be listed",
            )];
        };

        let cache = self.cache.clone();
        thread::Builder::new()
            .name(format!("ai-models-{provider_id}"))
            .spawn(move || {
                let message = match api::models(&record, &key) {
                    Ok(models) => {
                        let list = cache.put(provider_id, models);
                        Message::AiModels {
                            list,
                            refreshed: true,
                        }
                    }
                    Err(error) => Message::AiProviderError {
                        provider_id: Some(provider_id),
                        error,
                    },
                };
                asker.send(message);
            })
            .ok();
        Vec::new()
    }

    /// The records, with what the store adds to them.
    ///
    /// Shared with the broadcast after a write — one place that decides what an
    /// [`AiProviderInfo`] is. Which one is selected is deliberately not asked here: that is the
    /// settings blob's, and it travels beside this list on every write.
    fn infos(&self, host: &HostSettings) -> Vec<AiProviderInfo> {
        host.ai_providers
            .iter()
            .map(|provider| AiProviderInfo {
                has_key: self.store.has_ai_key(provider.id),
                provider: provider.clone(),
            })
            .collect()
    }

    /// Change a record and tell everybody. Every write in this file ends the same way: the
    /// settings blob the records live in, then the list drawn from it.
    fn change(&self, edit: impl FnOnce(&mut HostSettings) -> Result<(), String>) -> Vec<Reply> {
        let mut refused = None;
        let written = self.settings.update_host(|host| {
            refused = edit(host).err();
        });
        if let Some(reason) = refused {
            return vec![error(None, &reason)];
        }
        match written {
            Err(reason) => vec![error(None, &reason)],
            Ok(host) => vec![
                Reply::Everyone(Message::Settings {
                    layer: SettingsLayer::Host,
                    value: serde_json::to_string(&host).ok(),
                }),
                Reply::Everyone(Message::AiProviders {
                    providers: self.infos(&host),
                }),
            ],
        }
    }
}

impl ApiKeys for Providers {
    fn key(&self, provider: AiProviderId) -> Option<String> {
        self.store.ai_key_value(provider)
    }

    /// The smallest window any model this provider is configured with was listed as holding.
    ///
    /// The smallest rather than the chosen role's, because a backend reports one
    /// [`ubiq_proto::assist::AssistLimits`] and a subject is cut against it before a role is
    /// known. Cutting to the smaller of the two is right for both and wasteful for neither in
    /// practice — a provider's fast and smart models almost always share a window.
    fn context_tokens(&self, provider: AiProviderId) -> Option<u32> {
        let host = self.settings.host();
        let record = host.ai_providers.iter().find(|held| held.id == provider)?;
        let list = self.cache.get(provider)?;
        let listed = |model: &str| {
            list.models
                .iter()
                .find(|held| held.id == model)
                .and_then(|held| held.context_tokens)
        };
        [
            listed(record.model_for(ModelRole::Fast)),
            record.smart_model.as_deref().and_then(listed),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

// ── The model cache ─────────────────────────────────────────────────

/// The cached model lists, and the file they survive a restart in.
///
/// A cache rather than a settings field: it is derived from what a provider said, the host is the
/// only half that writes it, and losing it costs one button press. That is also why it is not in
/// the settings blob — a list of several hundred model names has no business in a file a user
/// opens to change a preference.
struct Cache {
    path: PathBuf,
    held: Mutex<HashMap<AiProviderId, AiModelList>>,
}

/// The cache as it sits on disk. A version, so a shape change is a discarded file rather than a
/// parse error the user has to find.
#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    lists: Vec<AiModelList>,
}

/// The shape this build writes and reads.
const CACHE_VERSION: u32 = 1;

impl Cache {
    fn open(path: PathBuf) -> Self {
        let held = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
            .filter(|stored| stored.version == CACHE_VERSION)
            .map(|stored| {
                stored
                    .lists
                    .into_iter()
                    .map(|list| (list.provider_id, list))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            path,
            held: Mutex::new(held),
        }
    }

    fn get(&self, provider: AiProviderId) -> Option<AiModelList> {
        self.lock().get(&provider).cloned()
    }

    /// Write a fresh list down, and answer it as it now stands.
    fn put(&self, provider: AiProviderId, models: Vec<AiModel>) -> AiModelList {
        let list = AiModelList {
            provider_id: provider,
            models,
            fetched_at_ms: chrono::Utc::now().timestamp_millis(),
        };
        let mut held = self.lock();
        held.insert(provider, list.clone());
        self.save(&held);
        list
    }

    fn forget(&self, provider: AiProviderId) {
        let mut held = self.lock();
        if held.remove(&provider).is_some() {
            self.save(&held);
        }
    }

    /// Persist the whole map. A failed write is a log line: the cache in memory is still right,
    /// and the cost of losing it is a refresh.
    fn save(&self, held: &HashMap<AiProviderId, AiModelList>) {
        let stored = Stored {
            version: CACHE_VERSION,
            lists: held.values().cloned().collect(),
        };
        match serde_json::to_vec(&stored) {
            Ok(bytes) => {
                if let Err(error) = write_atomic(&self.path, &bytes) {
                    tracing::warn!("the ai model cache was not written: {error}");
                }
            }
            Err(error) => tracing::warn!("the ai model cache would not serialise: {error}"),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<AiProviderId, AiModelList>> {
        self.held
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ── Small shared pieces ─────────────────────────────────────────────

/// A draft with its edges taken off, or the reason it is not a provider.
///
/// Trimmed here rather than in the interface, because "is this a name" is a question the half
/// that writes the record has to answer whatever a field did. A blank base URL and a blank smart
/// model both become `None`: an empty string in either would be a URL nothing resolves and a
/// model nothing answers to.
///
/// **A name is the only thing required.** A provider with no model is a real state and a useful
/// one: it is what a provider looks like between being given a key and having its models listed,
/// and it reports itself unavailable with a detail naming exactly that. Refusing it here would
/// force the user to type a model name before anything could tell them which names exist.
fn checked(draft: AiProviderDraft) -> Result<AiProviderDraft, String> {
    let name = draft.name.trim().to_string();
    if name.is_empty() {
        return Err("a provider needs a name".to_string());
    }
    let fast_model = draft.fast_model.trim().to_string();
    Ok(AiProviderDraft {
        kind: draft.kind,
        name,
        base_url: draft
            .base_url
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty()),
        fast_model,
        smart_model: draft
            .smart_model
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty()),
    })
}

fn error(provider_id: Option<AiProviderId>, reason: &str) -> Reply {
    Reply::Asker(Message::AiProviderError {
        provider_id,
        error: reason.to_string(),
    })
}

fn is_error(reply: &Reply) -> bool {
    matches!(reply.message(), Message::AiProviderError { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::assist::AiProviderKind;

    fn draft(name: &str, fast: &str) -> AiProviderDraft {
        AiProviderDraft {
            kind: AiProviderKind::OpenAiCompatible,
            name: name.to_string(),
            base_url: Some("  ".to_string()),
            fast_model: fast.to_string(),
            smart_model: Some(String::new()),
        }
    }

    #[test]
    fn a_draft_with_no_name_is_refused_and_one_with_no_model_is_not() {
        assert!(checked(draft("  ", "m")).is_err());
        // Half-configured on purpose: a provider is written, its models are listed, and only then
        // can the user pick one.
        let waiting = checked(draft("Somewhere", " ")).expect("a provider may have no model yet");
        assert!(waiting.fast_model.is_empty());
    }

    #[test]
    fn a_blank_endpoint_and_a_blank_smart_model_become_absent() {
        // An interface that sends an empty field rather than omitting one must not produce a URL
        // nothing resolves or a model nothing answers to.
        let checked = checked(draft(" Somewhere ", " quick ")).expect("a complete draft");
        assert_eq!(checked.name, "Somewhere");
        assert_eq!(checked.fast_model, "quick");
        assert_eq!(checked.base_url, None);
        assert_eq!(checked.smart_model, None);
    }

    #[test]
    fn a_cache_round_trips_through_its_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("ai-models.json");
        let provider = AiProviderId::generate();

        let cache = Cache::open(path.clone());
        assert!(cache.get(provider).is_none());
        let list = cache.put(
            provider,
            vec![AiModel {
                id: "quick".into(),
                label: "Quick".into(),
                context_tokens: Some(4_096),
            }],
        );
        assert_eq!(list.models.len(), 1);
        assert!(list.fetched_at_ms > 0);

        // A second cache over the same file is what a restart is.
        let reopened = Cache::open(path.clone());
        let held = reopened.get(provider).expect("the list survived");
        assert_eq!(held.models[0].id, "quick");
        assert_eq!(held.fetched_at_ms, list.fetched_at_ms);

        reopened.forget(provider);
        assert!(Cache::open(path).get(provider).is_none());
    }

    #[test]
    fn a_cache_file_from_another_shape_is_discarded_rather_than_fatal() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("ai-models.json");
        std::fs::write(&path, b"{\"version\":999,\"lists\":[]}").expect("a written file");
        assert!(
            Cache::open(path.clone())
                .get(AiProviderId::generate())
                .is_none()
        );
        std::fs::write(&path, b"not json at all").expect("a written file");
        assert!(Cache::open(path).get(AiProviderId::generate()).is_none());
    }
}
