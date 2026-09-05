//! A destination written down and read back.
//!
//! The round trip is the whole contract: whatever `Display` prints, `FromStr` has to return the
//! same value for, or a bookmark from three weeks ago lands somewhere else. Everything here is
//! arithmetic over ids and bytes — no window, no host.

use std::str::FromStr;

use ubiq::state::dock::ChatId;
use ubiq::state::editor::{Subject, tab_key};
use ubiq::state::nav::{Destination, Locus, View, resolve_relative};
use ubiq_proto::files::DiffBase;
use ubiq_proto::ids::{PaneId, ProjectId, SessionId, TaskId};
use ubiq_proto::work::AgentId;

use ubiq::state::orchestration::{InspectorTab, Selection};

fn project() -> ProjectId {
    ProjectId::generate()
}

/// Every view arm, one of each shape the parser has to tell apart.
fn views() -> Vec<View> {
    vec![
        View::Control,
        View::Kb,
        View::Git,
        View::Logs,
        View::Ide {
            key: "crates/ubiq/src/app/nav.rs".into(),
        },
        View::Ide {
            key: tab_key("README.md", Subject::Diff(DiffBase::Head)),
        },
        View::Explorer {
            path: "_docs/features".into(),
        },
        View::Terminal {
            pane: PaneId::generate(),
        },
        View::Graph {
            selection: Selection::Session(SessionId::generate()),
            tab: InspectorTab::Chat,
        },
        View::Graph {
            selection: Selection::Agent(AgentId::generate()),
            tab: InspectorTab::Tasks,
        },
        View::Agents {
            agent: AgentId::generate(),
        },
        View::Tasks {
            task: TaskId::generate(),
        },
        View::Chat {
            chat: ChatId::generate(),
        },
    ]
}

fn loci() -> Vec<Option<Locus>> {
    vec![
        None,
        Some(Locus::Line { line: 1712 }),
        Some(Locus::Span { from: 10, to: 24 }),
        Some(Locus::Anchor {
            slug: "catalogue".into(),
        }),
        Some(Locus::Viewport {
            x: -120.5,
            y: 40.0,
            scale: 1.25,
        }),
        Some(Locus::Node { key: "n-7".into() }),
    ]
}

#[test]
fn every_arm_and_every_locus_round_trips() {
    let project = project();
    for view in views() {
        for locus in loci() {
            let dest = Destination {
                project,
                view: view.clone(),
                locus,
            };
            let text = dest.to_string();
            let back = Destination::from_str(&text)
                .unwrap_or_else(|_| panic!("{text} did not parse back"));
            assert_eq!(back, dest, "{text}");
        }
    }
}

#[test]
fn the_written_examples_parse_as_what_they_say() {
    let id = project();
    let task = TaskId::generate();
    let agent = AgentId::generate();
    let cases: Vec<(String, Destination)> = vec![
        (
            format!("ubiq://{id}/ide/crates/ubiq/src/app/wire.rs#L1712"),
            Destination {
                project: id,
                view: View::Ide {
                    key: "crates/ubiq/src/app/wire.rs".into(),
                },
                locus: Some(Locus::Line { line: 1712 }),
            },
        ),
        (
            format!("ubiq://{id}/ide/README.md#L10-24"),
            Destination {
                project: id,
                view: View::Ide {
                    key: "README.md".into(),
                },
                locus: Some(Locus::Span { from: 10, to: 24 }),
            },
        ),
        (
            format!("ubiq://{id}/ide/_docs/INDEX.md#catalogue"),
            Destination {
                project: id,
                view: View::Ide {
                    key: "_docs/INDEX.md".into(),
                },
                locus: Some(Locus::Anchor {
                    slug: "catalogue".into(),
                }),
            },
        ),
        (
            format!("ubiq://{id}/tasks/{task}"),
            Destination::new(id, View::Tasks { task }),
        ),
        // The proposal wrote this one as `agents/<id>/chat`, from before the arm split: the
        // agents screen *is* the transcript, and the inspector tab belongs to the graph.
        (
            format!("ubiq://{id}/agents/{agent}"),
            Destination::new(id, View::Agents { agent }),
        ),
    ];
    for (text, want) in cases {
        assert_eq!(Destination::from_str(&text).ok(), Some(want), "{text}");
    }
}

#[test]
fn a_reversed_span_is_the_same_two_lines() {
    let id = project();
    let dest = Destination::from_str(&format!("ubiq://{id}/ide/a.rs#L24-10")).unwrap();
    assert_eq!(dest.locus, Some(Locus::Span { from: 10, to: 24 }));
}

