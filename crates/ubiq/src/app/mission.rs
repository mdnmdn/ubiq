//! What the window does with a mission: read one, put its surfaces in front of the reader, and
//! turn every gesture into a message.
//!
//! A mission is its parent task plus the host's derived
//! [`ubiq_proto::mission::MissionSnapshot`]. Every write here is a
//! [`ubiq_proto::messages::Message`] of the mission family — `MissionControl` for the mission,
//! `MissionTask` for one subtask, `AddGuidance`, `SetMissionField` — and nothing is decided on this
//! side: the window asks, the host acts and `MissionChanged` comes back.
//!
//! **The one thing only a window can do is start a conversation**, so the host asks for it
//! (`MissionLaunch`, addressed to one client) and [`AppState::launch_for_mission`] answers with
//! `MissionLaunched`.

use gpui::{Context, Window};
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::mission::{
    Driver, LaunchOutcome, LaunchRequest, LaunchRole, MissionAction, MissionField, MissionSnapshot,
    MissionTaskAction, Phase,
};
use ubiq_proto::work::AgentId;

use crate::state::PanelKind;
use crate::state::mission::{
    MissionMenuRow, MissionNote, MissionSection, MissionTab, MissionView, NoteKind,
};
use crate::state::work::WorkState;
use crate::state::workbench::MenuId;

use super::{AppState, PanelEdit};

/// One row of the `+` menu's *Missions* stage: enough to draw the row without the caller reaching
/// back into `WorkProjection` and `OpenProject::missions` itself.
pub struct OpenMissionRow {
    pub id: TaskId,
    pub key: Option<String>,
    pub title: String,
    pub phase: Phase,
}

impl AppState {
    /// One mission of the project on screen, by the parent task it is.
    pub fn mission(&self, task_id: TaskId, cx: &gpui::App) -> Option<&MissionSnapshot> {
        self.open_project(cx)
            .and_then(|open| open.missions.get(&task_id))
    }

    /// One mission held by **any** project this window has open. The Teams canvas under the window
    /// span draws every held project's work at once, so a lookup that asked only the active one
    /// would miss a mission of another.
    pub fn mission_anywhere(&self, task_id: TaskId) -> Option<&MissionSnapshot> {
        self.projects
            .values()
            .find_map(|open| open.missions.get(&task_id))
    }

    /// The project's missions eligible for the `+` menu's *Missions* stage: every mission not
    /// `Done` or `Abandoned`, most recently touched first, narrowed by `query` (already trimmed and
    /// lowercased) against key and title.
    pub fn open_missions(&self, query: &str, cx: &gpui::App) -> Vec<OpenMissionRow> {
        let Some(work) = self.work(cx) else {
            return Vec::new();
        };
        let Some(open) = self.open_project(cx) else {
            return Vec::new();
        };
        let mut rows: Vec<_> = open
            .missions
            .values()
            .filter(|snapshot| !matches!(snapshot.phase, Phase::Done | Phase::Abandoned))
            .filter_map(|snapshot| work.task(snapshot.task_id).map(|task| (task, snapshot)))
            .filter(|(task, _)| {
                query.is_empty()
                    || task.title.to_lowercase().contains(query)
                    || task
                        .key
                        .as_deref()
                        .is_some_and(|key| key.to_lowercase().contains(query))
            })
            .collect();
        rows.sort_by_key(|(task, _)| std::cmp::Reverse(task.updated_at));
        rows.into_iter()
            .map(|(task, snapshot)| OpenMissionRow {
                id: task.id,
                key: task.key.clone(),
                title: task.title.clone(),
                phase: snapshot.phase,
            })
            .collect()
    }

    /// One mission's computed history as this window last asked for it, newest first. `None` until
    /// the first `MissionHistory` lands.
    pub fn mission_history(
        &self,
        task_id: TaskId,
    ) -> Option<&Vec<ubiq_proto::mission::HistoryEntry>> {
        self.projects
            .values()
            .find_map(|open| open.mission_history.get(&task_id))
    }

    /// How the project on screen's missions are being looked at.
    pub fn mission_view(&self, cx: &gpui::App) -> Option<&MissionView> {
        self.open_project(cx).map(|open| &open.mission_view)
    }

