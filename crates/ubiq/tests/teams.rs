//! Selecting on the Teams canvas, and what it does to the conversation underneath.
//!
//! `TeamsSelection` is not only a fact the canvas reads back at itself — a session, an agent or one
//! of an agent's delegates selected there points the shared conversation at it too, through
//! `AppState::view_conversation_agent`. That field is the conversation's own, not the canvas's: a
//! chat tab and an agents column reading the same agent read the same `viewing`, so a selection
//! made on one surface is not invisible to the others. Nothing here renders a frame — the claim is
//! about state, and `chat.rs`'s `Fixture` is the pattern this one borrows.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::layout::Algo;
use ubiq::state::nav::{Destination, View};
use ubiq::state::new_agent::Target;
use ubiq::state::teams::{
    CARD_HEIGHT, CARD_WIDTH, TeamsHeld, TeamsInspectorTab, TeamsSelection, TeamsSpan, window_work,
};
use ubiq::state::work::WorkProjection;
use ubiq::state::{RailMode, WindowRegistry};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::messages::{AgentTypeInfo, Message};
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{Activity, AgentId, TaskRecord, WorkAgent, WorkSession};

const PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

struct Fixture {
    state: Entity<AppState>,
    _window: WindowHandle<Root>,
    host: bus::HostEnd,
    project: ProjectId,
}

impl Fixture {
    fn open(cx: &mut TestAppContext) -> Self {
        let snapshot = a_project();
        let project = snapshot.record.id;
        let (hub, host) = bus::hub();

        cx.update(|cx| {
            gpui_component::init(cx);
            ubiq::theme::set_mode(ubiq::app::boot_theme(), cx);
            BusHub::install(hub, cx);
            WindowRegistry::install(cx);
            cx.global_mut::<WindowRegistry>().apply(snapshot);
        });

        let held: Rc<RefCell<Option<Entity<AppState>>>> = Rc::default();
        let taken = held.clone();
        let window = cx.add_window(move |window, cx| {
            let state = cx.new(|cx| AppState::for_project(Some(project), 'A', window, cx));
            *taken.borrow_mut() = Some(state.clone());
            Root::new(state, window, cx)
        });
        cx.run_until_parked();

        let state = held
            .borrow_mut()
            .take()
            .expect("the window built its state");
        Self {
            state,
            _window: window,
            host,
            project,
        }
    }

    /// A second project, opened in this window beside the first and left held but *not* active.
    ///
    /// The window span's whole premise: two `OpenProject`s under one window, with the rail and the
    /// titlebar still pointed at the first. `open_in` points the window at what it just opened, so
    /// the first is put back in front — widening the canvas is not moving the window.
    fn hold_a_second(&self, cx: &mut TestAppContext) -> ProjectId {
        let snapshot = a_project_named("other");
        let second = snapshot.record.id;
        cx.update(|cx| {
            let registry = cx.global_mut::<WindowRegistry>();
            let window = registry.windows.first().expect("the fixture's window").id;
            registry.apply(snapshot);
            registry.open_in(window, second);
        });
        cx.run_until_parked();
        self.state
            .update(cx, |state, cx| state.activate_project(self.project, cx));
        cx.run_until_parked();
        second
    }

    /// The host says an agent is up, exactly as `StartConversation` is answered.
    fn started(&self, agent: WorkAgent, cx: &mut TestAppContext) {
        self.started_in(self.project, agent, cx);
    }

    /// The same, for any project the window holds — what gives the window span something to merge.
    fn started_in(&self, project: ProjectId, agent: WorkAgent, cx: &mut TestAppContext) {
        let session = WorkSession {
            id: agent.session,
            name: "the project".to_string(),
            branch: String::new(),
            worktree: false,
        };
        self.host.send(
            To::Everyone,
            Message::ConversationStarted {
                project_id: project,
                agent: Box::new(agent),
                session,
                accepts_input: true,
            },
        );
        cx.run_until_parked();
    }

