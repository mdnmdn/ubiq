//! UUIDs: generating them in the four versions worth having, writing them in the shapes people
//! paste, and reading one back to say what it is.

use ::uuid::{Uuid, Variant};
use chrono::DateTime;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UuidVersion {
    /// Time and node — the node here is random, never the machine's address.
    V1,
    /// Random.
    #[default]
    V4,
    /// SHA-1 of a namespace and a name: the same pair always gives the same UUID.
    V5,
    /// Unix-millisecond time-ordered, the one to sort by.
    V7,
}

impl UuidVersion {
    pub const ALL: [UuidVersion; 4] = [Self::V4, Self::V7, Self::V1, Self::V5];

    pub fn label(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V4 => "v4",
            Self::V5 => "v5",
            Self::V7 => "v7",
        }
    }
}

/// The four namespaces RFC 9562 defines for a v5.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Namespace {
    #[default]
    Dns,
    Url,
    Oid,
    X500,
}

impl Namespace {
    pub const ALL: [Namespace; 4] = [Self::Dns, Self::Url, Self::Oid, Self::X500];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dns => "DNS",
            Self::Url => "URL",
            Self::Oid => "OID",
            Self::X500 => "X.500",
        }
    }

    fn uuid(self) -> Uuid {
        match self {
            Self::Dns => Uuid::NAMESPACE_DNS,
            Self::Url => Uuid::NAMESPACE_URL,
            Self::Oid => Uuid::NAMESPACE_OID,
            Self::X500 => Uuid::NAMESPACE_X500,
        }
    }
}

/// How a UUID is written out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UuidFormat {
    pub upper: bool,
    pub hyphens: bool,
}

impl Default for UuidFormat {
    fn default() -> Self {
        Self {
            upper: false,
            hyphens: true,
        }
    }
}

pub fn format(uuid: &Uuid, fmt: UuidFormat) -> String {
    let text = if fmt.hyphens {
        uuid.hyphenated().to_string()
    } else {
        uuid.simple().to_string()
    };
    if fmt.upper { text.to_uppercase() } else { text }
}

/// `count` UUIDs of `version`. A v5 is a function of its inputs, so it is always exactly one.
pub fn generate(version: UuidVersion, count: usize, namespace: Namespace, name: &str) -> Vec<Uuid> {
    if version == UuidVersion::V5 {
        return vec![Uuid::new_v5(&namespace.uuid(), name.as_bytes())];
    }
    // A v1's node is random and marked so (the multicast bit), rather than a hardware address.
    let mut node = [0u8; 6];
    node.copy_from_slice(&Uuid::new_v4().as_bytes()[..6]);
    node[0] |= 0x01;
    (0..count.max(1))
        .map(|_| match version {
            UuidVersion::V1 => Uuid::now_v1(&node),
            UuidVersion::V7 => Uuid::now_v7(),
            _ => Uuid::new_v4(),
        })
        .collect()
}

/// What a pasted UUID is: its version, variant, embedded time where it has one, and its shapes.
pub fn inspect(text: &str) -> Result<Vec<(&'static str, String)>, String> {
    let text = text.trim();
    let uuid = Uuid::parse_str(text).map_err(|e| format!("Not a UUID: {e}"))?;
    let mut rows = vec![
        (
            "Version",
            match uuid.get_version_num() {
                0 if uuid.is_nil() => "nil".to_string(),
                n => format!("v{n}"),
            },
        ),
        (
            "Variant",
            match uuid.get_variant() {
                Variant::NCS => "NCS (reserved)",
                Variant::RFC4122 => "RFC 9562",
                Variant::Microsoft => "Microsoft (reserved)",
                _ => "future (reserved)",
            }
            .to_string(),
        ),
    ];
    if let Some(ts) = uuid.get_timestamp() {
        let (secs, nanos) = ts.to_unix();
        if let Some(at) = DateTime::from_timestamp(secs as i64, nanos) {
            rows.push(("Time", at.to_rfc3339()));
        }
    }
    rows.push(("Hyphenated", uuid.hyphenated().to_string()));
    rows.push(("Simple", uuid.simple().to_string()));
    rows.push(("Braced", uuid.braced().to_string()));
    rows.push(("URN", uuid.urn().to_string()));
    Ok(rows)
}

/// Whether `text` is a UUID written the way people write one — hyphenated, braced or as a URN.
/// Thirty-two bare hex digits are left to the digest detector.
pub fn looks_like(text: &str) -> bool {
    let text = text.trim();
    text.len() != 32 && Uuid::parse_str(text).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let id = Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8").unwrap();
        assert_eq!(
            format(&id, UuidFormat::default()),
            "67e55044-10b1-426f-9247-bb680e5fe0c8"
        );
        assert_eq!(
            format(
                &id,
                UuidFormat {
                    upper: true,
                    hyphens: false
                }
            ),
            "67E5504410B1426F9247BB680E5FE0C8"
        );
    }

    #[test]
    fn generates_each_version() {
        for version in UuidVersion::ALL {
            let ids = generate(version, 3, Namespace::Dns, "example.com");
            let expected = if version == UuidVersion::V5 { 1 } else { 3 };
            assert_eq!(ids.len(), expected, "{version:?}");
            let num = match version {
                UuidVersion::V1 => 1,
                UuidVersion::V4 => 4,
                UuidVersion::V5 => 5,
                UuidVersion::V7 => 7,
            };
            assert!(ids.iter().all(|id| id.get_version_num() == num));
        }
        // RFC 9562's own v5 example.
        assert_eq!(
            generate(UuidVersion::V5, 1, Namespace::Dns, "www.example.com")[0].to_string(),
            "2ed6657d-e927-568b-95e1-2665a8aea6a2"
        );
    }

    #[test]
    fn inspects_and_detects() {
        let v7 = generate(UuidVersion::V7, 1, Namespace::Dns, "")[0];
        let rows = inspect(&v7.to_string()).unwrap();
        assert_eq!(rows[0].1, "v7");
        assert!(rows.iter().any(|(name, _)| *name == "Time"));
        assert!(looks_like(&v7.to_string()));
        assert!(looks_like(&v7.braced().to_string()));
        assert!(!looks_like(&v7.simple().to_string()));
        assert!(inspect("nope").is_err());
    }
}