    /// Bring one mission's side panel on screen, opening it in its home region if this window has
    /// not got one yet.
    ///
    /// A `Reveal` rather than an `Open`, the chat tab's own path: a panel the dock already holds
    /// is brought forward rather than added twice, and the edit is queued because a dock change
    /// needs a `Window` and this is reached from places that have none.
    pub fn open_mission_panel(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::Mission(task_id)));
        self.aim_mission(task_id, cx);
        cx.notify();
    }

    /// The full view, in whichever shape this screen calls for.
    ///
    /// **In IDE mode `⤢` opens the document tab directly**, because that is the mode whose centre
    /// is a strip of documents and a modal there would cover the thing it belongs beside.
    pub fn open_mission_full(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        match self.workbench.is_ide() {
            true => self.open_mission_tab(task_id, cx),
            false => self.open_mission_modal(task_id, cx),
        }
    }

    /// Raise the full view as a modal. One at a time, like the plan dialog.
    pub fn open_mission_modal(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        self.workbench.mission = Some(task_id);
        self.aim_mission(task_id, cx);
        cx.notify();
    }

    pub fn close_mission_full(&mut self, cx: &mut Context<Self>) {
        self.workbench.mission = None;
        cx.notify();
    }

    /// The modal's *Open as tab*: the same view, moved into the centre region as a document.
    pub fn open_mission_tab(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        self.workbench.mission = None;
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::MissionView(task_id)));
        self.aim_mission(task_id, cx);
        cx.notify();
    }

    /// Which tab the full view is on — the project's own `MissionView`, so the modal and the
    /// document tab move together. The History tab asks for its list as it is put in front.
    pub fn set_mission_tab(&mut self, tab: MissionTab, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.tab = tab;
        }
        if tab == MissionTab::History
            && let Some(task_id) = self.shown_mission()
        {
            self.load_history(task_id, cx);
        }
        cx.notify();
    }

    /// The mission whose full view is up: the modal's, or the guidance composer's last aim.
    fn shown_mission(&self) -> Option<TaskId> {
        self.workbench.mission.or(self.workbench.guidance_mission)
    }

    /// Which shape the WBS is in — the graph, or the table of the same data.
    pub fn toggle_mission_wbs_table(&mut self, table: bool, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.wbs_table = table;
        }
        cx.notify();
    }

    /// How far the WBS graph is zoomed, on the Teams graph's own scale and step.
    pub fn zoom_mission_wbs(&mut self, delta: f32, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            let zoom = open.mission_view.zoom + delta;
            open.mission_view.zoom =
                zoom.clamp(crate::state::teams::ZOOM_MIN, crate::state::teams::ZOOM_MAX);
        }
        cx.notify();
    }

    /// Light the longest prerequisite chain, or put it out.
    pub fn toggle_mission_critical_path(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.wbs_critical = !open.mission_view.wbs_critical;
        }
        cx.notify();
    }

    /// Pick a node on the WBS: the task detail beside the graph is the board's own, so the board's
    /// selection is what moves.
    pub fn select_wbs_task(&mut self, task: TaskId, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.selected = Some(task);
        }
        if let Some(board) = self.board_mut(cx) {
            board.select(task);
        }
        cx.notify();
    }

    /// Show one work state in the Tasks tab and nothing else, or clear the filter when it is
    /// already the one on.
    pub fn toggle_mission_state_filter(&mut self, state: WorkState, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.toggle_state_filter(state);
        }
        cx.notify();
    }

    /// What clicking a segment of the side panel's progress bar does: the full view, on its Tasks
    /// tab, filtered to that state.
    pub fn open_mission_tasks(
        &mut self,
        task_id: TaskId,
        state: WorkState,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.tab = MissionTab::Tasks;
            open.mission_view.state_filter = Some(state);
        }
        self.open_mission_full(task_id, cx);
    }

    /// The Tasks tab with no state filter — *Open tasks* on a Needs-you row.
    pub fn open_mission_tasks_tab(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.tab = MissionTab::Tasks;
            open.mission_view.state_filter = None;
        }
        self.open_mission_full(task_id, cx);
    }

    /// Open a shut side-panel section or shut an open one, remembered against this mission's own
    /// parent task.
    pub fn toggle_mission_section(
        &mut self,
        task_id: TaskId,
        section: MissionSection,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.toggle_section(task_id, section);
        }
        cx.notify();
    }

    // ── the History tab's filters ───────────────────────────────────

    pub fn set_history_kind(
        &mut self,
        kind: Option<ubiq_proto::mission::HistoryKind>,
        cx: &mut Context<Self>,
    ) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.history.kind = kind;
        }
        cx.notify();
    }

    pub fn set_history_task(&mut self, task: Option<TaskId>, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.history.task = task;
        }
        cx.notify();
    }

    pub fn set_history_agent(&mut self, agent: Option<AgentId>, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.history.agent = agent;
        }
        cx.notify();
    }

    // ── the side panel's `⋯` ────────────────────────────────────────

    /// Open one mission's `⋯`, anchored where it was clicked. A trigger *opens*, never toggles.
    pub fn open_mission_menu(&mut self, task_id: TaskId, at: (f32, f32), cx: &mut Context<Self>) {
        self.workbench.mission_menu = Some((task_id, at));
        self.open_menu(MenuId::Mission, cx);
    }

    pub fn dismiss_mission_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.mission_menu = None;
        if self.workbench.open_menu == Some(MenuId::Mission) {
            self.workbench.open_menu = None;
        }
        cx.notify();
    }

    /// One row of that menu, by position — the rule every position-matched menu in this window
    /// follows. A row that means nothing in this phase is drawn disabled and never reaches here.
    pub fn pick_mission_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((task_id, _)) = self.workbench.mission_menu else {
            return;
        };
        let Some(row) = MissionMenuRow::all().get(index).copied() else {
            return;
        };
        let phase = self.mission_anywhere(task_id).map(|m| m.phase);
        self.dismiss_mission_menu(cx);
        match row {
            MissionMenuRow::OpenOnBoard => {
                self.set_rail_mode(crate::state::RailMode::TASKS, cx);
                self.select_task(task_id, cx);
            }
            MissionMenuRow::PauseResume => match phase {
                Some(Phase::Paused) => self.mission_control(task_id, MissionAction::Resume, cx),
                _ => self.mission_control(task_id, MissionAction::Pause, cx),
            },
            MissionMenuRow::Replan => self.open_mission_note(NoteKind::Replan(task_id), cx),
            MissionMenuRow::Complete => self.mission_control(task_id, MissionAction::Complete, cx),
            MissionMenuRow::Abandon => self.mission_control(task_id, MissionAction::Abandon, cx),
        }
    }

    // ── who is on a mission ─────────────────────────────────────────

    /// Which project holds this mission's snapshot. The panel is opened from surfaces that know
    /// the task and not the project, and a window may hold several.
    pub(super) fn project_of_mission(&self, task_id: TaskId) -> Option<ProjectId> {
        self.projects
            .iter()
            .find(|(_, open)| open.missions.contains_key(&task_id))
            .map(|(id, _)| *id)
    }

    /// The mission's live agents — the planner and every live worker or reviewer, as the host's
    /// snapshot lists them.
    pub fn mission_members(&self, task_id: TaskId) -> Vec<AgentId> {
        self.mission_anywhere(task_id)
            .map(|snapshot| snapshot.agents.clone())
            .unwrap_or_default()
    }

    /// Whether this window has a live conversation for the agent.
    pub fn conversation_live(&self, agent: AgentId) -> bool {
        self.projects.values().any(|open| {
            open.conversations
                .get(&agent)
                .is_some_and(|conversation| conversation.running())
        })
    }

    /// Put this agent's conversation in front of the reader — **Teams' own path**, reused rather
    /// than rewritten: a chat tab the project already holds is re-aimed rather than a second one
    /// grown.
    pub fn chat_with_mission_agent(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.open_teams_agent_panel(agent, cx);
    }

    // ── writes ──────────────────────────────────────────────────────

    /// One field of the mission's settings, written the one way.
    pub fn set_mission_field(
        &mut self,
        task_id: TaskId,
        field: MissionField,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        self.bus.send(Message::SetMissionField {
            project_id,
            task_id,
            field,
        });
        cx.notify();
    }

    /// The driver switch, usable mid-run.
    pub fn set_mission_driver(&mut self, task_id: TaskId, driver: Driver, cx: &mut Context<Self>) {
        self.set_mission_field(task_id, MissionField::Driver(driver), cx);
    }

    /// One gesture on the mission itself: approve the plan, start it, pause it…
    pub fn mission_control(
        &mut self,
        task_id: TaskId,
        action: MissionAction,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        self.bus.send(Message::MissionControl {
            project_id,
            task_id,
            action,
        });
        cx.notify();
    }

    /// One gesture on a subtask: confirm, retry, ▶, a review to add. `task_id` is the subtask.
    pub fn mission_task_action(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        action: MissionTaskAction,
        cx: &mut Context<Self>,
    ) {
        self.bus.send(Message::MissionTask {
            project_id,
            task_id,
            action,
        });
        cx.notify();
    }

    /// The Tasks tab's *Add task* in Breakdown: a new subtask of this mission. The host mints the
    /// id, so the parent link waits for `TaskCreated` (`BoardState::pending_parent`), and the new
    /// card is the board's selection, which the tab's detail pane shows for editing.
    pub fn add_mission_task(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        self.bus.send(Message::CreateTask {
            project_id,
            title: "New task".to_string(),
            session: None,
        });
        if let Some(board) = self.board_mut(cx) {
            board.awaiting_new = true;
            board.pending_parent = Some(task_id);
        }
        cx.notify();
    }

    /// Ask for the mission's computed history. Where a mission is *put in front of the reader*
    /// rather than on every frame, because a surface may not write state mid-render.
    pub fn load_history(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        self.bus.send(Message::LoadHistory {
            project_id,
            task_id,
        });
        cx.notify();
    }

    /// Which mission the guidance composer's Enter is addressed at; the History tab's list is
    /// asked for when it is the tab on screen.
    fn aim_mission(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        self.workbench.guidance_mission = Some(task_id);
        let on_history = self
            .mission_view(cx)
            .is_some_and(|view| view.tab == MissionTab::History);
        if on_history {
            self.load_history(task_id, cx);
        }
    }

    /// The *also send to running workers* box.
    pub fn toggle_guidance_to_running(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.guidance_to_running = !open.mission_view.guidance_to_running;
        }
        cx.notify();
    }

    /// Send a note for the whole mission: appended to `guidance.md` by the host, and — with the
    /// box ticked — delivered to every running worker as a message too.
    pub fn add_guidance(&mut self, task_id: TaskId, text: String, cx: &mut Context<Self>) -> bool {
        let text = text.trim().to_string();
        if text.is_empty() {
            return false;
        }
        let Some(project_id) = self.project_of_mission(task_id) else {
            return false;
        };
        let send_to_running = self
            .mission_view(cx)
            .is_some_and(|view| view.guidance_to_running);
        self.bus.send(Message::AddGuidance {
            project_id,
            task_id,
            text,
            send_to_running,
        });
        cx.notify();
        true
    }

    /// Send what is in the guidance composer, and clear it if it went. Which mission it is
    /// addressed at is `WorkbenchState::guidance_mission`.
    pub fn submit_mission_guidance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(task_id) = self.workbench.guidance_mission else {
            return;
        };
        let input = self.mission_guidance_input.clone();
        let text = input.read(cx).value().to_string();
        if self.add_guidance(task_id, text, cx) {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    // ── the note dialog ─────────────────────────────────────────────

    /// Raise the note dialog for one of the five places a mission asks for words.
    pub fn open_mission_note(&mut self, kind: NoteKind, cx: &mut Context<Self>) {
        self.workbench.mission_note = Some(MissionNote { kind });
        cx.notify();
    }

    pub fn close_mission_note(&mut self, cx: &mut Context<Self>) {
        self.workbench.mission_note = None;
        cx.notify();
    }

    /// Send the note: the textarea's text, to whichever of the five messages the dialog is for.
    /// Reject, fail and mission-reject need feedback; a replan or a redo may go without a note.
    pub fn send_mission_note(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = self.workbench.mission_note else {
            return;
        };
        let input = self.mission_note_input.clone();
        let text = input.read(cx).value().trim().to_string();
        let needs_text = matches!(
            note.kind,
            NoteKind::Reject(_) | NoteKind::Fail(_) | NoteKind::RejectMission(_)
        );
        if needs_text && text.is_empty() {
            return;
        }
        match note.kind {
            NoteKind::Reject(task) => {
                if let Some(project_id) = self.project(cx) {
                    self.mission_task_action(
                        project_id,
                        task,
                        MissionTaskAction::Reject { feedback: text },
                        cx,
                    );
                }
            }
            NoteKind::Fail(task) => {
                if let Some(project_id) = self.project(cx) {
                    self.mission_task_action(
                        project_id,
                        task,
                        MissionTaskAction::Fail { feedback: text },
                        cx,
                    );
                }
            }
            NoteKind::Replan(mission) => {
                self.mission_control(mission, MissionAction::Replan { note: text }, cx)
            }
            NoteKind::Redo(mission) => {
                self.mission_control(mission, MissionAction::RedoBreakdown { note: text }, cx)
            }
            NoteKind::RejectMission(mission) => {
                self.mission_control(mission, MissionAction::RejectMission { feedback: text }, cx)
            }
        }
        input.update(cx, |state, cx| state.set_value("", window, cx));
        self.workbench.mission_note = None;
        cx.notify();
    }

    // ── answering the host's launch request ─────────────────────────

    /// The host asks this window to start a conversation for a mission. Starts it through the
    /// ordinary path and answers with `MissionLaunched` — the agent's id on success, the reason on
    /// failure.
    pub(super) fn launch_for_mission(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        request: LaunchRequest,
        cx: &mut Context<Self>,
    ) {
        let outcome = match self.compose_mission_launch(project_id, &request) {
            Ok(agent) => LaunchOutcome::Launched(agent),
            Err(reason) => LaunchOutcome::Failed(reason),
        };
        self.bus.send(Message::MissionLaunched {
            project_id,
            task_id,
            id: request.id,
            outcome,
        });
        cx.notify();
    }

    /// Start (or revive) the conversation one request asks for, assign it to its task, mark it
    /// persistent if asked and send the host-composed prompt as its first turn.
    ///
    /// **Order is the contract's**: `StartConversation` first — the window mints the `AgentId` — then
    /// the assignment (parked until the `WorkAgent` exists), then `SetConversationPersistent`, then
    /// the prompt.
    fn compose_mission_launch(
        &mut self,
        project_id: ProjectId,
        request: &LaunchRequest,
    ) -> Result<AgentId, String> {
        // A planner resumed after a restart: the same conversation, revived, if this window knows
        // it. One it has never heard of falls through to a fresh start.
        if let Some(agent) = request.resume {
            let known = self
                .projects
                .get(&project_id)
                .is_some_and(|open| open.work.agent(agent).is_some());
            if known {
                if !self.conversation_live(agent) {
                    self.bus.send(Message::ReviveConversation {
                        source: agent,
                        agent_id: agent,
                        project_id,
                        session_id: self.session,
                    });
                }
                self.bus.send(Message::AssignAgent {
                    project_id,
                    agent_id: agent,
                    task_id: Some(request.task),
                });
                if request.persistent {
                    self.bus.send(Message::SetConversationPersistent {
                        agent_id: agent,
                        persistent: true,
                    });
                }
                self.send_prompt(agent, request.prompt.clone());
                return Ok(agent);
            }
        }

        let definitions = self.workbench.settings.definitions_in(Some(project_id));
        let definition = match request.definition.as_deref() {
            Some(id) => definitions.iter().find(|definition| definition.id == id),
            None => default_definition(&definitions, request.role),
        }
        .cloned()
        .ok_or_else(|| match request.definition.as_deref() {
            Some(id) => format!("the agent definition \u{201c}{id}\u{201d} is gone"),
            None => "no agent definition is tagged for this role — set one in settings".to_string(),
        })?;

        let agent_id = AgentId::generate();
        self.bus.send(Message::StartConversation {
            agent_id,
            project_id,
            session_id: self.session,
            rel_path: None,
            agent_type: definition.agent_type.clone(),
            account: definition.account.clone(),
            definition: Some(definition.id.clone()),
            model: definition.model.clone(),
            thinking: None,
            mode: definition.mode.clone(),
            mcps: request.mcps.clone(),
            skills: Vec::new(),
            grants: Vec::new(),
            remote: None,
            // The host's launch, not an agent's: nothing spawned this one.
            spawned_by: None,
        });
        // An assignment needs a `WorkAgent`, which exists only once the host has handled the
        // start — parked, and sent from the `ConversationStarted` arm.
        self.workbench
            .agent_assignments
            .insert(agent_id, (project_id, request.task));
        if request.persistent {
            self.bus.send(Message::SetConversationPersistent {
                agent_id,
                persistent: true,
            });
        }
        self.send_prompt(agent_id, request.prompt.clone());
        Ok(agent_id)
    }
}

/// The definition a role runs when the mission names none: the planner tag, then the old
/// coordinator tag, then anything ticked as a mission assistant; workers and reviewers take the
/// worker tag.
fn default_definition(
    definitions: &[ubiq_proto::messages::AgentDefinition],
    role: LaunchRole,
) -> Option<&ubiq_proto::messages::AgentDefinition> {
    use ubiq_proto::messages::{TAG_COORDINATOR, TAG_PLANNER, TAG_WORKER};
    let live = || definitions.iter().filter(|definition| !definition.disabled);
    match role {
        LaunchRole::Planner | LaunchRole::MissionReviewer => live()
            .find(|d| d.has_tag(TAG_PLANNER))
            .or_else(|| live().find(|d| d.has_tag(TAG_COORDINATOR)))
            .or_else(|| live().find(|d| d.mission_assistant == Some(true))),
        LaunchRole::Worker | LaunchRole::Reviewer => live().find(|d| d.has_tag(TAG_WORKER)),
    }
}
