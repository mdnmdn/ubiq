//! The host settings record: an older blob still parses, a new one round-trips, and a saved host
//! never carries a token.

use ubiq_proto::settings::{
    DronePreset, HOST_SETTINGS_SCHEMA, HostSettings, RemoteCarrier, RemoteScheme, SavedRemoteHost,
};

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
                carrier: RemoteCarrier::Socket,
            },
            SavedRemoteHost {
                id: String::new(),
                name: "build box".to_string(),
                address: "build.example.internal:7420".to_string(),
                scheme: RemoteScheme::Https,
                trust_insecure: true,
                carrier: RemoteCarrier::Socket,
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
///
/// `carrier` is held to the same rule and passes it for the same reason: an `Ssh` carrier names a
/// profile *id* and a folder, and the profile's own passphrase lives in the keychain under that id
/// (`D124`). A new field on this record is meant to fail this test until someone has looked at it.
#[test]
fn a_saved_host_has_no_field_a_token_could_land_in() {
    let record = SavedRemoteHost {
        id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
        name: "example".to_string(),
        address: "example.internal:7420".to_string(),
        scheme: RemoteScheme::Http,
        trust_insecure: false,
        carrier: RemoteCarrier::Socket,
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
        std::collections::BTreeSet::from([
            "id",
            "name",
            "address",
            "scheme",
            "trust_insecure",
            "carrier",
        ]),
        "a saved host must carry nothing beyond identity and connectivity \u{2014} in particular, no token"
    );
}

/// Schema seventeen's own `RemoteCarrier::Ssh` — written before `preset` existed — still parses,
/// and comes back as [`DronePreset::Attached`]: the one shape every record from before phase 7
/// already meant, not a guess.
#[test]
fn an_ssh_carrier_from_before_phase_seven_defaults_to_attached() {
    let json = r#"{"kind":"ssh","profile":"01ARZ3NDEKTSV4RRFFQ69G5FAV","root":"/srv/proj"}"#;
    let carrier: RemoteCarrier = serde_json::from_str(json).unwrap();
    match carrier {
        RemoteCarrier::Ssh { preset, root, .. } => {
            assert_eq!(preset, DronePreset::Attached);
            assert_eq!(root, "/srv/proj");
        }
        other => panic!("expected an ssh carrier, got {other:?}"),
    }
}

/// The preset round-trips for every arm — a saved `Session` or `Managed` host must come back
/// exactly as it was written, not silently downgraded to `Attached`.
#[test]
fn an_ssh_carriers_preset_round_trips() {
    for preset in [
        DronePreset::Attached,
        DronePreset::Session,
        DronePreset::Managed,
    ] {
        let carrier = RemoteCarrier::Ssh {
            profile: ubiq_proto::ids::SshProfileId::generate(),
            root: "/srv/proj".to_string(),
            preset,
            drone_path: None,
        };
        let json = serde_json::to_string(&carrier).unwrap();
        let read_back: RemoteCarrier = serde_json::from_str(&json).unwrap();
        assert_eq!(read_back, carrier);
    }
}

/// `drone_path` round-trips when set, and a record from before schema nineteen — which never
/// wrote the field at all — defaults it to `None` rather than refusing to parse.
#[test]
fn an_ssh_carriers_drone_path_round_trips_and_defaults() {
    let carrier = RemoteCarrier::Ssh {
        profile: ubiq_proto::ids::SshProfileId::generate(),
        root: "/srv/proj".to_string(),
        preset: DronePreset::Attached,
        drone_path: Some("/opt/ubiq/ubiq-drone".to_string()),
    };
    let json = serde_json::to_string(&carrier).unwrap();
    let read_back: RemoteCarrier = serde_json::from_str(&json).unwrap();
    assert_eq!(read_back, carrier);

    // Schema eighteen and earlier never wrote `drone_path` at all.
    let json = r#"{"kind":"ssh","profile":"01ARZ3NDEKTSV4RRFFQ69G5FAV","root":"/srv/proj","preset":"attached"}"#;
    let carrier: RemoteCarrier = serde_json::from_str(json).unwrap();
    match carrier {
        RemoteCarrier::Ssh { drone_path, .. } => assert_eq!(drone_path, None),
        other => panic!("expected an ssh carrier, got {other:?}"),
    }
}
