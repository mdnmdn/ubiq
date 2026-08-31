//! The file family's payloads on the wire.
//!
//! Both tests guard a property somebody would otherwise simplify away: that a failure crosses as a
//! kind the interface can branch on rather than as prose, and that contents cross as bytes.

use ubiq_proto::files::{DirEntry, DirListing, EntryKind, FileContents, FileError, FileVersion};

#[test]
fn a_file_error_round_trips_by_kind() {
    let cases = [
        FileError::Refused("outside the project".to_string()),
        FileError::Missing,
        FileError::WrongKind,
        FileError::Denied("Permission denied (os error 13)".to_string()),
        FileError::Conflict,
        FileError::Failed("Too many open files".to_string()),
    ];

    for error in cases {
        let json = serde_json::to_string(&error).unwrap();
        assert_eq!(serde_json::from_str::<FileError>(&json).unwrap(), error);
    }

    // The tag is what the interface branches on, so its shape is part of the contract.
    let json = serde_json::to_string(&FileError::Missing).unwrap();
    assert_eq!(json, r#"{"kind":"Missing"}"#);
    let json = serde_json::to_string(&FileError::Denied("no".to_string())).unwrap();
    assert_eq!(json, r#"{"kind":"Denied","reason":"no"}"#);
}

#[test]
fn contents_cross_as_bytes() {
    // A prefix cut mid-sequence is exactly what a truncated read produces, and it is why this is
    // not a `String`. Anything that decoded here would have to lose these bytes or refuse them.
    let contents = FileContents {
        bytes: vec![0xff, 0xfe, b'h', b'i', 0xe2, 0x82],
        len: 4096,
        truncated: true,
        is_binary: false,
        version: None,
    };

    let json = serde_json::to_string(&contents).unwrap();
    let back: FileContents = serde_json::from_str(&json).unwrap();
    assert_eq!(back, contents);
    assert_eq!(back.bytes, vec![0xff, 0xfe, b'h', b'i', 0xe2, 0x82]);
}

#[test]
fn a_listing_round_trips_with_its_entries() {
    let listing = DirListing {
        rel_path: "crates/ubiq-host".to_string(),
        entries: vec![
            DirEntry {
                name: "src".to_string(),
                rel_path: "crates/ubiq-host/src".to_string(),
                kind: EntryKind::Dir,
                size: None,
                symlink: false,
            },
            DirEntry {
                name: "Cargo.toml".to_string(),
                rel_path: "crates/ubiq-host/Cargo.toml".to_string(),
                kind: EntryKind::File,
                size: Some(612),
                symlink: false,
            },
        ],
        truncated: false,
    };

    let json = serde_json::to_string(&listing).unwrap();
    assert_eq!(serde_json::from_str::<DirListing>(&json).unwrap(), listing);
}

#[test]
fn a_version_with_no_modification_time_is_absent_rather_than_null() {
    let version = FileVersion {
        len: 12,
        modified: None,
    };
    let json = serde_json::to_string(&version).unwrap();
    assert_eq!(json, r#"{"len":12}"#);
    assert_eq!(serde_json::from_str::<FileVersion>(&json).unwrap(), version);
}
