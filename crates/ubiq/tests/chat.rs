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
use ubiq::state::agents::COLUMNS_MAX;
use ubiq::state::dock::ChatId;
use ubiq::state::{WindowRegistry, attach_choices, prefs};
use ubiq_proto::bus::{self, FromClient, To};
use ubiq_proto::ids::ProjectId;
use ubiq_proto::messages::Message;
use ubiq_proto::projects::{ProjectHealth, ProjectRecord, ProjectSnapshot};
use ubiq_proto::work::{Activity, AgentId, WorkAgent, WorkSession};

/// How long a drain of the host's inbox waits for one more message before calling it empty.
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
            temporary: false,
            created_at: Utc::now(),
            last_opened_at: None,
            search_excludes: Vec::new(),
            index: None,
            tools: Vec::new(),
            managed_repos: Vec::new(),
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
        activity: Activity::Ended,
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
#[gpui::test]
fn two_chat_tabs_coexist_with_different_attachments_and_slots(cx: &mut TestAppContext) {
    let fixture = Fixture::open(cx);
    let agent_a = AgentId::generate();
    let agent_b = AgentId::generate();
    fixture.started(an_agent(agent_a, "Claude Code"), cx);
    fixture.started(an_agent(agent_b, "Codex"), cx);

    let tab_a = fixture
        .state
        .read_with(cx, |state, cx| state.open_project(cx).unwrap().chats[0].id);
    fixture
        .state
        .update(cx, |state, cx| state.attach_chat(tab_a, Some(agent_a), cx));
    fixture
        .state
        .update(cx, |state, cx| state.open_chat_tab_now(cx));
    cx.run_until_parked();

    let tab_b = fixture.state.read_with(cx, |state, cx| {
        state
            .open_project(cx)
            .unwrap()
            .chats
            .iter()
            .find(|tab| tab.id != tab_a)
            .expect("the second tab exists")
            .id
    });
    fixture
        .state
        .update(cx, |state, cx| state.attach_chat(tab_b, Some(agent_b), cx));

    let chats = fixture.state.read_with(cx, |state, cx| {
        state.open_project(cx).unwrap().chats.clone()
    });
    assert_eq!(chats.len(), 2);
    let a = chats.iter().find(|tab| tab.id == tab_a).unwrap();
    let b = chats.iter().find(|tab| tab.id == tab_b).unwrap();
    assert_eq!(a.attached, Some(agent_a));
    assert_eq!(b.attached, Some(agent_b));
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

    let shown: Vec<_> = chats.iter().filter_map(|tab| tab.attached).collect();
    let from_a = attach_choices(&agents, &shown, Some(agent), "");
    assert_eq!(from_a.selected, Some(0), "A's own attachment stays picked");
    assert!(
        from_a.disabled.is_empty(),
        "a tab's own row is never the one it disables"
    );

    let from_b = attach_choices(&agents, &shown, None, "");
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
    // A fresh project with nothing persistent opens with the right region put away — see
    // `a_persistent_agent_opens_its_chat_on_entry` for the one exception.
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

/// A project with a persistent agent opens showing its chat — the one exception to a fresh
/// project opening with the right region closed.
#[gpui::test]
fn a_persistent_agent_opens_its_chat_on_entry(cx: &mut TestAppContext) {
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
        fixture.regions_open(cx).2,
        "a persistent agent's tab is the one thing that opens the region on its own"
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
    fixture.started(an_agent(agent_id, "Claude Code"), cx);

    // A tab of its own, opened the way the strip's `+` opens one, so there is a real dock leaf to
    // watch go — and the project's default tab stays behind as the view the delete must not touch.
    let watching = fixture
        .state
        .update(cx, |state, cx| state.open_chat_tab_now(cx))
        .expect("a second chat tab");
    cx.run_until_parked();
    fixture.state.update(cx, |state, cx| {
        state.attach_chat(watching, Some(agent_id), cx)
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
    assert_eq!(chats[0].attached, None);
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
