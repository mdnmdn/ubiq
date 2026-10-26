//! Reading a JSON Web Token: the header and payload as pretty JSON, and the three time claims as
//! dates. **No signature is verified** — there is no key here to verify it with, and the section
//! says so.

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::devtools::codec::b64_decode;
use crate::devtools::time::relative;

/// What a time claim says about the token right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimState {
    /// Nothing to judge (`iat`), or a bound that holds.
    Fine,
    /// `exp` is in the past.
    Expired,
    /// `nbf` is in the future.
    NotYet,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Claim {
    pub name: &'static str,
    /// What the claim means, in words.
    pub meaning: &'static str,
    /// The UTC date and how far it is from now.
    pub when: String,
    pub state: ClaimState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Jwt {
    pub header: String,
    pub payload: String,
    pub alg: Option<String>,
    pub claims: Vec<Claim>,
    /// Whether a third segment was present at all; an unsecured (`alg: none`) token has none.
    pub signed: bool,
}

impl Jwt {
    /// The one-line verdict: expired, not yet valid, or within its window.
    pub fn verdict(&self) -> (ClaimState, String) {
        if let Some(c) = self.claims.iter().find(|c| c.state == ClaimState::Expired) {
            return (ClaimState::Expired, format!("Expired — {}", c.when));
        }
        if let Some(c) = self.claims.iter().find(|c| c.state == ClaimState::NotYet) {
            return (ClaimState::NotYet, format!("Not valid yet — {}", c.when));
        }
        match self.claims.iter().find(|c| c.name == "exp") {
            Some(c) => (
                ClaimState::Fine,
                format!("Within its window — expires {}", c.when),
            ),
            None => (ClaimState::Fine, "No expiry claim".to_string()),
        }
    }
}

/// The token's segments, with a `Bearer ` prefix and surrounding whitespace dropped.
fn segments(token: &str) -> Option<Vec<&str>> {
    let token = token.trim();
    let token = token
        .strip_prefix("Bearer ")
        .or_else(|| token.strip_prefix("bearer "))
        .unwrap_or(token)
        .trim();
    let parts: Vec<&str> = token.split('.').collect();
    let url_safe = |s: &str| {
        s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'=')
    };
    (parts.len() == 3
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && parts.iter().all(|p| url_safe(p)))
    .then_some(parts)
}

fn json_segment(segment: &str, which: &str) -> Result<Value, String> {
    let bytes = b64_decode(segment).map_err(|e| format!("The {which} is not Base64url: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("The {which} is not JSON: {e}"))
}

/// The header, if `token` has a JWT's shape and its header is a JSON object naming an `alg`.
/// The detector's question — cheap, and false on anything else with two dots in it.
pub fn looks_like(token: &str) -> bool {
    segments(token)
        .and_then(|parts| json_segment(parts[0], "header").ok())
        .is_some_and(|header| header.get("alg").is_some())
}

pub fn decode(token: &str, now: DateTime<Utc>) -> Result<Jwt, String> {
    if token.trim().is_empty() {
        return Err("Paste a token".to_string());
    }
    let parts = segments(token).ok_or("A JWT is three Base64url segments separated by dots")?;
    let header = json_segment(parts[0], "header")?;
    let payload = json_segment(parts[1], "payload")?;
    let alg = header
        .get("alg")
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut claims = Vec::new();
    for (name, meaning) in [
        ("iat", "Issued at"),
        ("nbf", "Not before"),
        ("exp", "Expires"),
    ] {
        let Some(secs) = payload.get(name).and_then(Value::as_f64) else {
            continue;
        };
        let Some(at) = DateTime::from_timestamp(secs as i64, 0) else {
            continue;
        };
        let state = match name {
            "exp" if at <= now => ClaimState::Expired,
            "nbf" if at > now => ClaimState::NotYet,
            _ => ClaimState::Fine,
        };
        claims.push(Claim {
            name,
            meaning,
            when: format!(
                "{} ({})",
                at.format("%Y-%m-%d %H:%M:%S UTC"),
                relative(at, now)
            ),
            state,
        });
    }

    let pretty = |value: &Value| serde_json::to_string_pretty(value).unwrap_or_default();
    Ok(Jwt {
        header: pretty(&header),
        payload: pretty(&payload),
        alg,
        claims,
        signed: !parts[2].is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devtools::codec::{B64Alphabet, b64_encode};

    fn token(header: &str, payload: &str) -> String {
        format!(
            "{}.{}.c2ln",
            b64_encode(header.as_bytes(), B64Alphabet::UrlSafe),
            b64_encode(payload.as_bytes(), B64Alphabet::UrlSafe)
        )
    }

    #[test]
    fn decodes_header_payload_and_claims() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let jwt = token(
            r#"{"alg":"HS256","typ":"JWT"}"#,
            r#"{"sub":"42","iat":1699990000,"exp":1700003600}"#,
        );
        assert!(looks_like(&jwt));
        assert!(looks_like(&format!("Bearer {jwt}")));
        let decoded = decode(&jwt, now).unwrap();
        assert_eq!(decoded.alg.as_deref(), Some("HS256"));
        assert!(decoded.payload.contains("\"sub\": \"42\""));
        assert!(decoded.signed);
        assert_eq!(decoded.claims.len(), 2);
        let (state, text) = decoded.verdict();
        assert_eq!(state, ClaimState::Fine);
        assert!(text.contains("in 1 hour"), "{text}");
    }

    #[test]
    fn expired_and_not_yet() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let expired = decode(&token(r#"{"alg":"none"}"#, r#"{"exp":1699999000}"#), now).unwrap();
        assert_eq!(expired.verdict().0, ClaimState::Expired);
        let early = decode(&token(r#"{"alg":"none"}"#, r#"{"nbf":1700100000}"#), now).unwrap();
        assert_eq!(early.verdict().0, ClaimState::NotYet);
    }

    #[test]
    fn rejects_what_is_not_a_jwt() {
        assert!(!looks_like("a.b.c"));
        assert!(!looks_like("www.example.com"));
        assert!(!looks_like(&token(r#"{"typ":"JWT"}"#, "{}")));
        assert!(decode("a.b", Utc::now()).is_err());
    }
}