    /// The host says a task exists in one project, exactly as `CreateTask` is answered.
    fn a_task_in(&self, project: ProjectId, title: &str, cx: &mut TestAppContext) -> TaskId {
        let task = TaskRecord::new(title.to_string(), None, Utc::now());
        let id = task.id;
        self.host.send(
            To::Everyone,
            Message::TaskCreated {
                project_id: project,
                task,
            },
        );
        cx.run_until_parked();
        id
    }

    /// Everything this window has said to the host, drained — the helper the chat and conversation
    /// tests use, for the assertions about what was written down rather than what the window holds.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }
}

fn a_project() -> ProjectSnapshot {
    a_project_named("ubiq")
}

fn a_project_named(name: &str) -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: name.to_string(),
            path: format!("/tmp/{name}"),
            colour: 0,
            custom_colour: None,
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
        },
        health: ProjectHealth::Ok,
        open_panes: 0,
        ephemeral: false,
        workarea: "/tmp/ubiq-workarea".to_string(),
    }
}

fn an_agent(id: AgentId, name: &str) -> WorkAgent {
    WorkAgent {
        id,
        session: ubiq_proto::ids::SessionId::generate(),
        task: None,
        parent: None,
        name: name.to_string(),
        summary: None,
        role: "Implementer".to_string(),
        activity: Activity::Writing,
        note: String::new(),
        branch: "main".to_string(),
        tokens: 0.0,
        harness: "Claude Code".to_string(),
        account: "work".to_string(),
        model: String::new(),
        context_pct: 0,
        persistent: false,
        accept_all: false,
        debug_dump: None,
        run_dir: None,
        config_dir: None,
        thread: Vec::new(),
    }
}

/// One project's work as the host would have sent it: a session per agent, no tasks.
fn a_projection(agents: Vec<WorkAgent>, loaded: bool) -> WorkProjection {
    let sessions = agents
        .iter()
        .map(|agent| WorkSession {
            id: agent.session,
            name: agent.name.clone(),
            branch: String::new(),
            worktree: false,
        })
        .collect();
    WorkProjection {
        sessions,
        agents,
        tasks: Vec::new(),
        loaded,
    }
}

/// **The merge is a concatenation plus an owner map.** Two projects' live work becomes one
/// projection in the order given, every card answers which project minted it, and an agent the
/// window holds no conversation with is dropped by `live_work` before the merge ever sees it — the
/// narrowing each project gets on its own is the narrowing the window span gets too.
#[test]
fn window_work_merges_each_project_s_live_agents_and_remembers_whose_they_are() {
    let one = ProjectId::generate();
    let two = ProjectId::generate();
    let here = an_agent(AgentId::generate(), "here");
    let there = an_agent(AgentId::generate(), "there");
    let dark = an_agent(AgentId::generate(), "no conversation");

    let a = a_projection(vec![here.clone(), dark.clone()], true);
    let b = a_projection(vec![there.clone()], true);

    let (work, owner) = window_work(&[(one, &a, &[here.id][..]), (two, &b, &[there.id][..])]);

    let drawn: Vec<AgentId> = work.agents.iter().map(|agent| agent.id).collect();
    assert_eq!(
        drawn,
        vec![here.id, there.id],
        "both projects' live agents, in the order the window holds them"
    );
    assert_eq!(owner.get(&here.id), Some(&one));
    assert_eq!(owner.get(&there.id), Some(&two));
    assert_eq!(
        owner.get(&dark.id),
        None,
        "an agent with no live conversation is not on the canvas and so has no owner"
    );
    assert_eq!(work.sessions.len(), 2, "one session per drawn agent");
    assert!(work.loaded, "every project had answered its ListWork");

    // One project still waiting on the host makes the whole canvas unloaded: the merged claim is
    // "nothing here is still arriving", and half an answer does not support it.
    let (work, _) = window_work(&[
        (one, &a, &[here.id][..]),
        (
            two,
            &a_projection(vec![there.clone()], false),
            &[there.id][..],
        ),
    ]);
    assert!(!work.loaded);
}

