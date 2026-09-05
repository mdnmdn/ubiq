//! What the ⌘K navigator offers, and what a project remembers arriving at.
//!
//! No window and no host: `rows` names neither, which is the whole reason it is a free function.

use ubiq::state::dock::ChatId;
use ubiq::state::nav::{Bookmark, Destination, Locus, View};
use ubiq::state::navigator::{Group, NavAction, NavRow, RECENTS_MAX, kept_recents, remember, rows};
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::work::AgentId;

fn file(project: ProjectId, path: &str) -> Destination {
    Destination::new(project, View::Ide { key: path.into() })
}

fn mark(project: ProjectId, name: &str, path: &str) -> Bookmark {
    Bookmark {
        name: name.to_string(),
        dest: file(project, path),
        note: String::new(),
        anchor: None,
        adrift: false,
    }
}

/// Nine of each, so a cap of eight is visible.
fn nine(project: ProjectId, prefix: &str) -> Vec<Destination> {
    (1..=9)
        .map(|n| file(project, &format!("{prefix}/{n}.rs")))
        .collect()
}

fn ask(
    query: &str,
    project: ProjectId,
    recents: &[Destination],
    marks: &[Bookmark],
) -> Vec<NavRow> {
    let files = vec![
        ("router.rs".to_string(), "src/router.rs".to_string(), false),
        ("routes".to_string(), "src/routes".to_string(), true),
    ];
    let tasks = vec![(TaskId::generate(), "Rewire the router".to_string())];
    let agents = vec![(
        AgentId::generate(),
        "router-bot".to_string(),
        "reviewer".to_string(),
    )];
    rows(
        query,
        project,
        recents,
        marks,
        &files,
        &tasks,
        &agents,
        &|id| (id == project).then(|| "Ubiq".to_string()),
    )
}

/// Nothing typed is "where was I": what was arrived at, then what was written down — and no files,
/// because a file list with no query is the explorer.
#[test]
fn an_empty_query_is_recents_then_bookmarks() {
    let project = ProjectId::generate();
    let recents = nine(project, "recent");
    let marks: Vec<Bookmark> = (1..=9)
        .map(|n| mark(project, &format!("mark {n}"), &format!("marked/{n}.rs")))
        .collect();

    let found = ask("", project, &recents, &marks);
    let groups: Vec<Group> = found.iter().map(|row| row.group).collect();

    // Eight of each, capped and stopped, in that order.
    assert_eq!(found.len(), 16);
    assert!(groups[..8].iter().all(|group| *group == Group::Recent));
    assert!(groups[8..].iter().all(|group| *group == Group::Bookmark));
    assert!(!groups.contains(&Group::File));
    assert!(!groups.contains(&Group::Task));
}

/// A query reaches every group, and the groups keep the order they are drawn in.
#[test]
fn a_query_filters_across_groups_in_order() {
    let project = ProjectId::generate();
    let recents = vec![file(project, "src/router.rs"), file(project, "README.md")];
    let marks = vec![mark(project, "router entry", "src/router.rs")];

    let found = ask("rout", project, &recents, &marks);
    let groups: Vec<Group> = found.iter().map(|row| row.group).collect();

    assert_eq!(
        groups,
        vec![
            Group::Recent,
            Group::Bookmark,
            Group::File,
            Group::File,
            Group::Task,
            Group::Agent,
        ]
    );
    // The one recent that does not match is gone, and the folder row goes to the explorer.
    assert!(found.iter().all(|row| !row.label.contains("README")));
    assert_eq!(
        found[3].dest.as_ref().map(|dest| dest.view.clone()),
        Some(View::Explorer {
            path: "src/routes".into()
        })
    );
}

