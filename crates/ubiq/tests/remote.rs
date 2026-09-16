//! The remote-connect modal's pure parsing, exercised with no window, no host and no socket —
//! see `crates/ubiq/src/state/remote.rs`'s module doc for why this is the layer that can be
//! tested this way at all.

use ubiq::app::ssh_connect::DeployStep;
use ubiq::app::{AppState, BusHub};
use ubiq::state::remote::{AttemptId, parse_connection_string, with_default_port};
use ubiq::state::windows::WindowRegistry;
use ubiq::ui::remote_connect::deploy_note;
use ubiq_proto::ids::SshProfileId;
use ubiq_proto::settings::{DronePreset, RemoteCarrier, RemoteScheme, SavedRemoteHost};

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

/// One `Connecting` line, four extra sentences for the deploy a first connect may have to run.
/// No deploy is the dial's own wording, which is what every connect to a machine that already has
/// a drone shows — the common case keeps the sentence it always had.
#[test]
fn every_deploy_step_says_something_of_its_own() {
    let plain = deploy_note(None);
    let steps = [
        DeployStep::CheckingCache,
        DeployStep::Probing,
        DeployStep::Resolving,
        DeployStep::Uploading,
    ];
    let mut said: Vec<&str> = steps.iter().map(|step| deploy_note(Some(*step))).collect();
    assert!(said.iter().all(|note| !note.is_empty()));
    assert!(said.iter().all(|note| *note != plain));
    said.sort_unstable();
    said.dedup();
    assert_eq!(said.len(), steps.len(), "each step reads differently");
}

/// The drone path a dial actually launched is what the saved host keeps — the whole of what makes
/// a deploy run once rather than on every connect to the same machine. The next dial for this
/// profile and folder reads it straight back.
#[gpui::test]
fn a_saved_host_keeps_the_drone_path_a_dial_used(cx: &mut gpui::TestAppContext) {
    use gpui::AppContext as _;

    let (hub, _host) = ubiq_proto::bus::hub();
    cx.update(|cx| {
        gpui_component::init(cx);
        ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
        BusHub::install(hub, cx);
        WindowRegistry::install(cx);
        ubiq::app::install_key_bindings(cx);
    });

    let held: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<AppState>>>> = Default::default();
    let taken = held.clone();
    let _handle = cx.add_window(move |window, cx| {
        let state = cx.new(|cx| AppState::for_project(None, 'A', window, cx));
        *taken.borrow_mut() = Some(state.clone());
        gpui_component::Root::new(state, window, cx)
    });
    cx.run_until_parked();
    let state = held
        .borrow_mut()
        .take()
        .expect("the window built its state");

    let profile = SshProfileId::generate();
    let deployed = "~/.cache/ubiq/drone/0.4.0/ubiq-drone";
    state.update(cx, |state, _cx| {
        state
            .workbench
            .settings
            .host
            .remote_hosts
            .push(SavedRemoteHost {
                id: "save-1".to_string(),
                name: "build box".to_string(),
                address: "build.example".to_string(),
                scheme: RemoteScheme::Http,
                trust_insecure: false,
                carrier: RemoteCarrier::Ssh {
                    profile,
                    root: "/srv/proj".to_string(),
                    preset: DronePreset::Session,
                    drone_path: None,
                },
            });

        // The first connect dialled the bare remote `PATH`, found nothing, and deployed.
        assert_eq!(state.saved_drone_path(profile, "/srv/proj"), None);
        state.remember_drone_path("save-1", Some(deployed.to_string()));
        assert_eq!(
            state.saved_drone_path(profile, "/srv/proj").as_deref(),
            Some(deployed)
        );

        // The second connect dials that path and reports it back unchanged, which files nothing
        // new — and a folder nobody has connected to still has no path of its own.
        state.remember_drone_path("save-1", Some(deployed.to_string()));
        assert_eq!(
            state.saved_drone_path(profile, "/srv/proj").as_deref(),
            Some(deployed)
        );
        assert_eq!(state.saved_drone_path(profile, "/srv/other"), None);
    });
}