/// Selecting a delegate on the canvas points the shared conversation at it — the same field
/// `view_conversation_agent` writes for the agent switcher, because a delegate has no transcript of
/// its own to point at: what opens is its parent's, read from the delegate's turns.
#[gpui::test]
fn selecting_a_subagent_points_the_conversation_at_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Claude Code"), cx);

    fixture.state.update(cx, |state, cx| {
        state.select_in_teams(
            TeamsSelection::Subagent {
                agent: id,
                subagent: "t751".to_string(),
            },
            cx,
        )
    });

    let (selection, viewing) = fixture.state.read_with(cx, |state, cx| {
        let teams = state.teams(cx).expect("the project's Teams view");
        let conversation = state.conversation(id, cx).expect("the live conversation");
        (teams.selection.clone(), conversation.viewing.clone())
    });
    assert_eq!(
        selection,
        Some(TeamsSelection::Subagent {
            agent: id,
            subagent: "t751".to_string(),
        })
    );
    assert_eq!(
        viewing.as_deref(),
        Some("t751"),
        "the canvas's selection and the conversation's own field agree"
    );
}

/// Selecting the card itself — not one of its delegates — clears whatever delegate the
/// conversation was showing. A card picked up while a delegate is being read stays on it (that is
/// `start_teams_carry`'s own rule); a plain selection is the way back to the main transcript.
#[gpui::test]
fn selecting_the_card_clears_the_delegate_it_was_reading(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Claude Code"), cx);

    fixture.state.update(cx, |state, cx| {
        state.select_in_teams(
            TeamsSelection::Subagent {
                agent: id,
                subagent: "t751".to_string(),
            },
            cx,
        )
    });
    fixture
        .state
        .read_with(cx, |state, cx| {
            state.conversation(id, cx).unwrap().viewing.clone()
        })
        .expect("the delegate is showing before the plain selection");

    fixture.state.update(cx, |state, cx| {
        state.select_in_teams(TeamsSelection::Agent(id), cx)
    });

    let (selection, viewing) = fixture.state.read_with(cx, |state, cx| {
        let teams = state.teams(cx).unwrap();
        let conversation = state.conversation(id, cx).unwrap();
        (teams.selection.clone(), conversation.viewing.clone())
    });
    assert_eq!(selection, Some(TeamsSelection::Agent(id)));
    assert_eq!(
        viewing, None,
        "the plain card selection put the transcript back on the main agent"
    );
}

/// **`viewing` is the conversation's own field, not the canvas's** — so whatever the Teams screen
/// last asked for is what a chat tab or an agents column attached to the same agent would also
/// read, since every surface reaches it through the one `AppState::conversation` accessor rather
/// than a copy of its own. This is the semantics the module doc promises; nothing before this test
/// pinned it down.
#[gpui::test]
fn the_conversation_s_viewing_field_is_shared_by_every_surface_reading_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Claude Code"), cx);

    fixture.state.update(cx, |state, cx| {
        state.select_in_teams(
            TeamsSelection::Subagent {
                agent: id,
                subagent: "t9".to_string(),
            },
            cx,
        )
    });

    // A chat tab and an agents column are two different readers, but they resolve to the same
    // `Conversation` — there is only one, keyed by agent id, in `OpenProject::conversations`.
    let from_a_chat_tab = fixture.state.read_with(cx, |state, cx| {
        state.conversation(id, cx).unwrap().viewing.clone()
    });
    let from_an_agents_column = fixture.state.read_with(cx, |state, cx| {
        state.conversation(id, cx).unwrap().viewing.clone()
    });
    assert_eq!(from_a_chat_tab.as_deref(), Some("t9"));
    assert_eq!(from_a_chat_tab, from_an_agents_column);
}