/// A pasted link names one place, so the list is that place and nothing else.
#[test]
fn a_valid_uri_collapses_to_one_row() {
    let project = ProjectId::generate();
    let link = Destination {
        project,
        view: View::Ide {
            key: "src/app.rs".into(),
        },
        locus: Some(Locus::Line { line: 12 }),
    }
    .to_string();

    let found = ask(&link, project, &nine(project, "recent"), &[]);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].group, Group::Uri);
    // The project's name, not its ULID.
    assert!(found[0].label.starts_with("Ubiq · "));
    assert!(!found[0].label.contains(&project.to_string()));
    assert!(found[0].dest.is_some());
}

/// A link to a project this catalogue does not hold says so, and goes nowhere.
#[test]
fn a_link_to_an_unknown_project_says_so_and_is_not_clickable() {
    let project = ProjectId::generate();
    let link = file(ProjectId::generate(), "src/app.rs").to_string();

    let found = ask(&link, project, &[], &[]);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].group, Group::Uri);
    assert_eq!(found[0].dest, None);
    assert_eq!(found[0].label, "Unknown project");
}

/// Arriving is remembered newest first, and the same *place* at a new line is not a second entry —
/// otherwise scrolling one file leaves thirty-two copies of it.
#[test]
fn recents_dedupe_by_place_and_keep_the_newest_first() {
    let project = ProjectId::generate();
    let mut recents = Vec::new();

    remember(&mut recents, &file(project, "a.rs"));
    remember(&mut recents, &file(project, "b.rs"));
    let mut scrolled = file(project, "a.rs");
    scrolled.locus = Some(Locus::Line { line: 200 });
    remember(&mut recents, &scrolled);

    assert_eq!(recents.len(), 2);
    let kept = kept_recents(&recents);
    assert_eq!(kept[0].line(), Some(200));
    assert_eq!(kept[0].label(), "a.rs");
    assert_eq!(kept[1].label(), "b.rs");
}

/// The list is bounded, and what falls off the end is the oldest.
#[test]
fn recents_stop_at_the_cap() {
    let project = ProjectId::generate();
    let mut recents = Vec::new();
    for n in 0..RECENTS_MAX + 10 {
        remember(&mut recents, &file(project, &format!("{n}.rs")));
    }

    assert_eq!(recents.len(), RECENTS_MAX);
    let kept = kept_recents(&recents);
    assert_eq!(kept[0].label(), format!("{}.rs", RECENTS_MAX + 9));
    assert_eq!(kept[RECENTS_MAX - 1].label(), format!("{}.rs", 10));
}

/// A place that does not survive a restart is never written down, and a stored line this build no
/// longer parses is one lost row rather than a lost list.
#[test]
fn recents_keep_only_what_survives_and_still_parses() {
    let project = ProjectId::generate();
    let mut recents = Vec::new();
    remember(
        &mut recents,
        &Destination::new(
            project,
            View::Chat {
                chat: ChatId::generate(),
            },
        ),
    );
    assert!(recents.is_empty());

    recents.push("not a link".to_string());
    remember(&mut recents, &file(project, "a.rs"));
    assert_eq!(kept_recents(&recents).len(), 1);
}

/// A repository URL is an answer, not a filter: the list collapses to the one thing that can be
/// done with it, and the row carries an action rather than a place.
#[test]
fn a_repository_url_collapses_the_list_to_a_clone_row() {
    let project = ProjectId::generate();
    let found = ask("https://github.com/acme/router", project, &[], &[]);

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].group, Group::Clone);
    assert!(found[0].dest.is_none());
    assert_eq!(
        found[0].action,
        Some(NavAction::Clone(
            "https://github.com/acme/router".to_string()
        ))
    );
}

/// Ordinary text is still a filter. Nothing about "router" is a URL, so the groups answer as they
/// always did.
#[test]
fn ordinary_text_is_still_filtered_rather_than_offered_as_a_clone() {
    let project = ProjectId::generate();
    let found = ask("router", project, &[], &[]);

    assert!(found.iter().all(|row| row.group != Group::Clone));
    assert!(found.iter().any(|row| row.group == Group::File));
}
