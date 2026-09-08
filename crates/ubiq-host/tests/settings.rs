//! Application settings: two layers, two files, two recovery rules.

use std::fs;

use tempfile::TempDir;
use ubiq_host::store::file::FileSettingsStore;
use ubiq_host::store::memory::MemorySettingsStore;
use ubiq_host::store::{SettingsStore, StoreError};
use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings, SettingsLayer};

const UI_BLOB: &str = r#"{"schema":1,"explorer_preview":true,"markdown_open":"preview"}"#;

#[test]
fn each_layer_has_its_own_file() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());

    assert_eq!(
        store.path(SettingsLayer::Ui),
        dir.path().join("ui-settings.toml")
    );
    assert_eq!(
        store.path(SettingsLayer::Host),
        dir.path().join("host-settings.toml")
    );
}

#[test]
fn a_ui_blob_comes_back_exactly_as_it_went_in() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());

    store.set(SettingsLayer::Ui, UI_BLOB).unwrap();
    assert_eq!(
        store.get(SettingsLayer::Ui).unwrap().as_deref(),
        Some(UI_BLOB)
    );
}

#[test]
fn a_ui_blob_with_toml_metacharacters_survives() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());
    let awkward = "line\nbreak \"quotes\" [brackets] = and 'single'";

    store.set(SettingsLayer::Ui, awkward).unwrap();
    assert_eq!(
        store.get(SettingsLayer::Ui).unwrap().as_deref(),
        Some(awkward)
    );
}

#[test]
fn never_set_is_not_the_same_as_empty() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());

    assert_eq!(store.get(SettingsLayer::Ui).unwrap(), None);

    store.set(SettingsLayer::Ui, "").unwrap();
    assert_eq!(store.get(SettingsLayer::Ui).unwrap(), Some(String::new()));
}

#[test]
fn unreadable_ui_settings_are_discarded_rather_than_preserved() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());
    let path = store.path(SettingsLayer::Ui);
    fs::write(&path, "= not an envelope =").unwrap();

    assert_eq!(store.get(SettingsLayer::Ui).unwrap(), None);
}

#[test]
fn host_settings_round_trip_as_json_on_the_wire() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());
    let value = serde_json::to_string(&HostSettings::default()).unwrap();

    store.set(SettingsLayer::Host, &value).unwrap();

    let back = store.get(SettingsLayer::Host).unwrap().unwrap();
    let parsed: HostSettings = serde_json::from_str(&back).unwrap();
    assert_eq!(parsed.schema, HOST_SETTINGS_SCHEMA);

    // On disk the host owns the file, so it is TOML of the record, not an envelope.
    let on_disk = fs::read_to_string(store.path(SettingsLayer::Host)).unwrap();
    assert!(on_disk.contains("schema"), "disk was {on_disk}");
    assert!(!on_disk.contains("value ="), "disk was {on_disk}");
}

#[test]
fn a_corrupt_host_file_is_preserved() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());
    let path = store.path(SettingsLayer::Host);
    fs::write(&path, "= not host settings =").unwrap();

    let err = store.get(SettingsLayer::Host).unwrap_err();
    assert!(matches!(err, StoreError::Parse { .. }), "{err:?}");
    // The original is gone because it was moved aside, and something was kept.
    let kept: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        kept.iter()
            .any(|name| name.to_string_lossy().contains("host-settings")),
        "kept {kept:?}"
    );
}

#[test]
fn a_newer_host_schema_is_left_alone() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());
    let path = store.path(SettingsLayer::Host);
    fs::write(&path, "schema = 99\n").unwrap();

    let err = store.get(SettingsLayer::Host).unwrap_err();
    assert!(
        matches!(err, StoreError::UnknownVersion { found: 99, .. }),
        "{err:?}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "schema = 99\n");
}

#[test]
fn clearing_a_layer_that_was_never_set_is_fine() {
    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());

    assert!(store.clear(SettingsLayer::Ui).is_ok());
    store.set(SettingsLayer::Ui, UI_BLOB).unwrap();
    store.clear(SettingsLayer::Ui).unwrap();
    assert_eq!(store.get(SettingsLayer::Ui).unwrap(), None);
}