/// Which cards the canvas is drawing, through the one accessor every reader on that screen uses.
fn drawn(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<AgentId> {
    fixture.state.read_with(cx, |state, cx| {
        state
            .teams_work(cx)
            .expect("the canvas has work to draw")
            .agents
            .iter()
            .map(|agent| agent.id)
            .collect()
    })
}

/// **The span is what the canvas is scoped by, not the window's active project.** Two projects
/// held by one window: the project span draws the active one's card and nothing else — the reach
/// the screen has always had — and the window span draws both, because `teams_work` stops asking
/// `open_project` and asks `teams_projects` instead. Neither span moves the window: the active
/// project is the first one throughout.
#[gpui::test]
fn the_span_decides_whose_cards_the_canvas_draws(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let second = fixture.hold_a_second(cx);
    let here = AgentId::generate();
    let there = AgentId::generate();
    fixture.started(an_agent(here, "here"), cx);
    fixture.started_in(second, an_agent(there, "there"), cx);

    assert_eq!(
        fixture.state.read_with(cx, |state, _| state.teams_span),
        TeamsSpan::Project,
        "a window opens on the reach it had before the span existed"
    );
    assert_eq!(
        drawn(&fixture, cx),
        vec![here],
        "the project span draws the active project's agents alone"
    );

    fixture
        .state
        .update(cx, |state, cx| state.toggle_teams_span(cx));

    let both = drawn(&fixture, cx);
    assert_eq!(both.len(), 2, "the window span draws both projects' agents");
    assert!(both.contains(&here) && both.contains(&there));
    assert_eq!(
        fixture.state.read_with(cx, |state, cx| state.project(cx)),
        Some(fixture.project),
        "the span widened the canvas without pointing the window anywhere else"
    );
}

/// **Every card answers whose it is.** A merged projection has lost the project each record came
/// from, and every write the screen makes — the hand-over, the selection, the composer's send —
/// needs it back. `project_of_agent` is that answer, and it is the card's own project rather than
/// the window's under the window span.
#[gpui::test]
fn project_of_agent_answers_each_card_s_own_project_under_the_window_span(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let second = fixture.hold_a_second(cx);
    let here = AgentId::generate();
    let there = AgentId::generate();
    fixture.started(an_agent(here, "here"), cx);
    fixture.started_in(second, an_agent(there, "there"), cx);
    fixture
        .state
        .update(cx, |state, cx| state.toggle_teams_span(cx));

    fixture.state.read_with(cx, |state, cx| {
        assert_eq!(state.project_of_agent(here, cx), Some(fixture.project));
        assert_eq!(state.project_of_agent(there, cx), Some(second));
        // And the transcript follows the owner: the active project's map has never heard of the
        // foreign card, so `conversation` answers nothing for it where `teams_conversation` does.
        assert!(state.conversation(there, cx).is_none());
        assert!(state.teams_conversation(there, cx).is_some());
        assert!(state.teams_conversation(here, cx).is_some());
    });
}

/// **A link names the project of what it points at, and the span is not in the address.** A
/// selection made on the window span can be a card the window is not pointed at; the link built
/// from it names that card's project, so a window in the project span can follow it. What comes
/// back out of the text is the same `View::Teams` that went in.
#[gpui::test]
fn a_teams_link_names_the_selected_card_s_own_project(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let second = fixture.hold_a_second(cx);
    let there = AgentId::generate();
    fixture.started_in(second, an_agent(there, "there"), cx);

    fixture.state.update(cx, |state, cx| {
        state.toggle_teams_span(cx);
        state.workbench.rail_mode = RailMode::Teams;
        state.select_in_teams(TeamsSelection::Agent(there), cx);
    });

    let destination = fixture
        .state
        .read_with(cx, |state, cx| state.current_destination(cx))
        .expect("a selection on the canvas is a place");
    assert_eq!(
        destination.project, second,
        "the link names the project that owns the selected agent, not the one on screen"
    );
    assert_ne!(destination.project, fixture.project);
    assert_eq!(
        destination.view,
        View::Teams {
            selection: TeamsSelection::Agent(there),
            tab: TeamsInspectorTab::Chat,
        }
    );

    let text = destination.to_string();
    let parsed: Destination = text.parse().unwrap_or_else(|_| panic!("{text} is a link"));
    assert_eq!(parsed.project, second);
    assert_eq!(parsed.view, destination.view, "the round trip is lossless");
}

/// **Two spans, two arrangements.** The window span's view is the window's own, beside the
/// per-project ones, so switching span hands back a different `TeamsView` rather than rewriting
/// the one that was up — a shared view would throw the other's layout away on every switch.
#[gpui::test]
fn each_span_keeps_its_own_arrangement(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    fixture.hold_a_second(cx);
    let here = AgentId::generate();
    fixture.started(an_agent(here, "here"), cx);

    fixture.state.update(cx, |state, cx| {
        let teams = state.teams_mut(cx).expect("the project span's view");
        teams.algo = Algo::Columns;
        teams.zoom = 0.5;
    });

    fixture.state.update(cx, |state, cx| {
        state.toggle_teams_span(cx);
        let teams = state.teams_mut(cx).expect("the window span's view");
        assert_eq!(
            teams.algo,
            Algo::default(),
            "the window span opens on its own arrangement, not the project's"
        );
        teams.algo = Algo::Radial;
        teams.zoom = 2.0;
    });

    fixture.state.update(cx, |state, cx| {
        state.toggle_teams_span(cx);
        let teams = state.teams(cx).expect("the project span's view, again");
        assert_eq!(teams.algo, Algo::Columns);
        assert_eq!(
            teams.zoom, 0.5,
            "the project's layout survived the round trip"
        );
    });

    fixture.state.update(cx, |state, cx| {
        state.toggle_teams_span(cx);
        let teams = state.teams(cx).expect("the window span's view, again");
        assert_eq!(teams.algo, Algo::Radial);
        assert_eq!(teams.zoom, 2.0);
    });
}

// ── `+ Add agent`: starting in a project the window is not pointed at ────────────

impl Fixture {
    /// Answer the agent-type list, as the embedded harness library's own would. The form refuses
    /// to start against a harness this machine does not have, so a start needs one listed.
    fn answer_agent_types(&self, cx: &mut TestAppContext) {
        self.host.send(
            To::Everyone,
            Message::AgentTypes {
                agent_types: vec![AgentTypeInfo {
                    id: "claude-code".to_string(),
                    label: "Claude Code".to_string(),
                    command: "claude".to_string(),
                    available: true,
                    chat: true,
                    acp: false,
                    modes: Vec::new(),
                    unattended_mode: None,
                    keeps_sessions: true,
                    quota: Default::default(),
                }],
            },
        );
        cx.run_until_parked();
    }

    /// Press `+ Add agent`, pick a project out of the menu it raises, and answer the form's one
    /// required question — the whole gesture, in the order the toolbar performs it.
    fn add_agent_in(&self, project: ProjectId, cx: &mut TestAppContext) {
        let projects = self
            .state
            .read_with(cx, |state, cx| state.window_projects(cx));
        let index = projects
            .iter()
            .position(|id| *id == project)
            .expect("the window holds the project the menu is picking");
        self._window
            .update(cx, |_, window, cx| {
                self.state.update(cx, |state, cx| {
                    state.open_teams_add_agent(window, cx);
                    state.pick_teams_add_agent(index, window, cx);
                    state.pick_new_agent_target(
                        Target::Harness {
                            agent_type: "claude-code".to_string(),
                            account: Some("work".to_string()),
                        },
                        window,
                        cx,
                    );
                });
            })
            .expect("the window is open");
        cx.run_until_parked();
    }
}

/// **The project the menu named is the project the start is sent against.** Every other way into
/// the New agent form aims at the window's active project; the Teams toolbar's is the one that
/// does not, because under the window span the canvas is about projects the window is not pointed
/// at — and filing the start against the active one would put the agent where the user was not
/// looking.
#[gpui::test]
fn add_agent_starts_the_conversation_in_the_project_the_menu_named(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let second = fixture.hold_a_second(cx);
    fixture.answer_agent_types(cx);
    fixture
        .state
        .update(cx, |state, cx| state.toggle_teams_span(cx));
    let _ = fixture.said();

    fixture.add_agent_in(second, cx);
    fixture
        ._window
        .update(cx, |_, window, cx| {
            fixture
                .state
                .update(cx, |state, cx| state.confirm_new_agent_form(window, cx));
        })
        .expect("the window is open");
    cx.run_until_parked();

    let started = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::StartConversation { project_id, .. } => Some(project_id),
            _ => None,
        })
        .expect("confirming the form asks the host to start a conversation");
    assert_eq!(
        started, second,
        "the start went to the project the menu named, not to the active one"
    );
    assert_ne!(started, fixture.project, "and the active one is not it");
    fixture.state.read_with(cx, |state, _| {
        assert_eq!(
            state.new_agent_project, None,
            "a finished start leaves no project on the aim for the next one to inherit"
        );
    });
}

