//! Trust-on-first-use key pinning for a TLS client: the socket remote's dialer (`D228`).
//!
//! Three ways to trust a server, one verifier:
//!
//! - [`Trust::Chain`] — the platform's roots and the name check, unchanged. A host a CA vouches
//!   for is never pinned: the CA is the trust decision, and a renewal on a new key is its business.
//! - [`Trust::FirstUse`] — any certificate, and the key it carries is left in [`Seen`] for the
//!   caller to record. Taken only when the user ticked "trust certificate" and no key is pinned yet.
//! - [`Trust::Pinned`] — only the key the pin names. The chain, the name and the validity dates
//!   are not consulted: a pinned host is identified by its key and nothing else.
//!
//! **Every mode checks the handshake signature** against the presented certificate, so the key a
//! pin names is a key the peer holds — a man in the middle replaying the real certificate fails
//! here, not at the pin.
//!
//! **The pin** is the lowercase hex SHA-256 of the leaf's whole DER `SubjectPublicKeyInfo` (the
//! HPKP / `openssl pkey -pubin -outform der | sha256sum` form), so a certificate renewed on the
//! same key keeps its pin.

use std::sync::{Arc, Mutex};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest as _, Sha256};

/// How a dial decides to believe the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// The platform's roots.
    Chain,
    /// Any key, recorded.
    FirstUse,
    /// Exactly this key: a [`fingerprint`].
    Pinned(String),
}

impl Trust {
    /// The rule for a saved host: its pin if it has one, first use when it trusts any certificate,
    /// the platform's roots otherwise.
    pub fn for_host(pin: Option<&str>, trust_insecure: bool) -> Self {
        match pin {
            Some(pin) => Trust::Pinned(pin.to_string()),
            None if trust_insecure => Trust::FirstUse,
            None => Trust::Chain,
        }
    }
}

/// The fingerprint of the key the server presented, filled in by the handshake under
/// [`Trust::FirstUse`] and [`Trust::Pinned`] — accepted or not. Empty under [`Trust::Chain`].
pub type Seen = Arc<Mutex<Option<String>>>;

/// The pin of a certificate's key: lowercase hex SHA-256 of its DER `SubjectPublicKeyInfo`.
pub fn fingerprint(cert: &CertificateDer<'_>) -> Result<String, rustls::Error> {
    let parsed = ParsedCertificate::try_from(cert)?;
    Ok(Sha256::digest(parsed.subject_public_key_info().as_ref())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// A client config for one dial under `trust`, and the slot its handshake fills.
pub fn client_config(trust: Trust) -> (Arc<rustls::ClientConfig>, Seen) {
    let seen = Seen::default();
    let verifier = Arc::new(PinVerifier::new(trust, seen.clone()));
    // Infallible with `ring` compiled in: the provider names its own cipher suites.
    let config = rustls::ClientConfig::builder_with_provider(crate::tls::provider())
        .with_safe_default_protocol_versions()
        .expect("the TLS protocol versions")
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    (Arc::new(config), seen)
}

/// See the module comment.
#[derive(Debug)]
pub struct PinVerifier {
    /// The platform's rules: the whole check under [`Trust::Chain`], the signature check always.
    inner: Arc<WebPkiServerVerifier>,
    trust: Trust,
    seen: Seen,
}

impl PinVerifier {
    pub fn new(trust: Trust, seen: Seen) -> Self {
        let inner = WebPkiServerVerifier::builder_with_provider(
            crate::tls::roots(),
            crate::tls::provider(),
        )
        .build()
        // The only failure is an empty root store, and the roots carry the compiled-in floor.
        .expect("the web pki verifier");
        Self { inner, trust, seen }
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let pin = match &self.trust {
            Trust::Chain => {
                return self.inner.verify_server_cert(
                    end_entity,
                    intermediates,
                    server_name,
                    ocsp_response,
                    now,
                );
            }
            Trust::FirstUse => None,
            Trust::Pinned(pin) => Some(pin),
        };
        let presented = fingerprint(end_entity)?;
        let matches = pin.is_none_or(|pin| pin.eq_ignore_ascii_case(&presented));
        *self.seen.lock().unwrap_or_else(|p| p.into_inner()) = Some(presented);
        if matches {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::ApplicationVerificationFailure,
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELF_SIGNED: &[u8] = include_bytes!("../tests/data/self-signed.der");

    fn verify(trust: Trust) -> (Result<ServerCertVerified, rustls::Error>, Option<String>) {
        let seen = Seen::default();
        let verifier = PinVerifier::new(trust, seen.clone());
        let name = ServerName::try_from("example.invalid").unwrap();
        let result = verifier.verify_server_cert(
            &CertificateDer::from(SELF_SIGNED),
            &[],
            &name,
            &[],
            UnixTime::now(),
        );
        let seen = seen.lock().unwrap().clone();
        (result, seen)
    }

    #[test]
    fn the_pin_is_the_sha256_of_the_spki() {
        let pin = fingerprint(&CertificateDer::from(SELF_SIGNED)).unwrap();
        assert_eq!(pin.len(), 64);
        assert!(
            pin.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
        // Not the certificate's own hash: a renewal on the same key keeps the pin.
        let der: String = Sha256::digest(SELF_SIGNED)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_ne!(pin, der);
    }

    #[test]
    fn first_use_accepts_and_records_the_key() {
        let pin = fingerprint(&CertificateDer::from(SELF_SIGNED)).unwrap();
        let (result, seen) = verify(Trust::FirstUse);
        assert!(result.is_ok());
        assert_eq!(seen, Some(pin));
    }

    #[test]
    fn a_pin_accepts_only_its_key() {
        let pin = fingerprint(&CertificateDer::from(SELF_SIGNED)).unwrap();
        let (result, _) = verify(Trust::Pinned(pin.clone()));
        assert!(result.is_ok(), "the pinned key passes with no chain");

        let (result, seen) = verify(Trust::Pinned("00".repeat(32)));
        assert!(result.is_err(), "another key is refused");
        assert_eq!(seen, Some(pin), "and the refused key is reported");
    }

    #[test]
    fn the_chain_refuses_a_self_signed_host_and_records_nothing() {
        let (result, seen) = verify(Trust::Chain);
        assert!(result.is_err());
        assert_eq!(seen, None);
    }

    #[test]
    fn a_saved_host_picks_its_rule() {
        assert_eq!(Trust::for_host(None, false), Trust::Chain);
        assert_eq!(Trust::for_host(None, true), Trust::FirstUse);
        assert_eq!(
            Trust::for_host(Some("ab"), false),
            Trust::Pinned("ab".into())
        );
        assert_eq!(
            Trust::for_host(Some("ab"), true),
            Trust::Pinned("ab".into())
        );
    }
}
