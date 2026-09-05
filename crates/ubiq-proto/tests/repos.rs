//! The repository family's payloads, and the one function that decides what a repository URL is.
//!
//! `parse_repo_url` is the whole reason this file is long: it is the single place URL sniffing
//! lives, so both its acceptances and its refusals are contract rather than detail. The two
//! round-trip tests guard the rest — that a clone request crosses whole, and that a settings blob
//! written before the two roots existed still reads.

use ubiq_proto::ids::CloneId;
use ubiq_proto::messages::Message;
use ubiq_proto::repos::{CloneRequest, RepoSource, parse_repo_url};
use ubiq_proto::settings::{HOST_SETTINGS_SCHEMA, HostSettings};

#[test]
fn a_repository_url_is_taken_apart_and_normalised() {
    // (text, host, owner, name)
    let cases = [
        (
            "https://github.com/rust-lang/rust",
            "github.com",
            "rust-lang",
            "rust",
        ),
        (
            "https://github.com/rust-lang/rust.git",
            "github.com",
            "rust-lang",
            "rust",
        ),
        (
            "https://github.com/rust-lang/rust/",
            "github.com",
            "rust-lang",
            "rust",
        ),
        (
            "  https://github.com/rust-lang/rust  ",
            "github.com",
            "rust-lang",
            "rust",
        ),
        // Uppercase in a host is still the same host.
        (
            "https://GitHub.com/rust-lang/rust",
            "github.com",
            "rust-lang",
            "rust",
        ),
        // GitLab subgroups: everything but the last segment is the owner.
        (
            "https://gitlab.com/group/sub/name",
            "gitlab.com",
            "group/sub",
            "name",
        ),
        (
            "https://gitlab.com/group/sub/deeper/name.git",
            "gitlab.com",
            "group/sub/deeper",
            "name",
        ),
        // An unknown host is a repository only when the path says so.
        (
            "https://git.example.com/team/thing.git",
            "git.example.com",
            "team",
            "thing",
        ),
    ];

    for (text, host, owner, name) in cases {
        let parsed = parse_repo_url(text).unwrap_or_else(|| panic!("{text} is a repository"));
        assert_eq!(parsed.host, host, "{text}");
        assert_eq!(parsed.owner, owner, "{text}");
        assert_eq!(parsed.name, name, "{text}");
        // Whatever came in, the clone URL is one shape.
        assert_eq!(
            parsed.clone_url,
            format!("https://{host}/{owner}/{name}.git")
        );
    }
}

#[test]
fn an_ssh_url_parses_into_its_https_equivalent() {
    // Parsed, not accepted: the caller refuses it, and it can only say why because this parsed.
    let parsed = parse_repo_url("git@github.com:rust-lang/rust.git").unwrap();
    assert_eq!(parsed.host, "github.com");
    assert_eq!(parsed.owner, "rust-lang");
    assert_eq!(parsed.name, "rust");
    assert_eq!(parsed.clone_url, "https://github.com/rust-lang/rust.git");

    // An ssh URL on a host nothing knows still parses — the suffix is not what identifies it.
    let parsed = parse_repo_url("git@git.example.com:team/thing.git").unwrap();
    assert_eq!(parsed.clone_url, "https://git.example.com/team/thing.git");
}

#[test]
fn anything_that_is_not_a_repository_is_refused() {
    let cases = [
        "",
        "rust-lang/rust",
        "just some text",
        // An owner is not a repository.
        "https://github.com/rust-lang",
        "https://github.com/",
        "https://github.com",
        // A page inside a repository is a page.
        "https://github.com/rust-lang/rust/issues",
        // A query string or a fragment means a page, not a clone target.
        "https://github.com/rust-lang/rust?tab=readme",
        "https://github.com/rust-lang/rust#install",
        // GitHub paths that look like `owner/name` and are not.
        "https://github.com/settings/profile",
        "https://github.com/marketplace/actions/checkout",
        "https://github.com/orgs/rust-lang",
        "https://github.com/notifications/beta",
        "https://github.com/explore/things",
        // An unknown host with no `.git` to identify it.
        "https://example.com/team/thing",
        // Not https, and not the ssh form either.
        "http://github.com/rust-lang/rust",
        "ftp://github.com/rust-lang/rust",
        "https://",
    ];

    for text in cases {
        assert!(parse_repo_url(text).is_none(), "{text} is not a repository");
    }
}

#[test]
fn a_clone_request_crosses_whole() {
    let message = Message::CloneRepo {
        request: CloneRequest {
            clone_id: CloneId::generate(),
            source: RepoSource::Url("https://github.com/rust-lang/rust.git".to_string()),
            branch: Some("main".to_string()),
            shallow: true,
            parent: "/Users/someone/code".to_string(),
            name: "rust".to_string(),
            ephemeral: false,
        },
    };

    let json = serde_json::to_string(&message).unwrap();
    let back = serde_json::from_str::<Message>(&json).unwrap();
    let Message::CloneRepo { request } = back else {
        panic!("not a clone");
    };
    let Message::CloneRepo { request: sent } = message else {
        unreachable!()
    };
    assert_eq!(request, sent);
    // The destination is `parent/name`, so both halves have to survive as they were typed.
    assert_eq!(request.parent, "/Users/someone/code");
    assert_eq!(request.name, "rust");
}

#[test]
fn a_settings_record_written_before_the_two_roots_still_reads() {
    // A schema-3 blob: every field added since defaults, which is what lets an older record parse.
    let json = r#"{
        "schema": 3,
        "isolate_agents": false,
        "search_excludes": ["target"],
        "search_fallbacks": ["grep"]
    }"#;

    let settings = serde_json::from_str::<HostSettings>(json).unwrap();
    assert_eq!(settings.schema, 3);
    assert_eq!(settings.projects_root, None);
    assert_eq!(settings.ephemeral_root, None);

    // And a record this build writes round-trips with both roots set.
    let written = HostSettings {
        projects_root: Some("/Users/someone/code".to_string()),
        ephemeral_root: Some("/Users/someone/.cache/ubiq".to_string()),
        ..HostSettings::default()
    };
    assert_eq!(written.schema, HOST_SETTINGS_SCHEMA);
    let json = serde_json::to_string(&written).unwrap();
    assert_eq!(
        serde_json::from_str::<HostSettings>(&json).unwrap(),
        written
    );
}