/// **A cancelled start leaves nothing pointed anywhere.** The override is an aim, not an answer,
/// so dismissing the form drops it with the rest — otherwise the next start from any surface in
/// this window would land in a project nobody chose for it.
#[gpui::test]
fn cancelling_the_form_forgets_the_project_it_was_aimed_at(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let second = fixture.hold_a_second(cx);
    fixture.answer_agent_types(cx);

    fixture.add_agent_in(second, cx);
    fixture.state.read_with(cx, |state, _| {
        assert_eq!(
            state.new_agent_project,
            Some(second),
            "the pick is what the form is composing against"
        );
    });

    fixture
        .state
        .update(cx, |state, cx| state.close_new_agent(cx));
    cx.run_until_parked();
    let _ = fixture.said();

    fixture.state.read_with(cx, |state, _| {
        assert_eq!(state.new_agent_project, None, "cancelling dropped the aim");
    });

    // And the next start, raised the ordinary way, goes to the active project.
    fixture
        ._window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.open_new_agent(window, cx);
                state.pick_new_agent_target(
                    Target::Harness {
                        agent_type: "claude-code".to_string(),
                        account: Some("work".to_string()),
                    },
                    window,
                    cx,
                );
                state.confirm_new_agent_form(window, cx);
            });
        })
        .expect("the window is open");
    cx.run_until_parked();

    let started = fixture
        .said()
        .into_iter()
        .find_map(|message| match message {
            Message::StartConversation { project_id, .. } => Some(project_id),
            _ => None,
        })
        .expect("the next form started something");
    assert_eq!(
        started, fixture.project,
        "the cancelled aim did not leak into the start after it"
    );
}

