//! Certificate verification, with one pin.
//!
//! The order matters and is the whole design: the machine's own rules are asked first, so a
//! company's internal CA in the platform trust store works without anyone being asked to vouch for
//! anything. Only a chain those rules refuse reaches the pin, and only an exact SHA-256 match on
//! the leaf's DER passes there.
//!
//! A pin says *which certificate*, never that a bad signature is acceptable — so the three
//! signature methods delegate to the real verifier unchanged. There is no "accept anything" mode
//! in this file, and there is no path that reaches `ServerCertVerified::assertion()` without
//! either the machine or a fingerprint the user typed back agreeing first.
//!
//! What a refusal leaves behind is a [`CertInfo`] in a shared slot, so the request that failed can
//! say *which* certificate it was refusing rather than only that it refused.

use std::sync::{Arc, Mutex, OnceLock};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};
use ubiq_proto::connectors::{CertInfo, CertReason};

/// Where a refused leaf is left for the request that failed to pick up.
pub type Seen = Arc<Mutex<Option<CertInfo>>>;

/// An empty slot for one request.
pub fn seen() -> Seen {
    Arc::new(Mutex::new(None))
}

/// The crypto provider, named rather than inherited.
///
/// `CryptoProvider::get_default_or_install_from_crate_features()` panics when two providers are
/// compiled in and none was installed — which is a runtime panic caused by a *different* crate's
/// feature flags. Naming `ring` here means nothing elsewhere in the workspace can turn a
/// dependency edge into a crash at the first HTTPS request.
pub fn provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// The trust anchors: the platform's store first, the compiled-in public roots as the floor.
///
/// Both, not either. The platform store is what makes an internal CA work; the floor is what makes
/// a machine whose store cannot be read still able to reach a public provider.
pub fn roots() -> Arc<RootCertStore> {
    static ROOTS: OnceLock<Arc<RootCertStore>> = OnceLock::new();
    ROOTS
        .get_or_init(|| {
            let mut store = RootCertStore::empty();
            let native = rustls_native_certs::load_native_certs();
            for error in &native.errors {
                tracing::debug!("a platform trust root was not read: {error}");
            }
            store.add_parsable_certificates(native.certs);
            store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            Arc::new(store)
        })
        .clone()
}

/// The machine's rules, then one fingerprint.
#[derive(Debug)]
pub struct PinVerifier {
    inner: Arc<WebPkiServerVerifier>,
    /// The fingerprint the user has already vouched for at this origin, if any.
    pin: Option<String>,
    seen: Seen,
}

impl PinVerifier {
    pub fn new(pin: Option<String>, seen: Seen) -> Arc<Self> {
        let inner = WebPkiServerVerifier::builder_with_provider(roots(), provider())
            .build()
            // The only failure is an empty root store, and `roots()` always carries the compiled-in
            // floor — so this cannot fire without the build itself being broken.
            .expect("the web pki verifier");
        Arc::new(Self { inner, pin, seen })
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
        let refusal = match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) => return Ok(verified),
            Err(refusal) => refusal,
        };
        let sha256 = fingerprint(end_entity);
        if self.pin.as_deref() == Some(sha256.as_str()) {
            return Ok(ServerCertVerified::assertion());
        }
        *lock(&self.seen) = Some(describe(end_entity, &refusal));
        // The original error, not a summary: the caller decides whether a certificate the user
        // could vouch for outranks it, and it cannot decide that from a rewritten message.
        Err(refusal)
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

/// The SHA-256 of a certificate's DER, lowercase and unseparated — the exact string a
/// `TrustCertificate` carries back and a `TrustedCert` stores.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A leaf as the user is shown it. Everything here is public — a certificate is what a server
/// hands to anyone who connects.
///
/// A certificate this parser cannot read still produces a row, with the fingerprint filled in and
/// the names empty: the fingerprint is the part the user compares against what their administrator
/// told them, and it is computed from the bytes rather than from the parse.
pub fn describe(der: &CertificateDer<'_>, refusal: &rustls::Error) -> CertInfo {
    let sha256 = fingerprint(der);
    let mut info = CertInfo {
        subject: String::new(),
        sans: Vec::new(),
        issuer: String::new(),
        not_before: 0,
        not_after: 0,
        sha256,
        self_signed: false,
        reason: reason(refusal),
    };
    let Ok((_, cert)) = x509_parser::parse_x509_certificate(der) else {
        return info;
    };
    info.subject = cert.subject().to_string();
    info.issuer = cert.issuer().to_string();
    info.self_signed = info.subject == info.issuer;
    info.not_before = cert.validity().not_before.timestamp();
    info.not_after = cert.validity().not_after.timestamp();
    if let Ok(Some(san)) = cert.subject_alternative_name() {
        info.sans = san
            .value
            .general_names
            .iter()
            .filter_map(|name| match name {
                x509_parser::extensions::GeneralName::DNSName(dns) => Some((*dns).to_string()),
                x509_parser::extensions::GeneralName::IPAddress(bytes) => Some(ip(bytes)),
                _ => None,
            })
            .collect();
    }
    info
}

/// Why the chain did not check out, in the four words a dialog can put in a sentence.
///
// ponytail: deliberately coarser than rustls' `CertificateError` set — everything not named here
// reads as `UnknownIssuer`, which is the honest default for "this did not check out". Widen it
// when a dialog sentence turns out to be wrong, not because the error enum has more variants.
fn reason(refusal: &rustls::Error) -> CertReason {
    use rustls::CertificateError as Cert;
    let rustls::Error::InvalidCertificate(cert) = refusal else {
        return CertReason::UnknownIssuer;
    };
    match cert {
        Cert::Expired | Cert::ExpiredContext { .. } => CertReason::Expired,
        Cert::NotValidYet | Cert::NotValidYetContext { .. } => CertReason::NotYetValid,
        Cert::NotValidForName | Cert::NotValidForNameContext { .. } => CertReason::HostnameMismatch,
        _ => CertReason::UnknownIssuer,
    }
}

/// A SAN IP address as it is written down. Four bytes is v4, sixteen is v6; anything else is a
/// certificate doing something this display does not need to understand.
fn ip(bytes: &[u8]) -> String {
    match bytes.len() {
        4 => std::net::Ipv4Addr::from([bytes[0], bytes[1], bytes[2], bytes[3]]).to_string(),
        16 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(bytes);
            std::net::Ipv6Addr::from(octets).to_string()
        }
        _ => String::new(),
    }
}

/// A poisoned slot holds no invariant — the panic that poisoned it left a `CertInfo` or a `None`,
/// and either is fine to look at.
pub fn lock<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|poison| poison.into_inner())
}
