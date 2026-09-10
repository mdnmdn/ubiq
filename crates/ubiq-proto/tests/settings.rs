//! The host settings record: an older blob still parses, a new one round-trips, and a saved host
//! never carries a token.

use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings, RemoteScheme, SavedRemoteHost};

#[test]
fn a_settings_record_written_before_remote_hosts_still_reads() {
    // A schema-6 blob: every field added since defaults, which is what lets an older record parse.
    let json = r#"{
        "schema": 6,
        "isolate_agents": false,
        "search_excludes": ["target"],
        "search_fallbacks": ["grep"]
    }"#;

    let settings = serde_json::from_str::<HostSettings>(json).unwrap();
    assert_eq!(settings.schema, 6);
    assert_eq!(settings.remote_hosts, Vec::new());
}

#[test]
fn a_record_with_saved_hosts_round_trips() {
    let written = HostSettings {
        remote_hosts: vec![
            SavedRemoteHost {
                id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
                name: "office desktop".to_string(),
                address: "10.0.0.4:7420".to_string(),
                scheme: RemoteScheme::Http,
                trust_insecure: false,
            },
            SavedRemoteHost {
                id: String::new(),
                name: "build box".to_string(),
                address: "build.example.internal:7420".to_string(),
                scheme: RemoteScheme::Https,
                trust_insecure: true,
            },
        ],
        ..HostSettings::default()
    };
    assert_eq!(written.schema, HOST_SETTINGS_SCHEMA);

    let json = serde_json::to_string(&written).unwrap();
    let read_back: HostSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(read_back, written);
}

/// The whole reason `remote_hosts` reads connectivity only: a token typed into the settings
/// panel must never be part of what this record can carry, so there is no field to serialise it
/// into even by mistake. The id, scheme and trust flag ride along — none of them is a secret.
#[test]
fn a_saved_host_has_no_field_a_token_could_land_in() {
    let record = SavedRemoteHost {
        id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        name: "example".to_string(),
        address: "example.internal:7420".to_string(),
        scheme: RemoteScheme::Http,
        trust_insecure: false,
    };
    let json = serde_json::to_value(&record).unwrap();
    let object = json
        .as_object()
        .expect("a saved host serialises as an object");
    assert_eq!(
        object
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["id", "name", "address", "scheme", "trust_insecure"]),
        "a saved host must carry nothing beyond identity and connectivity \u{2014} in particular, no token"
    );
}
