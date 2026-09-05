//! Connections to external services: the URL a provider's instance is reached at, the table behind
//! it, and the one thing in the family that is a security boundary rather than a convenience.

use rustls::client::danger::ServerCertVerifier;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use ubiq_host::connectors::providers::{self, api_url};
use ubiq_host::connectors::{store, tls};
use ubiq_proto::connectors::{ConnectError, ProviderId};
use ubiq_proto::messages::LoginStatus;

/// A leaf nothing on any machine trusts, generated once and committed:
///
/// ```text
/// openssl req -x509 -newkey rsa:2048 -nodes -keyout /dev/null -outform der \
///   -out crates/ubiq-host/tests/data/self-signed.der -days 3650 -subj "/CN=ubiq.test"
/// ```
const SELF_SIGNED: &[u8] = include_bytes!("data/self-signed.der");

#[test]
fn an_instance_url_is_joined_not_rebuilt() {
    // An on-premises install under a path keeps it — which is the whole reason the instance is
    // stored as typed rather than reconstructed from a host name.
    assert_eq!(
        api_url(
            ProviderId::Gitlab,
            Some("https://example.com/gitlab"),
            "/user"
        )
        .unwrap(),
        "https://example.com/gitlab/api/v4/user"
    );
    // One trailing slash is the same address, not a different one.
    assert_eq!(
        api_url(
            ProviderId::Gitlab,
            Some("https://example.com/gitlab/"),
            "/user"
        )
        .unwrap(),
        "https://example.com/gitlab/api/v4/user"
    );
    // The cloud host already has its base baked in; a self-hosted one does not.
    assert_eq!(
        api_url(ProviderId::Github, None, "/user").unwrap(),
        "https://api.github.com/user"
    );
    assert_eq!(
        api_url(ProviderId::Github, Some("https://ghe.corp"), "/user").unwrap(),
        "https://ghe.corp/api/v3/user"
    );
}

#[test]
fn a_bare_host_name_is_not_an_instance() {
    // There is no scheme to assume, so this is refused where the user typed it rather than
    // guessed at and failed later.
    assert_eq!(
        api_url(ProviderId::Github, Some("ghe.corp"), "/user"),
        Err(ConnectError::BadInstance)
    );
}

#[test]
fn every_provider_row_is_complete() {
    // A half-landed edit to the table is what this catches: a row with no whoami cannot identify
    // an account, and a row with no account keys cannot read the answer.
    for &provider in ProviderId::all() {
        let row = providers::of(provider);
        assert!(!row.whoami.is_empty(), "{provider:?} names no whoami path");
        assert!(
            !row.account_keys.is_empty(),
            "{provider:?} names no account key"
        );
        assert!(!row.secret_prompt.is_empty(), "{provider:?} has no prompt");
    }
}

#[test]
fn an_account_name_is_the_first_key_the_body_has() {
    // Gitea answers `login`; a body with only `username` still resolves, because the row names
    // both in order.
    let login = serde_json::json!({ "login": "ada", "username": "ada-l" });
    assert_eq!(
        providers::account_name(ProviderId::Gitea, &login).as_deref(),
        Some("ada")
    );
    let username = serde_json::json!({ "username": "ada-l" });
    assert_eq!(
        providers::account_name(ProviderId::Gitea, &username).as_deref(),
        Some("ada-l")
    );
    // A body that answered something else is not this product — which is how a Gitea URL typed
    // into a GitLab connection fails at connect time rather than at first use.
    let elsewhere = serde_json::json!({ "displayName": "Ada" });
    assert_eq!(
        providers::account_name(ProviderId::Gitlab, &elsewhere),
        None
    );
}

#[test]
fn a_pin_accepts_exactly_one_certificate_and_describes_the_rest() {
    let der = CertificateDer::from(SELF_SIGNED);
    let name = ServerName::try_from("ubiq.test").unwrap();
    let sha256 = tls::fingerprint(SELF_SIGNED);

    // The pin the user vouched for: the machine still refuses the chain, and the fingerprint is
    // what lets it through.
    let seen = tls::seen();
    let verifier = tls::PinVerifier::new(Some(sha256.clone()), seen.clone());
    assert!(
        verifier
            .verify_server_cert(&der, &[], &name, &[], UnixTime::now())
            .is_ok()
    );

    // One hex digit out is a different certificate, and the answer is the same as for no pin at
    // all — plus a description of what was actually offered.
    let mut wrong = sha256.clone();
    wrong.replace_range(0..1, if sha256.starts_with('a') { "b" } else { "a" });
    let seen = tls::seen();
    let verifier = tls::PinVerifier::new(Some(wrong), seen.clone());
    assert!(
        verifier
            .verify_server_cert(&der, &[], &name, &[], UnixTime::now())
            .is_err()
    );
    let offered = tls::lock(&seen).clone().expect("the leaf was described");
    assert_eq!(offered.sha256, sha256);
    assert!(offered.self_signed, "a self-signed leaf says so");

    // No pin: refused, and described, so the interface has something to ask about.
    let seen = tls::seen();
    let verifier = tls::PinVerifier::new(None, seen.clone());
    assert!(
        verifier
            .verify_server_cert(&der, &[], &name, &[], UnixTime::now())
            .is_err()
    );
    assert_eq!(
        tls::lock(&seen).clone().map(|info| info.sha256).as_deref(),
        Some(sha256.as_str())
    );
}

#[test]
fn a_token_says_what_it_can_about_its_own_validity() {
    let now = 1_700_000_000_000;
    // An expiry in the future, in the seconds the OAuth answers use.
    let valid = vec![br#"{"access_token":"x","expires_at":1700000600}"#.to_vec()];
    assert_eq!(
        store::Store::status_of(&valid, now),
        LoginStatus::Valid {
            expires_at_ms: 1_700_000_600_000
        }
    );
    let expired = vec![br#"{"access_token":"x","expires_at":1699999000}"#.to_vec()];
    assert_eq!(
        store::Store::status_of(&expired, now),
        LoginStatus::Expired {
            expires_at_ms: 1_699_999_000_000
        }
    );
    // A personal access token names no expiry, and usually works anyway.
    let unknown = vec![br#"{"access_token":"x"}"#.to_vec()];
    assert_eq!(store::Store::status_of(&unknown, now), LoginStatus::Unknown);
    // Nothing stored at all.
    assert_eq!(store::Store::status_of(&[], now), LoginStatus::Missing);
}
