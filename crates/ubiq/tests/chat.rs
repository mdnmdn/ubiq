//! The IDE's chat surface: many tabs, each attached to a conversation the host owns, or to none.
//!
//! The claim under test is the one the design doc makes about every view — "a view is a
//! perspective on a run the host owns" — applied to the one surface that used to be single
//! instance. Two tabs coexist, attaching one never touches the other's draft, closing one never
//! touches the conversation it was looking at, and a slot a close frees comes back clean.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::Utc;
use gpui::{AppContext as _, Entity, TestAppContext, WindowHandle};
use gpui_component::Root;
use ubiq::app::{AppState, BusHub};
use ubiq::state::agents::{CHATS_MAX, COLUMNS_MAX};
use ubiq::state::dock::ChatId;
use ubiq::state::{RailMode, WindowRegistry, attach_choices, prefs};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

/// How long a drain of the host's inbox waits for one more message before calling it empty.
const PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

struct Fixture {
    state: Entity<AppState>,
    window: WindowHandle<Root>,
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
            window,
            host,
            project,
        }
    }

    /// The host says an agent is up, exactly as `StartConversation` is answered.
    fn started(&self, agent: WorkAgent, cx: &mut TestAppContext) {
        let session = WorkSession {
            id: agent.session,
            name: "the project".to_string(),
            branch: String::new(),
            worktree: false,
        };
        self.host.send(
            To::Everyone,
            Message::ConversationStarted {
                project_id: self.project,
                agent: Box::new(agent),
                session,
                accepts_input: true,
            },
        );
        cx.run_until_parked();
    }

    /// The host says a project's whole work, exactly as `ListWork` is answered.
    fn work_list(&self, agents: Vec<WorkAgent>, cx: &mut TestAppContext) {
        self.host.send(
            To::Everyone,
            Message::WorkList {
                project_id: self.project,
                sessions: Vec::new(),
                agents,
                tasks: Vec::new(),
            },
        );
        cx.run_until_parked();
    }

    /// The host says a conversation is gone for good, exactly as `EndConversation` is answered.
    fn deleted(&self, agent_id: AgentId, cx: &mut TestAppContext) {
        self.host
            .send(To::Everyone, Message::ConversationDeleted { agent_id });
        cx.run_until_parked();
    }

    /// Everything this window has said to the host, drained. The same helper the conversation
    /// tests use, for the one assertion that has to read what was written down rather than what
    /// the window holds.
    fn said(&self) -> Vec<Message> {
        let mut said = Vec::new();
        while let Ok(event) = self.host.recv_timeout(PATIENCE) {
            if let FromClient::Said { message, .. } = event {
                said.push(message);
            }
        }
        said
    }

    fn regions_open(&self, cx: &mut TestAppContext) -> (bool, bool, bool) {
        self.state.read_with(cx, |state, cx| state.regions_open(cx))
    }

    /// Every `ubiq.chat` leaf in the dock's own dump, wherever it sits in the tree — the panel
    /// tree's own answer to "is a tab actually there", independent of `OpenProject::chats`.
    fn chat_leaves(&self, cx: &mut TestAppContext) -> Vec<String> {
        let blob = self.state.read_with(cx, |state, cx| {
            serde_json::to_value(state.dock().read(cx).dump(cx)).expect("the dump serialises")
        });
        let mut keys = Vec::new();
        collect_chat_leaves(&blob, &mut keys);
        keys
    }
}

fn collect_chat_leaves(value: &serde_json::Value, into: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("panel_name").and_then(|name| name.as_str()) == Some("ubiq.chat")
                && let Some(id) = map
                    .get("info")
                    .and_then(|info| info.get("panel"))
                    .and_then(|payload| payload.get("chat"))
                    .and_then(|id| id.as_str())
            {
                into.push(id.to_string());
            }
            for child in map.values() {
                collect_chat_leaves(child, into);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_chat_leaves(item, into);
            }
        }
        _ => {}
    }
}

