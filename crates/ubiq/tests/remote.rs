//! The remote-connect modal's pure parsing, exercised with no window, no host and no socket —
//! see `crates/ubiq/src/state/remote.rs`'s module doc for why this is the layer that can be
//! tested this way at all.

use ubiq::state::remote::{AttemptId, parse_connection_string, with_default_port};

#[test]
fn full_connection_string_yields_address_and_token() {
    let parsed = parse_connection_string("http://192.168.1.5:7420?token=abc123");
    assert_eq!(parsed.address, "192.168.1.5:7420");
    assert_eq!(parsed.token.as_deref(), Some("abc123"));
}

#[test]
fn connection_string_with_no_port_keeps_the_bare_host() {
    let parsed = parse_connection_string("http://ubiq.local?token=abc123");
    assert_eq!(parsed.address, "ubiq.local");
    assert_eq!(parsed.token.as_deref(), Some("abc123"));
    assert_eq!(with_default_port(&parsed.address), "ubiq.local:7420");
}

#[test]
fn connection_string_with_no_token_leaves_it_none() {
    let parsed = parse_connection_string("http://192.168.1.5:7420");
    assert_eq!(parsed.address, "192.168.1.5:7420");
    assert_eq!(parsed.token, None);
}

#[test]
fn a_token_with_url_safe_base64_characters_round_trips() {
    // Ubiq's own tokens are base64url, so `-` and `_` must survive — a parser that treated either
    // as a delimiter would silently truncate every token this host ever prints.
    let parsed = parse_connection_string("http://host:7420?token=aB3-_xyZ-9_Q");
    assert_eq!(parsed.token.as_deref(), Some("aB3-_xyZ-9_Q"));
}

#[test]
fn a_malformed_string_still_yields_an_address_rather_than_nothing() {
    let parsed = parse_connection_string("not a url at all!!");
    assert_eq!(parsed.address, "not a url at all!!");
    assert_eq!(parsed.token, None);
}

#[test]
fn an_address_pasted_alone_has_no_token() {
    let parsed = parse_connection_string("192.168.1.5");
    assert_eq!(parsed.address, "192.168.1.5");
    assert_eq!(parsed.token, None);
    assert_eq!(with_default_port(&parsed.address), "192.168.1.5:7420");
}

#[test]
fn an_address_with_an_explicit_port_is_left_alone() {
    assert_eq!(with_default_port("192.168.1.5:9000"), "192.168.1.5:9000");
}

#[test]
fn a_trailing_colon_with_nothing_after_it_still_gets_the_default_port() {
    assert_eq!(with_default_port("192.168.1.5:"), "192.168.1.5::7420");
}

#[test]
fn https_scheme_is_stripped_the_same_as_http() {
    let parsed = parse_connection_string("https://host:7420?token=abc");
    assert_eq!(parsed.address, "host:7420");
}

#[test]
fn a_trailing_slash_on_the_address_is_dropped() {
    let parsed = parse_connection_string("http://host:7420/?token=abc");
    assert_eq!(parsed.address, "host:7420");
}

#[test]
fn attempt_ids_are_never_equal_to_a_fresh_one() {
    let a = AttemptId::generate();
    let b = AttemptId::generate();
    assert_ne!(a, b);
}