#[test]
fn junk_is_not_a_link() {
    let id = project();
    for text in [
        "https://example.com/a".to_string(),
        "ubiq://not-a-ulid/logs".to_string(),
        format!("ubiq://{id}/nowhere"),
        // A slug that names nothing takes nothing: a trailing segment is a different string.
        format!("ubiq://{id}/logs/extra"),
        format!("ubiq://{id}/ide/a%zz.rs"),
        format!("ubiq://{id}/ide/a.rs#L0"),
        // One segment means one.
        format!("ubiq://{id}/tasks/{}/extra", TaskId::generate()),
        // The selection prefix is forced.
        format!("ubiq://{id}/graph/{}", SessionId::generate()),
    ] {
        assert!(Destination::from_str(&text).is_err(), "{text} parsed");
    }
}

#[test]
fn a_path_carrying_the_grammars_own_characters_survives() {
    let id = project();
    let key = "_docs/a #1 100% done.md".to_string();
    let dest = Destination {
        project: id,
        view: View::Ide { key: key.clone() },
        locus: Some(Locus::Line { line: 3 }),
    };
    let text = dest.to_string();
    assert!(
        text.contains("%23") && text.contains("%25") && text.contains("%20"),
        "{text}"
    );
    assert_eq!(Destination::from_str(&text).unwrap(), dest);
}

#[test]
fn utf8_is_left_readable() {
    let id = project();
    let dest = Destination::new(
        id,
        View::Ide {
            key: "_docs/café.md".into(),
        },
    );
    assert!(dest.to_string().contains("café"));
    assert_eq!(Destination::from_str(&dest.to_string()).unwrap(), dest);
}

#[test]
fn an_anchor_that_looks_like_another_locus_says_it_is_an_anchor() {
    let id = project();
    for slug in ["L42", "L10-24", "v=1,2,3", "n=key", "a=x"] {
        let dest = Destination {
            project: id,
            view: View::Ide { key: "a.md".into() },
            locus: Some(Locus::Anchor { slug: slug.into() }),
        };
        let text = dest.to_string();
        assert!(text.contains("#a="), "{text}");
        assert_eq!(Destination::from_str(&text).unwrap(), dest, "{text}");
    }
}

#[test]
fn a_path_that_leaves_the_project_is_not_a_link() {
    let id = project();
    for path in ["../secrets", "/etc/passwd", "a/../../b"] {
        let text = format!("ubiq://{id}/ide/{path}");
        assert!(Destination::from_str(&text).is_err(), "{text} parsed");
    }
}

// ── relative links, as a document writes them ──────────────────────────────

fn relative(target: &str) -> Option<Destination> {
    resolve_relative(project(), "_docs/x.md", target)
}

fn key_of(dest: &Destination) -> String {
    match &dest.view {
        View::Ide { key } => key.clone(),
        other => panic!("not a file: {other:?}"),
    }
}

#[test]
fn a_relative_link_walks_from_the_documents_folder() {
    let up = relative("../src/app.rs#L200").unwrap();
    assert_eq!(key_of(&up), "src/app.rs");
    assert_eq!(up.locus, Some(Locus::Line { line: 200 }));

    // `.` and a bare name are the same thing: the document's own folder.
    assert_eq!(
        key_of(&relative("./sibling.md").unwrap()),
        "_docs/sibling.md"
    );
    assert_eq!(key_of(&relative("sibling.md").unwrap()), "_docs/sibling.md");
}

#[test]
fn escaping_the_project_root_is_not_a_link() {
    assert_eq!(relative("../../../.."), None);
    assert_eq!(relative("/etc/passwd"), None);
}

#[test]
fn a_bare_fragment_is_the_same_document() {
    let id = project();
    let here = resolve_relative(id, "_docs/x.md", "#heading").unwrap();
    assert_eq!(key_of(&here), "_docs/x.md");
    assert_eq!(
        here.locus,
        Some(Locus::Anchor {
            slug: "heading".into()
        })
    );
}

#[test]
fn somewhere_else_entirely_is_left_to_the_operating_system() {
    assert_eq!(relative("https://example.com"), None);
    assert_eq!(relative("http://example.com"), None);
    assert_eq!(relative("mailto:someone@example.com"), None);
}

#[test]
fn a_full_link_inside_a_document_wins_over_the_document() {
    let elsewhere = ProjectId::generate();
    let written = format!("ubiq://{elsewhere}/tasks/{}", TaskId::generate());
    let dest = resolve_relative(project(), "_docs/x.md", &written).unwrap();
    assert_eq!(dest.project, elsewhere);
    assert_eq!(dest.to_string(), written);
}