fn a_project() -> ProjectSnapshot {
    ProjectSnapshot {
        record: ProjectRecord {
            id: ProjectId::generate(),
            name: "ubiq".to_string(),
            path: "/tmp/ubiq".to_string(),
            colour: 0,
            custom_colour: None,
            storage: Default::default(),
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            mission_term: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
            lanes: Vec::new(),
            runs_on: None,
            initials: String::new(),
            definitions_use_global: true,
            definitions_allowed: Vec::new(),
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
        title: None,
        definition: None,
        activity: Activity::Ended,
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

/// A fresh project's window opens with one chat tab already there — the default arrangement's own
/// — attached to nothing, so `+` is never the only way to get one.
#[gpui::test]
fn a_fresh_project_opens_with_one_unattached_chat_tab(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let chats = fixture.state.read_with(cx, |state, cx| {
        state.open_project(cx).unwrap().chats.clone()
    });
    assert_eq!(chats.len(), 1, "expected exactly the default tab");
    assert_eq!(chats[0].attached, None);
    assert!(chats[0].slot >= COLUMNS_MAX);
}

/// Two chat tabs coexist, each with its own attachment and its own composer slot — the whole
/// point of turning the panel into tabs rather than one surface with a selection.
///
/// Two fresh starts already produce this without any manual attach: the first claims the
/// project's one default tab (`reveal_agent_for_mode`'s doing, same as any other IDE-mode
/// reveal), and the second, finding it taken, mints a tab of its own and attaches that instead.
#[gpui::test]
fn two_chat_tabs_coexist_with_different_attachments_and_slots(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let agent_a = AgentId::generate();
    let agent_b = AgentId::generate();
    fixture.started(an_agent(agent_a, "Claude Code"), cx);
    fixture.started(an_agent(agent_b, "Codex"), cx);

    let chats = fixture.state.read_with(cx, |state, cx| {
        state.open_project(cx).unwrap().chats.clone()
    });
    assert_eq!(chats.len(), 2, "each fresh start claims a tab of its own");
    let a = chats
        .iter()
        .find(|tab| tab.attached == Some(agent_a))
        .unwrap();
    let b = chats
        .iter()
        .find(|tab| tab.attached == Some(agent_b))
        .unwrap();
    assert_ne!(a.slot, b.slot, "each tab must own a composer of its own");
}

/// What is typed into one tab's composer must not turn up in another's — the same rule a column's
/// own draft follows, applied to the chat surface's own slots.
#[gpui::test]
fn typing_in_one_chat_tab_leaves_the_other_s_draft_alone(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let tab_a = fixture
        .state
        .read_with(cx, |state, cx| state.open_project(cx).unwrap().chats[0].id);
    fixture
        .state
        .update(cx, |state, cx| state.open_chat_tab_now(cx));
    cx.run_until_parked();

    let (slot_a, slot_b) = fixture.state.read_with(cx, |state, cx| {
        let chats = &state.open_project(cx).unwrap().chats;
        let a = chats.iter().find(|tab| tab.id == tab_a).unwrap().slot;
        let b = chats.iter().find(|tab| tab.id != tab_a).unwrap().slot;
        (a, b)
    });

    fixture.state.update(cx, |state, cx| {
        state
            .agents_mut(cx)
            .unwrap()
            .set_draft(slot_a, "hello from A".to_string());
    });

    fixture.state.read_with(cx, |state, cx| {
        let agents = state.agents(cx).unwrap();
        assert_eq!(agents.draft(slot_a), "hello from A");
        assert_eq!(agents.draft(slot_b), "", "B's draft must stay untouched");
    });
}

/// Closing the last chat tab is allowed — there is no last-tab guard anywhere in this tree — and
/// leaves no panel behind and nothing to panic on the next frame.
#[gpui::test]
fn closing_the_last_chat_tab_leaves_no_panel_and_panics_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let tab = fixture
        .state
        .read_with(cx, |state, cx| state.open_project(cx).unwrap().chats[0].id);

    // The gesture, not the dock's callback: `closed_chat_tab` is what fires *after* a leaf has
    // already gone, so calling it directly would assert against a dock nobody asked to remove
    // anything. `close_chat_tab` queues the edit the dock's own close button queues.
    fixture
        .state
        .update(cx, |state, cx| state.close_chat_tab(tab, cx));
    cx.run_until_parked();

    let chats = fixture.state.read_with(cx, |state, cx| {
        state.open_project(cx).unwrap().chats.clone()
    });
    assert!(chats.is_empty());
    assert!(
        fixture.chat_leaves(cx).is_empty(),
        "the panel should have gone with the tab"
    );
}

/// A slot a close frees is the next one `+` hands out, and it carries no draft — what was typed
/// at the old tab must not turn up addressed to the new one.
#[gpui::test]
fn a_slot_freed_by_a_close_is_reused_and_carries_no_draft(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let (tab, slot) = fixture.state.read_with(cx, |state, cx| {
        let tab = state.open_project(cx).unwrap().chats[0];
        (tab.id, tab.slot)
    });

    fixture.state.update(cx, |state, cx| {
        state
            .agents_mut(cx)
            .unwrap()
            .set_draft(slot, "half a thought".to_string());
    });
    fixture
        .state
        .update(cx, |state, cx| state.closed_chat_tab(tab, cx));
    fixture
        .state
        .update(cx, |state, cx| state.open_chat_tab_now(cx));
    cx.run_until_parked();

    let (reused_slot, draft) = fixture.state.read_with(cx, |state, cx| {
        let new_tab = state.open_project(cx).unwrap().chats[0];
        let draft = state.agents(cx).unwrap().draft(new_tab.slot).to_string();
        (new_tab.slot, draft)
    });
    assert_eq!(
        reused_slot, slot,
        "the freed slot should be handed back out"
    );
    assert_eq!(draft, "", "the new tab must not inherit the old draft");
}

/// A conversation attached to tab A draws disabled in tab B's picker, and stays selectable in
/// A's own — exclusivity is per chat tab, and a tab's own current pick is never the row it
/// disables.
#[test]
fn a_conversation_disables_in_another_tab_s_picker_but_not_its_own() {
    use ubiq::state::ChatTab;

    let a = ChatId::generate();
    let b = ChatId::generate();
    let agent = AgentId::generate();
    let chats = [
        ChatTab {
            id: a,
            slot: COLUMNS_MAX,
            attached: Some(agent),
            picker_open: false,
        },
        ChatTab {
            id: b,
            slot: COLUMNS_MAX + 1,
            attached: None,
            picker_open: false,
        },
    ];
    let agents = vec![an_agent(agent, "Claude Code")];

    let live = [agent];
    let shown: Vec<_> = chats.iter().filter_map(|tab| tab.attached).collect();
    let from_a = attach_choices(&agents, &live, &shown, Some(agent), "");
    assert_eq!(from_a.selected, Some(0), "A's own attachment stays picked");
    assert!(
        from_a.disabled.is_empty(),
        "a tab's own row is never the one it disables"
    );

    let from_b = attach_choices(&agents, &live, &shown, None, "");
    assert_eq!(from_b.selected, None);
    assert_eq!(
        from_b.disabled,
        vec![0],
        "attached elsewhere, so drawn disabled — never dropped"
    );
    assert_eq!(
        from_b.items.len(),
        1,
        "a disabled row is still drawn, not filtered out"
    );
}

/// Creating a chat has to bring the right region back, not only add the panel to a tree the user
/// cannot see. A fresh conversation started while the region is put away is exactly the titlebar's
/// own "new agent" shortcut, and a tab that lands in a closed region is a tab that did nothing.
#[gpui::test]
fn a_freshly_attached_chat_tab_reopens_a_closed_right_region(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    // A fresh project opens with the right region put away, and nothing but a gesture reopens it —
    // see `a_persistent_agent_claims_a_chat_tab_without_opening_the_region`.
    assert!(
        !fixture.regions_open(cx).2,
        "the right region starts closed"
    );

    fixture
        .state
        .update(cx, |state, cx| state.open_chat_tab_now(cx));
    cx.run_until_parked();

    assert!(
        fixture.regions_open(cx).2,
        "creating a chat must reopen the region that hosts it"
    );
    assert!(!fixture.chat_leaves(cx).is_empty(), "the tab is on screen");
}

/// A project with a persistent agent claims a chat tab for it, and **leaves the right region
/// exactly as it found it**. A closed right region stays closed until the user's own click opens
/// it; the agent's work arriving is not a gesture. The tab is still attached and still in the
/// tree, so opening the region later lands on the persistent conversation rather than a fresh
/// empty tab.
#[gpui::test]
fn a_persistent_agent_claims_a_chat_tab_without_opening_the_region(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert!(
        !fixture.regions_open(cx).2,
        "with nothing persistent, the right region starts closed"
    );

    let mut agent = an_agent(AgentId::generate(), "Claude Code");
    agent.persistent = true;
    let agent_id = agent.id;
    fixture.work_list(vec![agent], cx);

    assert!(
        !fixture.regions_open(cx).2,
        "a persistent agent must not open a region the user left closed"
    );
    assert!(
        !fixture.chat_leaves(cx).is_empty(),
        "the tab is in the tree all the same, waiting for the region to be opened"
    );
    let attached = fixture.state.read_with(cx, |state, cx| {
        state
            .open_project(cx)
            .unwrap()
            .chats
            .iter()
            .find_map(|tab| tab.attached)
    });
    assert_eq!(attached, Some(agent_id));
}

/// Deleting a conversation **closes** the tab that was looking at it, panel and all. Every other
/// view stays: the delete names one conversation, not the surface it happened to be drawn on.
///
/// A detached tab would be an empty panel left where a conversation used to be, which is the one
/// thing a delete must not leave behind — an empty tab is what a `+` produces on request.
#[gpui::test]
fn deleting_a_conversation_closes_the_chat_tab_that_was_looking_at_it(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let agent_id = AgentId::generate();
    let other_id = AgentId::generate();
    // The first fresh start claims the project's one default tab; a second fresh start, once
    // that tab is taken, claims one of its own the same way `reveal_agent_for_mode` mints one for
    // an IDE-mode reveal — see `two_chat_tabs_coexist_with_different_attachments_and_slots`. So
    // there are two real dock leaves without this test opening or attaching either by hand: one
    // to watch go, one the delete must not touch.
    fixture.started(an_agent(agent_id, "Claude Code"), cx);
    fixture.started(an_agent(other_id, "Codex"), cx);

    let (watching, other) = fixture.state.read_with(cx, |state, cx| {
        let chats = &state.open_project(cx).unwrap().chats;
        (
            chats
                .iter()
                .find(|tab| tab.attached == Some(agent_id))
                .unwrap()
                .id,
            chats
                .iter()
                .find(|tab| tab.attached == Some(other_id))
                .unwrap()
                .id,
        )
    });
    assert!(
        fixture.chat_leaves(cx).contains(&watching.to_string()),
        "the attached tab is on screen before the delete"
    );

    fixture.deleted(agent_id, cx);

    let (chats, has_conversation, listed) = fixture.state.read_with(cx, |state, cx| {
        let open = state.open_project(cx).unwrap();
        (
            open.chats.clone(),
            open.conversations.contains_key(&agent_id),
            open.work.agent(agent_id).is_some(),
        )
    });
    assert!(
        !chats.iter().any(|tab| tab.id == watching),
        "the tab looking at a deleted conversation must close, not detach"
    );
    assert_eq!(chats.len(), 1, "the project's other view is untouched");
    assert_eq!(chats[0].id, other);
    assert_eq!(chats[0].attached, Some(other_id));
    assert!(
        !fixture.chat_leaves(cx).contains(&watching.to_string()),
        "the dock panel goes with the tab, not just the ChatTab row"
    );
    assert!(
        !has_conversation,
        "the window's own copy of the conversation must go with the delete"
    );
    assert!(
        !listed,
        "and its work row too — the columns, the bench and every attach list read that one list"
    );
}

/// A conversation nobody marked persistent is not written down, because it will not survive: the
/// host keeps only the persistent rows and the boot sweep has already deleted the rest. Writing
/// one would revive a tab attached to an agent that no longer exists.
///
/// A persistent one still is — that is the whole of what the blob is for.
#[gpui::test]
fn only_a_persistent_attachment_is_remembered(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);

    let kept_id = AgentId::generate();
    let mut kept = an_agent(kept_id, "Claude Code");
    kept.persistent = true;
    let passing_id = AgentId::generate();
    fixture.started(kept, cx);
    fixture.started(an_agent(passing_id, "Codex"), cx);

    let first = fixture
        .state
        .read_with(cx, |state, cx| state.open_project(cx).unwrap().chats[0].id);
    let second = fixture
        .state
        .update(cx, |state, cx| state.open_chat_tab_now(cx))
        .expect("a second chat tab");
    cx.run_until_parked();
    fixture
        .state
        .update(cx, |state, cx| state.attach_chat(first, Some(kept_id), cx));
    fixture.state.update(cx, |state, cx| {
        state.attach_chat(second, Some(passing_id), cx)
    });

    fixture
        .state
        .update(cx, |state, cx| state.remember_view(cx));
    cx.run_until_parked();

    let remembered = fixture
        .said()
        .into_iter()
        .rev()
        .find_map(|message| match message {
            Message::SetPreferences { value, .. } => {
                prefs::decode::<prefs::ViewPrefs>(&value).map(|view| view.chats)
            }
            _ => None,
        })
        .expect("the window wrote its view down");

    assert_eq!(
        remembered,
        vec![kept_id.to_string()],
        "the persistent attachment is kept and the passing one is written as nothing"
    );
}

/// The fix for a reveal landing on a surface nothing draws: `AppState::reveal_agent_for_mode` is
/// what `Message::ConversationStarted`'s fallback arm and the hidden-agents chevron both call now,
/// in place of `reveal_agent`, which only ever changes agents-column membership — invisible in
/// every mode but `RailMode::AGENTS` (`ui/rail.rs`). The fixture opens in `RailMode::IDE`, the
/// default a fresh window lands in, so a fresh start here is exactly the New agent dialog's Start
/// button landing in the mode the two bug reports were filed against.
#[gpui::test]
fn a_fresh_start_in_ide_mode_lands_on_a_revealed_attached_chat_panel(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    assert_eq!(
        fixture
            .state
            .read_with(cx, |state, _cx| state.workbench.rail_mode),
        RailMode::IDE
    );
    assert!(
        !fixture.regions_open(cx).2,
        "a fresh project opens with the right region put away"
    );

    let id = AgentId::generate();
    fixture.started(an_agent(id, "Ship the release"), cx);

    let tab = fixture
        .state
        .read_with(cx, |state, cx| {
            state
                .open_project(cx)
                .unwrap()
                .chats
                .iter()
                .find(|tab| tab.attached == Some(id))
                .map(|tab| tab.id)
        })
        .expect("the fresh agent landed on an attached chat tab");

    assert!(
        fixture.regions_open(cx).2,
        "landing on a chat panel must bring the right region back, the same as any other gesture \
         that creates one"
    );
    assert!(
        fixture.chat_leaves(cx).contains(&tab.to_string()),
        "the attached tab is actually on screen, not only recorded in `OpenProject::chats`"
    );
    assert!(
        !fixture
            .state
            .read_with(cx, |state, cx| state.agents(cx).unwrap().on_screen(id)),
        "IDE mode shows the agent through the chat panel, not the agents columns reveal_agent \
         would have used"
    );
}

/// Picking an agent that already has a chat tab — the attach menu's own row, or a second reveal of
/// the same conversation — must reuse that tab, never mint a second one pointed at the same
/// conversation.
#[gpui::test]
fn reveal_agent_for_mode_reuses_an_already_attached_tab(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let id = AgentId::generate();
    fixture.started(an_agent(id, "Ship the release"), cx);

    let before = fixture.state.read_with(cx, |state, cx| {
        state.open_project(cx).unwrap().chats.clone()
    });
    assert_eq!(
        before.len(),
        1,
        "the fresh start claimed the project's one default tab"
    );
    assert_eq!(before[0].attached, Some(id));

    fixture
        .state
        .update(cx, |state, cx| state.reveal_agent_for_mode(id, cx));
    cx.run_until_parked();

    let after = fixture.state.read_with(cx, |state, cx| {
        state.open_project(cx).unwrap().chats.clone()
    });
    assert_eq!(
        after.len(),
        1,
        "revealing an agent already on a chat tab must not mint a second one"
    );
    assert_eq!(after[0].id, before[0].id, "the same tab, not a new one");
    assert_eq!(after[0].attached, Some(id));
}

/// REPRO (bug 2): the same fresh start, one mode along. The user's rule is that the current mode
/// drives what is seen — so a start in GIT / TEAMS / KB / TASKS must land on a revealed chat panel
/// in *that* mode, exactly as it does in IDE.
#[gpui::test]
fn a_fresh_start_lands_on_a_revealed_chat_panel_in_every_project_mode(cx: &mut TestAppContext) {
    for mode in [
        RailMode::GIT,
        RailMode::TEAMS,
        RailMode::KB,
        RailMode::TASKS,
    ] {
        let fixture = Fixture::open(cx);
        fixture
            .state
            .update(cx, |state, cx| state.set_rail_mode(mode, cx));
        cx.run_until_parked();

        let id = AgentId::generate();
        fixture.started(an_agent(id, "Ship the release"), cx);

        let tab = fixture
            .state
            .read_with(cx, |state, cx| {
                state
                    .open_project(cx)
                    .unwrap()
                    .chats
                    .iter()
                    .find(|tab| tab.attached == Some(id))
                    .map(|tab| tab.id)
            })
            .unwrap_or_else(|| panic!("{mode:?}: the fresh agent landed on an attached chat tab"));

        assert!(
            fixture.regions_open(cx).2,
            "{mode:?}: landing on a chat panel must bring the right region back"
        );
        assert!(
            fixture.chat_leaves(cx).contains(&tab.to_string()),
            "{mode:?}: the attached tab is actually on screen"
        );
    }
}

/// The harness the New agent dialog offers in these tests — one available, conversable harness.
fn a_harness(fixture: &Fixture, cx: &mut TestAppContext) {
    fixture.host.send(
        To::Everyone,
        Message::AgentTypes {
            agent_types: vec![ubiq_proto::messages::AgentTypeInfo {
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
                shares_home: false,
                steers: false,
                device_login: false,
            }],
        },
    );
    cx.run_until_parked();
}

/// The titlebar's *New agent* button, the form answered with a bare harness, and Start — then the
/// host's answer. Returns the id the start minted.
fn start_from_the_titlebar(fixture: &Fixture, cx: &mut TestAppContext) -> AgentId {
    fixture
        .window
        .update(cx, |_, window, cx| {
            fixture.state.update(cx, |state, cx| {
                state.open_new_agent_direct(window, cx);
                state.pick_new_agent_target(
                    ubiq::state::new_agent::Target::Harness {
                        agent_type: "claude-code".to_string(),
                        account: None,
                    },
                    window,
                    cx,
                );
            })
        })
        .expect("the window is open");
    cx.run_until_parked();
    let id = fixture
        .state
        .update(cx, |state, cx| state.start_new_agent(cx))
        .expect("Start answered with an id");
    cx.run_until_parked();
    fixture.started(an_agent(id, "Ship the release"), cx);
    id
}

/// REPRO (bug 1): Start, from the control the user actually presses, has to land somewhere the
/// reader can see — in every project mode, and every time, not only for the first few.
#[gpui::test]
fn the_titlebars_start_lands_on_a_visible_chat_panel_in_every_project_mode(
    cx: &mut TestAppContext,
) {
    for mode in [
        RailMode::IDE,
        RailMode::GIT,
        RailMode::TEAMS,
        RailMode::KB,
        RailMode::TASKS,
    ] {
        let fixture = Fixture::open(cx);
        a_harness(&fixture, cx);
        fixture
            .state
            .update(cx, |state, cx| state.set_rail_mode(mode, cx));
        cx.run_until_parked();

        let id = start_from_the_titlebar(&fixture, cx);

        let tab = fixture
            .state
            .read_with(cx, |state, cx| {
                state
                    .open_project(cx)
                    .unwrap()
                    .chats
                    .iter()
                    .find(|tab| tab.attached == Some(id))
                    .map(|tab| tab.id)
            })
            .unwrap_or_else(|| panic!("{mode:?}: the start landed on an attached chat tab"));
        assert!(
            fixture.chat_leaves(cx).contains(&tab.to_string()),
            "{mode:?}: that tab is actually in the dock tree"
        );
        assert!(
            fixture.regions_open(cx).2,
            "{mode:?}: the right region is back"
        );
    }
}

/// REPRO (bug 1, the sticking half): the same Start, again and again. Nothing about pressing it a
/// ninth time makes it a different gesture, so it may not quietly stop working.
#[gpui::test]
fn the_titlebars_start_keeps_working_past_the_chat_slot_band(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    a_harness(&fixture, cx);

    for round in 1..=(COLUMNS_MAX + 2) {
        let id = start_from_the_titlebar(&fixture, cx);
        let tab = fixture
            .state
            .read_with(cx, |state, cx| {
                state
                    .open_project(cx)
                    .unwrap()
                    .chats
                    .iter()
                    .find(|tab| tab.attached == Some(id))
                    .map(|tab| tab.id)
            })
            .unwrap_or_else(|| panic!("start #{round} landed on no chat tab at all"));
        assert!(
            fixture.chat_leaves(cx).contains(&tab.to_string()),
            "start #{round}: its tab is in the dock tree"
        );
    }
}

/// REPRO (bug 2): picking a row in the attach control beside *New agent* has to show the agent in
/// **every** project mode, including when the project has already used up its chat slot band —
/// the state a window reaches after a few starts, and the one the old `reveal_agent` fallback
/// answered by putting the conversation in the agents columns, which only `RailMode::AGENTS`
/// draws. The user's rule is that the current mode drives what is seen; "nothing" is not a mode.
#[gpui::test]
fn attaching_shows_the_agent_in_every_project_mode_even_with_the_slot_band_full(
    cx: &mut TestAppContext,
) {
    for mode in [
        RailMode::IDE,
        RailMode::GIT,
        RailMode::TEAMS,
        RailMode::KB,
        RailMode::TASKS,
    ] {
        let fixture = Fixture::open(cx);
        fixture
            .state
            .update(cx, |state, cx| state.set_rail_mode(mode, cx));
        cx.run_until_parked();

        // Fill the band: one conversation per chat slot, each claiming a tab of its own.
        let mut agents = Vec::new();
        for n in 0..CHATS_MAX {
            let id = AgentId::generate();
            fixture.started(an_agent(id, &format!("agent {n}")), cx);
            agents.push(id);
        }
        assert_eq!(
            fixture
                .state
                .read_with(cx, |state, cx| state.open_project(cx).unwrap().chats.len()),
            CHATS_MAX,
            "{mode:?}: the band is full, so nothing can be minted"
        );

        // One more conversation, on no tab at all — exactly what the chevron lists.
        let hidden = AgentId::generate();
        fixture
            .state
            .update(cx, |state, cx| state.reveal_agent_for_mode(hidden, cx));
        cx.run_until_parked();

        let tab = fixture
            .state
            .read_with(cx, |state, cx| {
                state
                    .open_project(cx)
                    .unwrap()
                    .chats
                    .iter()
                    .find(|tab| tab.attached == Some(hidden))
                    .map(|tab| tab.id)
            })
            .unwrap_or_else(|| panic!("{mode:?}: the picked agent reached a chat tab"));
        assert!(
            fixture.chat_leaves(cx).contains(&tab.to_string()),
            "{mode:?}: and that tab is on screen"
        );
    }
}