#[test]
fn a_store_that_refuses_to_write_still_answers() {
    let store = MemorySettingsStore::new();
    store.fail_writes(true);

    let failed = store.set(SettingsLayer::Ui, UI_BLOB);
    assert!(matches!(failed, Err(StoreError::NotDurable)));
    assert_eq!(store.get(SettingsLayer::Ui).unwrap(), None);
}

/// The connector fields are the host's, and a `SetSettings` must not carry them away.
///
/// The interface mirrors the whole host record and writes the whole of it back, so its copy of the
/// connections is as old as the dialog it was opened with. A flow that completed in between wrote
/// the real one. This is that race, and the toggle in the same blob proves the fix discards only
/// the three fields rather than the whole write.
#[test]
fn a_ui_write_keeps_its_own_fields_and_leaves_the_hosts_alone() {
    use ubiq_host::settings::Settings;
    use ubiq_proto::connectors::{AuthKind, Connection, ProviderId};
    use ubiq_proto::ids::ConnectionId;
    use ubiq_proto::settings::SettingsLayer;

    let settings = Settings::open(Box::new(MemorySettingsStore::new()));
    let id = ConnectionId::generate();

    // A flow completes: the host writes a connection nobody has told the interface about yet.
    settings
        .update_host(|host| {
            host.connections.push(Connection {
                id,
                provider: ProviderId::Gitlab,
                label: "work".into(),
                instance: Some("https://gitlab.example.com".into()),
                auth: AuthKind::Token,
                scopes: vec!["api".into()],
                account: "mdn".into(),
                client_id: None,
                oauth_app: None,
            });
        })
        .unwrap();
    assert!(
        settings.host().isolate_agents,
        "the toggle starts at its default, so flipping it below is a real change"
    );

    // The interface writes back the record it was holding: no connections, and a flipped toggle.
    let stale = HostSettings {
        isolate_agents: false,
        ..HostSettings::default()
    };
    let replies = settings.set(SettingsLayer::Host, serde_json::to_string(&stale).unwrap());
    assert!(replies.is_empty(), "a good blob answers nothing");

    let after = settings.host();
    assert_eq!(
        after.connections.len(),
        1,
        "the interface's stale copy took the connection with it"
    );
    assert_eq!(after.connections[0].id, id);
    assert!(
        !after.isolate_agents,
        "the toggle the interface actually owns was refused"
    );
}

/// The same rule for the provider records, and the reason it matters more there.
///
/// A provider's key is filed in the OS keychain under the record's id. A stale interface copy that
/// dropped a record would strand that key: unreachable, unlistable and still there. So this write
/// is discarded exactly as the connector fields' is.
#[test]
fn a_ui_write_cannot_drop_an_ai_provider_and_strand_its_key() {
    use ubiq_host::settings::Settings;
    use ubiq_proto::assist::{AiProvider, AiProviderKind, AssistProvider};
    use ubiq_proto::ids::AiProviderId;
    use ubiq_proto::settings::SettingsLayer;

    let settings = Settings::open(Box::new(MemorySettingsStore::new()));
    let id = AiProviderId::generate();

    // A provider is added: the host writes the record, having already filed the key.
    settings
        .update_host(|host| {
            host.ai_providers.push(AiProvider {
                id,
                kind: AiProviderKind::Anthropic,
                name: "work".into(),
                base_url: None,
                fast_model: "quick".into(),
                smart_model: None,
            });
            host.assist = AssistProvider::Api { provider_id: id };
        })
        .unwrap();

    // The interface writes back a record it opened before any of that existed.
    let stale = HostSettings {
        isolate_agents: false,
        ..HostSettings::default()
    };
    let replies = settings.set(SettingsLayer::Host, serde_json::to_string(&stale).unwrap());
    assert!(replies.is_empty(), "a good blob answers nothing");

    let after = settings.host();
    assert_eq!(
        after.ai_providers.len(),
        1,
        "the interface's stale copy took the provider — and its key — with it"
    );
    assert_eq!(after.ai_providers[0].id, id);
    assert!(
        !after.isolate_agents,
        "the toggle the interface actually owns was refused"
    );
    // The setting itself *is* the interface's to write, and reverting to the default is a real
    // write: which provider assistance uses is a preference, unlike the records behind it.
    assert_eq!(after.assist, AssistProvider::Off);
}