/// **The window span's view has to be laid out, or it is not a canvas.** Every wire arm that
/// learns of an arrival lays the arriving project's own `TeamsView` out; the window span's view is
/// a second arrangement over a second set of cards, and nothing in those arms reaches it. Left
/// out, its `Layout` stays empty, `Layout::at` answers the same default offset for every record,
/// and the span draws one pile in the corner with each new arrival under the last.
///
/// The work arrives while the *project* span is up, which is the case that matters: the view is
/// the window's, not the screen's, and a span switched to afterwards must find the cards placed.
#[gpui::test]
fn work_arriving_lays_the_window_span_s_own_view_out(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let second = fixture.hold_a_second(cx);
    let here = AgentId::generate();
    let there = AgentId::generate();
    fixture.started(an_agent(here, "here"), cx);
    fixture.started_in(second, an_agent(there, "there"), cx);

    fixture
        .state
        .update(cx, |state, cx| state.toggle_teams_span(cx));

    let (a, b) = fixture.state.read_with(cx, |state, cx| {
        let work = state.teams_work(cx).expect("the merged projection");
        let teams = state.teams(cx).expect("the window span's view");
        (
            teams.at_id(&work, here).expect("this project's card"),
            teams.at_id(&work, there).expect("the other project's card"),
        )
    });
    assert_ne!(
        a, b,
        "two projects' cards are two places on the canvas, not one pile at the default offset"
    );
}

