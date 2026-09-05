//! Material on the bus: the one shape it takes, and the guarantee that shape carries.
//!
//! The contract's rule is not "these variants are blessed" but "material crosses only in a
//! `Secret`, and a `Secret` is never printed". These tests are what make that a guarantee rather
//! than a comment — the whole `Message` derives `Debug`, and both halves log messages.

use ubiq_proto::connectors::ProviderId;
use ubiq_proto::ids::ConnectId;
use ubiq_proto::messages::{Message, Secret};

const MATERIAL: &str = "ghp_a_real_looking_token";

#[test]
fn a_secret_never_prints_itself() {
    let secret = Secret::new(MATERIAL);
    assert_eq!(format!("{secret:?}"), "Secret(***)");
    assert!(!format!("{secret:?}").contains(MATERIAL));
}

#[test]
fn a_message_carrying_material_does_not_print_it() {
    // The path that matters: something logs a whole message, as both halves do.
    let message = Message::SubmitConnectSecret {
        connect_id: ConnectId::generate(),
        secret: Secret::new(MATERIAL),
    };
    let printed = format!("{message:?}");
    assert!(
        !printed.contains(MATERIAL),
        "material reached a Debug: {printed}"
    );
    assert!(printed.contains("Secret(***)"));

    let app = Message::SetAppSecret {
        provider: ProviderId::Gitlab,
        origin: Some("https://gitlab.example.com".into()),
        secret: Secret::new(MATERIAL),
    };
    let printed = format!("{app:?}");
    assert!(
        !printed.contains(MATERIAL),
        "material reached a Debug: {printed}"
    );
    // The non-secret half of the payload still reads, so a log line stays useful.
    assert!(printed.contains("gitlab.example.com"));
}

#[test]
fn a_secret_still_reaches_the_host_intact() {
    // Redacting the wire as well as the log would leave the host nothing to store.
    let json = serde_json::to_string(&Secret::new(MATERIAL)).unwrap();
    assert_eq!(json, format!("\"{MATERIAL}\""));
    let back: Secret = serde_json::from_str(&json).unwrap();
    assert_eq!(back.expose(), MATERIAL);
}