/// A record from a newer Ubiq is still refused, and the connector fields did not change that.
#[test]
fn a_newer_schema_is_still_refused() {
    use ubiq_host::settings::Settings;
    use ubiq_proto::settings::SettingsLayer;

    let settings = Settings::open(Box::new(MemorySettingsStore::new()));
    let ahead = format!(r#"{{"schema":{}}}"#, HOST_SETTINGS_SCHEMA + 1);
    let replies = settings.set(SettingsLayer::Host, ahead);
    assert_eq!(replies.len(), 1, "a blob from the future must be reported");
}

// ── The indexing level ──────────────────────────────────────────

/// A settings file written before the level existed still parses, and reads the default.
///
/// This is the whole migration: the documented rule is that a record from an older schema parses
/// because every field added since carries a default, and `IndexLevel`'s is `Light`. There is no
/// migration pass to get wrong.
#[test]
fn a_schema_four_file_parses_and_indexes_lightly() {
    use ubiq_proto::projects::IndexLevel;

    let dir = TempDir::new().unwrap();
    let store = FileSettingsStore::new(dir.path().to_path_buf());
    let path = store.path(SettingsLayer::Host);
    // Exactly what the build before this one wrote: no `index_level` key at all.
    fs::write(
        &path,
        "schema = 4\nisolate_agents = true\nsearch_excludes = [\"target\"]\n",
    )
    .unwrap();

    // The store parses the TOML on disk and hands back JSON, so the default has already been
    // filled in by the time anyone sees it.
    let blob = store.get(SettingsLayer::Host).unwrap().unwrap();
    let parsed: HostSettings = serde_json::from_str(&blob).unwrap();

    assert_eq!(parsed.index_level, IndexLevel::Light);
    assert_eq!(parsed.search_excludes, vec!["target".to_string()]);
}

/// The default is `Light`, and the levels are cumulative rather than alternative.
#[test]
fn the_levels_are_cumulative() {
    use ubiq_proto::projects::IndexLevel;

    assert_eq!(IndexLevel::default(), IndexLevel::Light);

    // `Full` keeps everything `Light` keeps. A symbol table answers about names and never about
    // content, so it can never stand in for the full-text half.
    assert!(!IndexLevel::None.keeps_text());
    assert!(!IndexLevel::None.keeps_symbols());
    assert!(IndexLevel::Light.keeps_text());
    assert!(!IndexLevel::Light.keeps_symbols());
    assert!(IndexLevel::Full.keeps_text());
    assert!(IndexLevel::Full.keeps_symbols());
}

/// The three states an update can carry stay distinct across the wire.
///
/// The regression this pins: `Option<Option<IndexLevel>>` would collapse "clear the override" and
/// "say nothing" into the same `None` after a serde round trip.
#[test]
fn clearing_an_override_is_not_the_same_as_saying_nothing() {
    use ubiq_proto::projects::{IndexChange, IndexLevel};

    let said_nothing: Option<IndexChange> = None;
    let clear = Some(IndexChange::Inherit);
    let pin = Some(IndexChange::Set(IndexLevel::Full));

    for value in [said_nothing, clear, pin] {
        let json = serde_json::to_string(&value).unwrap();
        let back: Option<IndexChange> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, value, "round trip lost {value:?} (as {json})");
    }

    assert_eq!(IndexChange::Inherit.resolve(), None);
    assert_eq!(
        IndexChange::Set(IndexLevel::Full).resolve(),
        Some(IndexLevel::Full)
    );
}
