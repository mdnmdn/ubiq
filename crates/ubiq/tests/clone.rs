//! Cloning a project, as the interface holds it.
//!
//! No window and no host: everything asserted here is a free function or a method on `CloneState`,
//! which is what makes the discipline — discard an answer naming an id nobody holds — testable at
//! all.

use ubiq::app::Holds;
use ubiq::state::clone::{CloneState, check_url};
use ubiq_proto::ids::RepoQueryId;
use ubiq_proto::repos::RemoteRepo;

fn repo(name: &str) -> RemoteRepo {
    RemoteRepo {
        id: format!("acme/{name}"),
        name: name.to_string(),
        full_name: format!("acme/{name}"),
        description: None,
        default_branch: Some("main".to_string()),
        private: false,
        clone_url: format!("https://github.com/acme/{name}.git"),
        pushed_at: None,
    }
}

#[test]
fn a_listing_naming_the_id_we_asked_with_is_taken() {
    let asked = RepoQueryId::generate();
    let mut clone = CloneState {
        repos_query: Some(asked),
        ..CloneState::default()
    };

    assert!(clone.accept_repos(asked, vec![repo("router")], true));
    assert_eq!(clone.repos.len(), 1);
    assert!(clone.truncated);
    assert_eq!(clone.repos_query, None);
}

#[test]
fn a_listing_naming_a_stale_id_is_discarded() {
    let asked = RepoQueryId::generate();
    let mut clone = CloneState {
        repos_query: Some(asked),
        repos: vec![repo("router")],
        ..CloneState::default()
    };

    // The answer to a question that was replaced: it names an id nobody holds any more.
    assert!(!clone.accept_repos(RepoQueryId::generate(), vec![repo("other")], false));
    assert_eq!(clone.repos.len(), 1);
    assert_eq!(clone.repos[0].name, "router");
    assert_eq!(clone.repos_query, Some(asked));
}

#[test]
fn a_branch_listing_naming_a_stale_id_is_discarded() {
    let asked = RepoQueryId::generate();
    let mut clone = CloneState {
        branches_query: Some(asked),
        ..CloneState::default()
    };

    assert!(!clone.accept_branches(RepoQueryId::generate(), vec!["main".into()], None));
    assert!(clone.branches.is_empty());
    assert!(clone.accept_branches(asked, vec!["main".into()], Some("main".into())));
    assert_eq!(clone.branch.as_deref(), Some("main"));
}

#[test]
fn a_branch_already_picked_survives_the_listing_that_lands_after_it() {
    let asked = RepoQueryId::generate();
    let mut clone = CloneState {
        branches_query: Some(asked),
        branch: Some("release".to_string()),
        ..CloneState::default()
    };

    clone.accept_branches(
        asked,
        vec!["main".into(), "release".into()],
        Some("main".into()),
    );
    assert_eq!(clone.branch.as_deref(), Some("release"));
}

#[test]
fn the_filter_runs_in_memory_and_only_asks_the_provider_when_it_runs_dry() {
    let mut clone = CloneState {
        repos: vec![repo("router"), repo("ledger")],
        truncated: true,
        filter: "rtr".to_string(),
        ..CloneState::default()
    };
    assert_eq!(clone.visible().len(), 1);
    assert!(!clone.wants_server_search());

    clone.filter = "zzz".to_string();
    assert!(clone.visible().is_empty());
    assert!(clone.wants_server_search());

    // Nothing local and nothing more to fetch is an honest "no matches", not another request.
    clone.truncated = false;
    assert!(!clone.wants_server_search());
}

#[test]
fn an_ssh_url_is_refused_with_a_sentence_rather_than_called_unparseable() {
    let refused = check_url("git@github.com:acme/router.git").expect("not empty");
    assert!(refused.unwrap_err().contains("https"));
    assert!(check_url("  ").is_none());
    assert!(
        check_url("https://github.com/acme/router")
            .expect("not empty")
            .is_ok()
    );
}

#[test]
fn an_ephemeral_project_always_holds_something_and_says_what_is_lost() {
    let holds = Holds {
        files: 0,
        panes: 0,
        ephemeral: true,
    };
    assert!(holds.anything());
    assert_eq!(
        holds.sentence().as_deref(),
        Some("This clone will be discarded")
    );

    // The folder outranks the counts: what is lost is the whole clone, not two buffers.
    let busy = Holds {
        files: 2,
        panes: 1,
        ephemeral: true,
    };
    assert_eq!(
        busy.sentence().as_deref(),
        Some("This clone will be discarded")
    );

    let ordinary = Holds {
        files: 2,
        panes: 1,
        ephemeral: false,
    };
    assert_eq!(
        ordinary.sentence().as_deref(),
        Some("2 unsaved files and 1 terminal")
    );
}