/// **A hand-over is within a project.** Under the window span the merged projection's tasks are
/// every held project's, so without a narrowing every project's container is a live drop target
/// for every card — and the drop would send an `AssignAgent` naming one project, its own agent and
/// another project's task, which the host has never heard of and refuses after the canvas has
/// already drawn the move.
///
/// The container never lights up for a foreign card, so nothing is sent and nothing is
/// re-anchored: the card stays exactly where it was let go of, because position is the interface's
/// own fact whatever the drop meant. A card dropped into its *own* project's container is the
/// control — the hand-over still happens, and it is the narrowing that is new rather than the drop.
#[gpui::test]
fn a_card_is_never_filed_into_another_project_s_task(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let second = fixture.hold_a_second(cx);
    let mine = fixture.a_task_in(fixture.project, "ours", cx);
    let theirs = fixture.a_task_in(second, "theirs", cx);

    let carried = AgentId::generate();
    let ours = AgentId::generate();
    let foreign = AgentId::generate();
    fixture.started(an_agent(carried, "carried"), cx);
    let mut on_ours = an_agent(ours, "on ours");
    on_ours.task = Some(mine);
    fixture.started(on_ours, cx);
    let mut on_theirs = an_agent(foreign, "on theirs");
    on_theirs.task = Some(theirs);
    fixture.started_in(second, on_theirs, cx);

    fixture.state.update(cx, |state, cx| {
        state.toggle_teams_span(cx);
        // The two containers put far enough apart that neither drop can be read as the other.
        // Pinned rather than left to the packer: where the arrangement happens to put two frames
        // is not what this test is about, and a card dropped in the overlap would prove nothing.
        let teams = state.teams_mut(cx).expect("the window span's view");
        teams.layout.place_task(mine, (0.0, 0.0));
        teams.layout.place_agent(ours, (0.0, 0.0));
        teams.layout.place_task(theirs, (2000.0, 0.0));
        teams.layout.place_agent(foreign, (0.0, 0.0));
    });

    // Where a card has to be let go of to land inside one container: its centre on the container's
    // centre, which is what `task_at` measures.
    let centre_of = |task: TaskId, cx: &mut TestAppContext| {
        fixture.state.read_with(cx, |state, cx| {
            let work = state.teams_work(cx).expect("the merged projection");
            let teams = state.teams(cx).expect("the window span's view");
            let (x, y, w, h) = teams
                .bounds_of(&work, task)
                .expect("the container is on the canvas");
            (
                x + w / 2.0 - CARD_WIDTH / 2.0,
                y + h / 2.0 - CARD_HEIGHT / 2.0,
            )
        })
    };

    // Drained here so the drop's own traffic is all that is read back.
    let _ = fixture.said();
    let over_theirs = centre_of(theirs, cx);
    // The whole gesture in one update: a frame between the pick-up and the drop settles a carry
    // with no live drag behind it, which is `settle_teams`'s own rule and would put the card down
    // before this can read what it was over.
    let over = fixture.state.update(cx, |state, cx| {
        state.start_teams_carry(TeamsHeld::Agent(carried), (0.0, 0.0), cx);
        state.move_teams_carry(over_theirs, over_theirs, cx);
        let over = state
            .teams(cx)
            .and_then(|teams| teams.carry.as_ref().map(|carry| carry.over));
        state.end_teams_carry(cx);
        over
    });
    assert_eq!(
        over,
        Some(None),
        "the foreign container did not light up under a card that cannot join it"
    );
    cx.run_until_parked();
    let said = fixture.said();
    assert!(
        !said
            .iter()
            .any(|message| matches!(message, Message::AssignAgent { .. })),
        "a foreign drop is not a hand-over and asks the host for nothing: {said:?}"
    );
    assert_eq!(
        fixture.state.read_with(cx, |state, cx| {
            let work = state.teams_work(cx).expect("the merged projection");
            state.teams(cx).unwrap().at_id(&work, carried)
        }),
        Some(over_theirs),
        "the card stayed where it was let go of rather than being re-anchored to a frame it \
         cannot join"
    );

    // The control: the same gesture over its own project's container is the hand-over it always
    // was.
    let over_ours = centre_of(mine, cx);
    fixture.state.update(cx, |state, cx| {
        state.start_teams_carry(TeamsHeld::Agent(carried), (0.0, 0.0), cx);
        state.move_teams_carry(over_ours, over_ours, cx);
        state.end_teams_carry(cx);
    });
    cx.run_until_parked();
    let said = fixture.said();
    assert!(
        said.iter().any(|message| matches!(
            message,
            Message::AssignAgent {
                project_id,
                agent_id,
                task_id: Some(task_id),
            } if *project_id == fixture.project && *agent_id == carried && *task_id == mine
        )),
        "a drop into its own project's container still files the card: {said:?}"
    );
}
