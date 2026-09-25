//! What the window does with a mission: read one, and put its side panel in front of the reader.
//!
//! Every write is a [`ubiq_proto::messages::Message`] — the mission family's `SetMissionField`,
//! the phase moves that are a request rather than a field write, and the conversation family's
//! own `StartConversation` / `AssignAgent` / `UnloadConversation`.
//!
//! **The spawn policy lives here, and nowhere else** (M13, settled). The host relays a request and
//! records the answer; the *window* resolves the kind, mints the [`AgentId`], composes the launch
//! and decides — because the launch is the window's, and `MissionRecord::spawn_policy` is read on
//! this side of the bus only. [`AppState::mission_spawn_requested`] is the whole of the decision
//! and [`AppState::compose_mission_launch`] is the one composition every mission launch goes
//! through.

use gpui::{Context, Window};
use ubiq_proto::ids::{ProjectId, SpawnId, TaskId};
use ubiq_proto::messages::Message;
use ubiq_proto::mission::{
    Actor, AgentKind, MissionField, MissionRecord, PendingSpawn, Phase, SpawnOutcome, SpawnPolicy,
};
use ubiq_proto::work::AgentId;

use crate::state::PanelKind;
use crate::state::mission::{
    COORDINATOR_MCPS, KindTarget, MissionJournal, MissionLaunch, MissionMenuRow, MissionSpawnMenu,
    MissionSpawnRow, MissionTab, MissionView, SpawnPick, SpawnStage, WORKER_MCPS, worker_briefing,
};
use crate::state::work::WorkState;
use crate::state::workbench::MenuId;

use super::{AppState, PanelEdit};

/// One row of the `+` menu's *Missions* stage (§6.3): enough to draw the row without the caller
/// reaching back into `WorkProjection` and `OpenProject::missions` itself.
pub struct OpenMissionRow {
    pub id: TaskId,
    pub key: Option<String>,
    pub title: String,
    pub phase: Phase,
}

impl AppState {
    /// One mission of the project on screen, by the anchor task it is.
    pub fn mission(&self, task_id: TaskId, cx: &gpui::App) -> Option<&MissionRecord> {
        self.open_project(cx)
            .and_then(|open| open.missions.get(&task_id))
    }

    /// One mission held by **any** project this window has open, and whose it is.
    ///
    /// [`Self::mission`]'s answer is the project on screen's, which is the right one everywhere a
    /// mission surface is opened from that project. The Teams canvas under
    /// [`crate::state::TeamsSpan::Window`] draws every held project's work at once, so a fence on
    /// it may be a mission the active project has never heard of — and a lookup that asked only
    /// the active one would draw the fence with no phase, no coordinator and no hexagon.
    pub fn mission_anywhere(&self, task_id: TaskId) -> Option<&MissionRecord> {
        self.projects
            .values()
            .find_map(|open| open.missions.get(&task_id))
    }

    /// The project's missions eligible for the `+` menu's *Missions* stage (§6.3): every mission
    /// not `Completed` or `Abandoned`, most recently active first, narrowed by `query` (already
    /// trimmed and lowercased) against key and title.
    ///
    /// Excludes rather than dims: unlike the board toolbar's mission filter, there is no reason to
    /// offer opening a closed mission's panel from here — its side panel is still one click away
    /// through the board or the task it anchors.
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
            .filter(|record| !matches!(record.phase, Phase::Completed | Phase::Abandoned))
            .filter_map(|record| work.task(record.task_id).map(|task| (task, record)))
            .filter(|(task, _)| {
                query.is_empty()
                    || task.title.to_lowercase().contains(query)
                    || task
                        .key
                        .as_deref()
                        .is_some_and(|key| key.to_lowercase().contains(query))
            })
            .collect();
        // "Most recently active" reads off `updated_at` — the one activity signal a mission
        // record carries today. Nothing populates the *needs you* dot until S2/S3 (§6.3), so this
        // menu draws none rather than inventing one.
        rows.sort_by_key(|(_, record)| std::cmp::Reverse(record.updated_at));
        rows.into_iter()
            .map(|(task, record)| OpenMissionRow {
                id: task.id,
                key: task.key.clone(),
                title: task.title.clone(),
                phase: record.phase,
            })
            .collect()
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
        self.aim_mission_feedback(task_id, cx);
        cx.notify();
    }

    /// The full view, in whichever shape this screen calls for (§6.2).
    ///
    /// **In IDE mode `⤢` opens the document tab directly**, because that is the mode whose centre
    /// is a strip of documents and a modal there would cover the thing it belongs beside.
    /// Everywhere else it is the modal, which never disturbs the arrangement under it.
    pub fn open_mission_full(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        match self.workbench.is_ide() {
            true => self.open_mission_tab(task_id, cx),
            false => self.open_mission_modal(task_id, cx),
        }
    }

    /// Raise the full view as a modal. One at a time, like the plan dialog.
    pub fn open_mission_modal(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        self.workbench.mission = Some(task_id);
        self.aim_mission_feedback(task_id, cx);
        cx.notify();
    }

    pub fn close_mission_full(&mut self, cx: &mut Context<Self>) {
        self.workbench.mission = None;
        cx.notify();
    }

    /// The modal's *Open as tab*: the same view, moved into the centre region as a document.
    ///
    /// The modal closes, because the two shapes are one view and leaving both up would be the
    /// same mission twice. A `Reveal` rather than an `Open` for `open_mission_panel`'s reason.
    pub fn open_mission_tab(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        self.workbench.mission = None;
        self.pending_panels
            .push(PanelEdit::Reveal(PanelKind::MissionView(task_id)));
        self.aim_mission_feedback(task_id, cx);
        cx.notify();
    }

    /// Which tab the full view is on — the project's own `MissionView`, so the modal and the
    /// document tab move together.
    pub fn set_mission_tab(&mut self, tab: MissionTab, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.tab = tab;
        }
        cx.notify();
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
    /// selection is what moves — a task is edited here exactly as it is there, and the field being
    /// edited is the one state both surfaces read.
    ///
    /// Unlike [`AppState::select_task`] this reveals no panel: the detail is already on screen
    /// beside the graph, and raising the dock's Task panel over the full view would answer a click
    /// with a second copy of the same thing.
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
    /// already the one on (M26).
    pub fn toggle_mission_state_filter(&mut self, state: WorkState, cx: &mut Context<Self>) {
        if let Some(open) = self.open_project_mut(cx) {
            open.mission_view.toggle_state_filter(state);
        }
        cx.notify();
    }

    /// What clicking a segment of the side panel's progress bar does: the full view, on its Tasks
    /// tab, filtered to that state — one gesture, and the filter is why both surfaces share a
    /// view state at all.
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

    // ── moving the phase ────────────────────────────────────────────

    /// Move a mission's phase — **the user's own act, and so always a [`Message::SetPhase`]**.
    ///
    /// [`Message::RequestPhase`] is the *coordinator's* message: it carries no requester, and the
    /// host reads who asked off `MissionRecord::coordinator`, so a window sending one would put
    /// words in an agent's mouth. Every gesture on this side — a step on the stepper, Complete,
    /// Abandon, Confirm, Decline — is this one message, and the host tells the three acts apart by
    /// the phase named: the pending request's phase **confirms** it, the phase the mission is
    /// already in **declines** it, anything else is a free move. The plan gate is the only refusal,
    /// and it comes back as a `MissionError` rather than being second-guessed here.
    pub fn set_mission_phase(&mut self, task_id: TaskId, phase: Phase, cx: &mut Context<Self>) {
        let Some(project_id) = self.mission(task_id, cx).map(|record| record.project_id) else {
            return;
        };
        self.bus.send(Message::SetPhase {
            project_id,
            task_id,
            phase,
        });
        cx.notify();
    }

    /// Answer the pending request with a yes — which is naming the phase it asked for.
    pub fn confirm_mission_phase(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(phase) = self
            .mission(task_id, cx)
            .and_then(|record| record.pending_phase.as_ref())
            .map(|pending| pending.phase)
        else {
            return;
        };
        self.set_mission_phase(task_id, phase, cx);
    }

    /// Answer it with a no — which is naming the phase the mission is already in.
    pub fn decline_mission_phase(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(phase) = self.mission(task_id, cx).map(|record| record.phase) else {
            return;
        };
        self.set_mission_phase(task_id, phase, cx);
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
    /// follows. A row this build has nothing behind is drawn disabled and never reaches here.
    pub fn pick_mission_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((task_id, _)) = self.workbench.mission_menu else {
            return;
        };
        let Some(row) = MissionMenuRow::all().get(index).copied() else {
            return;
        };
        self.dismiss_mission_menu(cx);
        match row {
            // The board, with the anchor selected. **Not M27's mission filter** — that is the
            // board's own, and it is built beside this wave rather than in it.
            MissionMenuRow::OpenOnBoard => {
                self.set_rail_mode(crate::state::RailMode::Tasks, cx);
                self.select_task(task_id, cx);
            }
            // Both are phase moves, and a phase move by the user is a `SetPhase` — see
            // [`Self::set_mission_phase`].
            MissionMenuRow::Complete => self.set_mission_phase(task_id, Phase::Completed, cx),
            MissionMenuRow::Abandon => self.set_mission_phase(task_id, Phase::Abandoned, cx),
            // Every member's harness, unloaded. Not ended: `UnloadConversation` keeps the
            // transcript and the run directory, so each one resumes (M10's "detached, never
            // killed" applied to the whole roster at once).
            MissionMenuRow::PauseAll => self.pause_mission_agents(task_id, cx),
            MissionMenuRow::OpenOnTeams | MissionMenuRow::ExecutionMode => {}
        }
    }

    // ── who is on a mission ─────────────────────────────────────────

    /// Which project holds this mission's record. The panel is opened from surfaces that know the
    /// task and not the project, and a window may hold several.
    pub(super) fn project_of_mission(&self, task_id: TaskId) -> Option<ProjectId> {
        self.projects
            .iter()
            .find(|(_, open)| open.missions.contains_key(&task_id))
            .map(|(id, _)| *id)
    }

    /// The mission's live roster, in the order it was written.
    ///
    /// **The record, not the derivation.** `MissionRecord::roster` is the membership rule's own
    /// answer (M11) — assigned to the mission or a child, *or* spawned by a member — and the
    /// second half cannot be recomputed. A member that has left is history, not membership.
    pub fn mission_members(&self, task_id: TaskId) -> Vec<AgentId> {
        self.project_of_mission(task_id)
            .and_then(|id| self.projects.get(&id))
            .and_then(|open| open.missions.get(&task_id))
            .map(|record| {
                record
                    .roster
                    .iter()
                    .filter(|entry| entry.left_at.is_none())
                    .map(|entry| entry.agent)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Whether this window has a live conversation for the agent — what tells a roster row's
    /// `Chat` from its `Resume`.
    pub fn conversation_live(&self, agent: AgentId) -> bool {
        self.projects.values().any(|open| {
            open.conversations
                .get(&agent)
                .is_some_and(|conversation| conversation.running())
        })
    }

    // ── the roster's lifecycle actions (§6.2) ───────────────────────

    /// Put this agent's conversation in front of the reader — **Teams' own path**, reused rather
    /// than rewritten: a chat tab the project already holds is re-aimed rather than a second one
    /// grown.
    pub fn chat_with_mission_agent(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.open_teams_agent_panel(agent, cx);
    }

    /// Stop one member: its harness goes, its conversation stays. Never `EndConversation` — a
    /// mission surface does not delete anybody's transcript.
    pub fn stop_mission_agent(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.unload_agent(agent, cx);
    }

    /// Take an agent off the mission: it stops pointing at any of the mission's tasks, which is
    /// what membership is read from. Its harness keeps running and nothing it has said is lost.
    pub fn detach_mission_agent(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_agent(agent, cx) else {
            return;
        };
        self.bus.send(Message::AssignAgent {
            project_id,
            agent_id: agent,
            task_id: None,
        });
        cx.notify();
    }

    /// Adopt an agent already running here onto the mission, by pointing it at the anchor task.
    pub fn attach_mission_agent(
        &mut self,
        task_id: TaskId,
        agent: AgentId,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        self.bus.send(Message::AssignAgent {
            project_id,
            agent_id: agent,
            task_id: Some(task_id),
        });
        cx.notify();
    }

    /// Crown a member (M10's handoff).
    ///
    /// **One message.** The host owns the whole of the handoff — the outgoing coordinator is put
    /// back to worker, told in as many words that it no longer coordinates, and left running; the
    /// incoming one joins the roster if it was not already on it. A window that detached the old
    /// one itself would be killing something M10 says to leave alone.
    pub fn make_mission_coordinator(
        &mut self,
        task_id: TaskId,
        agent: AgentId,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        self.bus.send(Message::SetMissionField {
            project_id,
            task_id,
            field: MissionField::Coordinator(Some(agent)),
        });
        cx.notify();
    }

    /// Unload every member's harness — the `⋯`'s *Pause all agents*.
    pub fn pause_mission_agents(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        for agent in self.mission_members(task_id) {
            self.bus
                .send(Message::UnloadConversation { agent_id: agent });
        }
        cx.notify();
    }

    /// Start the coordinator's harness again, under the same id — what a roster row offers in
    /// place of `Chat` when nothing is loaded behind it.
    pub fn resume_mission_agent(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.resume_agent(agent, cx);
    }

    // ── the journal (M12) ───────────────────────────────────────────

    /// One mission's journal as this window has read it back.
    pub fn mission_journal(&self, task_id: TaskId) -> Option<&MissionJournal> {
        self.project_of_mission(task_id)
            .and_then(|id| self.projects.get(&id))
            .and_then(|open| open.mission_journals.get(&task_id))
    }

    /// Ask for the newest page, once. A surface cannot ask — it may not write state mid-render —
    /// so this is called where a mission is *put in front of the reader*, which is an event.
    pub fn load_mission_journal(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        let Some(open) = self.projects.get_mut(&project_id) else {
            return;
        };
        let held = open.mission_journals.entry(task_id).or_default();
        if held.asked {
            return;
        }
        held.asked = true;
        self.bus.send(Message::LoadJournal {
            project_id,
            task_id,
            before: None,
            limit: None,
        });
        cx.notify();
    }

    /// Another page back — the *Activity* tab's own step. The cursor is the oldest sequence held,
    /// which is why a page never skips or repeats a line.
    pub fn load_more_mission_journal(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        let before = self.mission_journal(task_id).and_then(|held| held.oldest());
        self.bus.send(Message::LoadJournal {
            project_id,
            task_id,
            before,
            limit: None,
        });
        cx.notify();
    }

    // ── feedback (M12) ──────────────────────────────────────────────

    /// Which mission a feedback composer's Enter is addressed at, and its journal asked for.
    fn aim_mission_feedback(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        self.workbench.feedback_mission = Some(task_id);
        self.load_mission_journal(task_id, cx);
    }

    /// Say something to the mission — the panel's *Feedback* line, live (M12).
    ///
    /// **Two halves, both of them delivery, and neither is invented here.**
    /// [`Message::SendToAgent`] is the durable one: the host journals it as the mission's feedback
    /// (that is what `read_feedback` reads) and puts it in the coordinator's row. The prompt is
    /// what reaches a model, and it goes out on the window's own path — so a turn already running
    /// puts it on `Conversation::queued`, the queue `wire`'s `ConversationUpdate` arm drains on
    /// `Run::Idle`, rather than on a second queue of this surface's own.
    ///
    /// **A coordinator that will not take it is said so, not pretended at.** The ACP bridge
    /// refuses a prompt mid-turn and a harness that has answered `accepts_input == false` takes no
    /// second turn at all; in both cases the line is still journaled, and the coordinator reads it
    /// with `read_feedback` when it next asks. The sentence says that rather than nothing.
    pub fn send_mission_feedback(
        &mut self,
        task_id: TaskId,
        text: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let text = text.trim().to_string();
        if text.is_empty() {
            return false;
        }
        let Some(project_id) = self.project_of_mission(task_id) else {
            return false;
        };
        let Some(coordinator) = self
            .projects
            .get(&project_id)
            .and_then(|open| open.missions.get(&task_id))
            .and_then(|record| record.coordinator)
        else {
            self.workbench.work_error = Some("this mission has no coordinator to tell".to_string());
            cx.notify();
            return false;
        };

        self.bus.send(Message::SendToAgent {
            project_id,
            agent_id: coordinator,
            text: text.clone(),
        });

        let held = self
            .projects
            .values()
            .find_map(|open| open.conversations.get(&coordinator))
            .map(|conversation| (conversation.run, conversation.accepts_input));
        match held {
            // Working: the interface's own queue, drained when the turn ends.
            Some((crate::state::conversation::Run::Working, _)) => {
                if let Some(open) = self
                    .projects
                    .values_mut()
                    .find(|open| open.conversations.contains_key(&coordinator))
                    && let Some(conversation) = open.conversations.get_mut(&coordinator)
                {
                    conversation.enqueue(text);
                }
            }
            Some((_, true)) => self.send_prompt(coordinator, text),
            // A harness that takes no further input. Journaled, and said so.
            Some((_, false)) => {
                self.workbench.work_error = Some(
                    "the coordinator is taking no more turns \u{2014} this is in the journal, and \
                     it reads feedback from there"
                        .to_string(),
                );
            }
            // Nothing loaded in this window: the journal is the whole of the delivery, and
            // `read_feedback` is how the coordinator gets it.
            None => {}
        }
        cx.notify();
        true
    }

    /// Send what is in one of the two feedback composers, and clear it if it went.
    ///
    /// `panel` picks which field — the side panel's or the *Activity* tab's. Which mission it is
    /// addressed at is `WorkbenchState::feedback_mission`, written when a mission surface was put
    /// in front of the reader, because a field's own Enter handler has nothing else to ask.
    pub fn submit_mission_feedback(
        &mut self,
        panel: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(task_id) = self.workbench.feedback_mission else {
            return;
        };
        let input = match panel {
            true => self.mission_feedback_input.clone(),
            false => self.mission_feedback_tab_input.clone(),
        };
        let text = input.read(cx).value().to_string();
        if self.send_mission_feedback(task_id, text, cx) {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    // ── the spawn policy (M13) ──────────────────────────────────────

    /// A member has asked the mission for another agent. **The window decides** — the host relays
    /// and records, and the policy lives here because the launch does (M13, settled).
    ///
    /// - `never` refuses with a sentence.
    /// - `auto` launches while the mission is under `spawn_limit` workers, and otherwise leaves
    ///   the row in *Needs you*: over the cap is a reason to ask, not a reason to refuse.
    /// - `ask` leaves the row — except for the scheduler's own request, which `PendingSpawn::auto`
    ///   marks: choosing auto execution *was* the consent (M23), so only `never` refuses it.
    pub(super) fn mission_spawn_requested(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        request: PendingSpawn,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self
            .projects
            .get(&project_id)
            .and_then(|open| open.missions.get(&task_id))
        else {
            return;
        };
        let policy = record.spawn_policy;
        let limit = record.spawn_limit;
        // The workers on it — the coordinator is not what a spawn cap is about.
        let workers = record
            .roster
            .iter()
            .filter(|entry| entry.left_at.is_none() && Some(entry.agent) != record.coordinator)
            .count();

        match policy {
            SpawnPolicy::Never => self.decline_spawn(
                project_id,
                task_id,
                request.id,
                request.kind.clone(),
                "this mission does not allow its agents to spawn other agents".to_string(),
                cx,
            ),
            SpawnPolicy::Auto if workers < limit => {
                self.launch_spawn(project_id, task_id, &request, cx);
            }
            SpawnPolicy::Ask if request.auto => {
                self.launch_spawn(project_id, task_id, &request, cx);
            }
            // The row stays on the record and is drawn in *Needs you* — see `ui::mission::panel`.
            SpawnPolicy::Auto | SpawnPolicy::Ask => cx.notify(),
        }
    }

    /// Allow one pending request — the *Needs you* row's own button, with whatever kind the user
    /// changed it to.
    pub fn allow_mission_spawn(&mut self, task_id: TaskId, id: SpawnId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        let Some(request) = self
            .projects
            .get(&project_id)
            .and_then(|open| open.missions.get(&task_id))
            .and_then(|record| record.pending_spawn(id))
            .cloned()
        else {
            return;
        };
        self.launch_spawn(project_id, task_id, &request, cx);
    }

    /// Refuse one — the user's own no. A decline is a real answer, which is why the requester is
    /// told a sentence rather than left waiting.
    pub fn refuse_mission_spawn(&mut self, task_id: TaskId, id: SpawnId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        let kind = self
            .projects
            .get(&project_id)
            .and_then(|open| open.missions.get(&task_id))
            .and_then(|record| record.pending_spawn(id))
            .map(|request| request.kind.clone())
            .unwrap_or_default();
        self.decline_spawn(
            project_id,
            task_id,
            id,
            kind,
            "the user declined this request".to_string(),
            cx,
        );
    }

    fn decline_spawn(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        id: SpawnId,
        _kind: String,
        reason: String,
        cx: &mut Context<Self>,
    ) {
        self.bus.send(Message::AnswerSpawn {
            project_id,
            task_id,
            request_id: id,
            outcome: SpawnOutcome::Declined { reason },
        });
        self.forget_spawn_pick(project_id, id);
        cx.notify();
    }

    /// Compose the launch one pending request asks for, send it, and answer the host.
    ///
    /// **Order is the contract's**: `StartConversation` first — the window mints the `AgentId`,
    /// so there is a real agent to name — then the briefing, then `AnswerSpawn` carrying the kind
    /// **actually used**, which is only knowable here because the user may have changed it.
    fn launch_spawn(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        request: &PendingSpawn,
        cx: &mut Context<Self>,
    ) {
        let pick = self
            .projects
            .get(&project_id)
            .and_then(|open| open.mission_view.spawn_picks.get(&request.id))
            .cloned()
            .unwrap_or(SpawnPick {
                kind: request.kind.clone(),
                profile: request.profile.clone(),
            });
        let spawned_by = match request.by {
            Actor::Agent(agent) => Some(agent),
            _ => None,
        };
        let briefing = worker_briefing(
            task_id,
            &pick.kind,
            request.task,
            &request.reason,
            &request.prompt,
        );

        let launch = MissionLaunch {
            kind: pick.kind.clone(),
            profile: pick.profile.clone(),
            coordinator: false,
            spawned_by,
            briefing,
            // An assignment clears `WorkAgent::parent` (`Work::assign_agent`), which is the spawn
            // link the Teams connector draws — so it is only sent where the requester actually
            // named a task. Membership without one is the roster's, written by the host from the
            // answer below (M11).
            assign_to: request.task,
        };
        match self.compose_mission_launch(project_id, launch, cx) {
            Ok(agent) => {
                self.bus.send(Message::AnswerSpawn {
                    project_id,
                    task_id,
                    request_id: request.id,
                    outcome: SpawnOutcome::Launched {
                        agent,
                        kind: pick.kind.clone(),
                    },
                });
                self.forget_spawn_pick(project_id, request.id);
            }
            Err(reason) => self.decline_spawn(
                project_id,
                task_id,
                request.id,
                pick.kind.clone(),
                reason,
                cx,
            ),
        }
        cx.notify();
    }

    fn forget_spawn_pick(&mut self, project_id: ProjectId, id: SpawnId) {
        if let Some(open) = self.projects.get_mut(&project_id) {
            open.mission_view.spawn_picks.remove(&id);
        }
    }

    /// Compose and send one mission launch — **the one composition**, shared by the spawn policy
    /// and by the panel's *Spawn ▾*.
    ///
    /// A kind resolves to a profile, and the kind's own four overrides outrank it exactly as a
    /// launch's picks already outrank a profile's. A kind that resolves to no harness at all is
    /// the one refusal, and the sentence it answers with is what the requester reads.
    fn compose_mission_launch(
        &mut self,
        project_id: ProjectId,
        launch: MissionLaunch,
        cx: &mut Context<Self>,
    ) -> Result<AgentId, String> {
        let kind_name = launch.kind.as_str();
        let kind: Option<AgentKind> = self
            .projects
            .get(&project_id)
            .and_then(|open| {
                open.missions
                    .values()
                    .find_map(|record| record.kind_named(kind_name))
            })
            .cloned();
        let profile_id = kind
            .as_ref()
            .and_then(|kind| kind.profile.clone())
            .or(launch.profile.clone());
        let profile = profile_id.and_then(|id| {
            self.workbench
                .settings
                .profiles_in(Some(project_id))
                .into_iter()
                .find(|profile| profile.id == id)
        });
        let agent_type = kind
            .as_ref()
            .and_then(|kind| kind.agent_type.clone())
            .or_else(|| profile.as_ref().map(|profile| profile.agent_type.clone()))
            .ok_or_else(|| {
                format!("no profile or harness resolves the kind \u{201c}{kind_name}\u{201d}")
            })?;

        let agent_id = AgentId::generate();
        self.bus.send(Message::StartConversation {
            agent_id,
            project_id,
            session_id: self.session,
            rel_path: None,
            agent_type,
            account: kind
                .as_ref()
                .and_then(|kind| kind.account.clone())
                .or_else(|| profile.as_ref().and_then(|profile| profile.account.clone())),
            profile: profile.as_ref().map(|profile| profile.id.clone()),
            model: kind
                .as_ref()
                .and_then(|kind| kind.model.clone())
                .or_else(|| profile.as_ref().and_then(|profile| profile.model.clone())),
            thinking: None,
            mode: kind
                .as_ref()
                .and_then(|kind| kind.permission_mode.clone())
                .or_else(|| profile.as_ref().and_then(|profile| profile.mode.clone())),
            mcps: match launch.coordinator {
                true => COORDINATOR_MCPS.iter().map(|it| it.to_string()).collect(),
                false => WORKER_MCPS.iter().map(|it| it.to_string()).collect(),
            },
            spawned_by: launch.spawned_by,
        });
        if let Some(task) = launch.assign_to {
            self.workbench
                .agent_assignments
                .insert(agent_id, (project_id, task));
        }
        self.send_prompt(agent_id, launch.briefing);
        cx.notify();
        Ok(agent_id)
    }

    /// The pick the user made on a pending row, or the request's own answer where they made none.
    pub fn mission_spawn_pick(&self, task_id: TaskId, request: &PendingSpawn) -> SpawnPick {
        self.project_of_mission(task_id)
            .and_then(|id| self.projects.get(&id))
            .and_then(|open| open.mission_view.spawn_picks.get(&request.id))
            .cloned()
            .unwrap_or(SpawnPick {
                kind: request.kind.clone(),
                profile: request.profile.clone(),
            })
    }

    // ── Spawn ▾ (§6.1) ──────────────────────────────────────────────

    /// What *Spawn ▾* offers, in the order it draws them — built here so the rows the menu draws
    /// and the row a click resolves against are the same list by construction.
    pub fn mission_spawn_rows(&self, task_id: TaskId) -> Vec<MissionSpawnRow> {
        let mut rows = vec![MissionSpawnRow::Coordinator];
        if let Some(record) = self
            .project_of_mission(task_id)
            .and_then(|id| self.projects.get(&id))
            .and_then(|open| open.missions.get(&task_id))
        {
            rows.extend(
                record
                    .agent_kinds
                    .iter()
                    .map(|kind| MissionSpawnRow::Kind(kind.name.clone())),
            );
        }
        rows.push(MissionSpawnRow::Any);
        rows.push(MissionSpawnRow::Attach);
        rows
    }

    /// Every agent running in this window's project that the mission has not already got.
    pub fn mission_attach_candidates(&self, task_id: TaskId, cx: &gpui::App) -> Vec<AgentId> {
        let held = self.mission_members(task_id);
        self.work(cx)
            .map(|work| {
                work.agents
                    .iter()
                    .map(|agent| agent.id)
                    .filter(|id| !held.contains(id))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn open_mission_spawn_menu(
        &mut self,
        task_id: TaskId,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        self.workbench.mission_spawn_menu = Some(MissionSpawnMenu {
            task: task_id,
            at,
            stage: SpawnStage::Kinds,
        });
        self.open_menu(MenuId::MissionSpawn, cx);
    }

    pub fn dismiss_mission_spawn_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.mission_spawn_menu = None;
        if self.workbench.open_menu == Some(MenuId::MissionSpawn) {
            self.workbench.open_menu = None;
        }
        cx.notify();
    }

    /// One row of *Spawn ▾*, by position and by stage.
    pub fn pick_mission_spawn_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.workbench.mission_spawn_menu else {
            return;
        };
        let task_id = menu.task;
        match menu.stage {
            SpawnStage::Attach => {
                let picked = self
                    .mission_attach_candidates(task_id, cx)
                    .get(index)
                    .copied();
                self.dismiss_mission_spawn_menu(cx);
                if let Some(agent) = picked {
                    self.attach_mission_agent(task_id, agent, cx);
                }
            }
            SpawnStage::Kinds => {
                let Some(row) = self.mission_spawn_rows(task_id).get(index).cloned() else {
                    return;
                };
                match row {
                    // The second stage, in place: which running agent to adopt.
                    MissionSpawnRow::Attach => {
                        self.workbench.mission_spawn_menu = Some(MissionSpawnMenu {
                            stage: SpawnStage::Attach,
                            ..menu
                        });
                        cx.notify();
                    }
                    MissionSpawnRow::Any => {
                        self.dismiss_mission_spawn_menu(cx);
                        self.spawn_any_mission_agent(task_id, window, cx);
                    }
                    MissionSpawnRow::Coordinator => {
                        self.dismiss_mission_spawn_menu(cx);
                        self.spawn_mission_coordinator(task_id, cx);
                    }
                    MissionSpawnRow::Kind(name) => {
                        self.dismiss_mission_spawn_menu(cx);
                        self.spawn_mission_kind(task_id, &name, cx);
                    }
                }
            }
        }
    }

    /// Launch one of the mission's kinds, on the user's own say-so. No request, so no
    /// `AnswerSpawn` and nobody to tell — and nothing to lose by assigning it to the mission,
    /// since there is no spawn link to clear.
    pub fn spawn_mission_kind(&mut self, task_id: TaskId, kind: &str, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        let launch = MissionLaunch {
            kind: kind.to_string(),
            briefing: worker_briefing(task_id, kind, None, "", ""),
            assign_to: Some(task_id),
            ..MissionLaunch::default()
        };
        if let Err(reason) = self.compose_mission_launch(project_id, launch, cx) {
            self.workbench.work_error = Some(reason);
            cx.notify();
        }
    }

    /// Launch a coordinator for a mission that has none, and crown it.
    ///
    /// The kind called `coordinator` where the table has one, and otherwise the first profile
    /// ticked *mission assistant* — the same list the new-mission dialog offers, so the two ways
    /// to put a coordinator on a mission resolve against one answer.
    pub fn spawn_mission_coordinator(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        let Some(project_id) = self.project_of_mission(task_id) else {
            return;
        };
        let named = self
            .projects
            .get(&project_id)
            .and_then(|open| open.missions.get(&task_id))
            .and_then(|record| record.kind_named("coordinator"))
            .map(|kind| kind.name.clone());
        let profile = match &named {
            Some(_) => None,
            None => {
                let profiles = self.workbench.settings.profiles_in(Some(project_id));
                match crate::state::new_mission::assistants(&profiles).first() {
                    Some(profile) => Some(profile.id.clone()),
                    None => {
                        self.workbench.work_error = Some(
                            "no profile is ticked \u{201c}mission assistant\u{201d}, so there is \
                             nothing to run a coordinator as"
                                .to_string(),
                        );
                        cx.notify();
                        return;
                    }
                }
            }
        };

        let (title, description) = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .map(|task| (task.title.clone(), task.description.clone()))
            .unwrap_or_default();
        let require_plan = self
            .projects
            .get(&project_id)
            .and_then(|open| open.missions.get(&task_id))
            .is_some_and(|record| record.require_plan);
        let briefing = crate::state::new_mission::mission_briefing(
            &title,
            &description,
            task_id,
            require_plan,
            false,
        );

        let launch = MissionLaunch {
            kind: named.unwrap_or_else(|| "coordinator".to_string()),
            profile,
            coordinator: true,
            briefing,
            assign_to: Some(task_id),
            ..MissionLaunch::default()
        };
        match self.compose_mission_launch(project_id, launch, cx) {
            Ok(agent) => self.make_mission_coordinator(task_id, agent, cx),
            Err(reason) => {
                self.workbench.work_error = Some(reason);
                cx.notify();
            }
        }
    }

    /// *Any agent* — the New agent form, every answer of it, pre-filled for this mission.
    ///
    /// **The form, not a second composition.** `assign_task_to_agent` is the board's own pre-fill
    /// and this is the mission's: the same open, with the mission's MCP set ticked and the
    /// briefing in the opening prompt.
    pub fn spawn_any_mission_agent(
        &mut self,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.assign_task_to_agent(task_id, window, cx);
        if let Some(form) = self.new_agent_form_mut() {
            for mcp in WORKER_MCPS {
                if !form.mcps.iter().any(|held| held == mcp) {
                    form.mcps.push(mcp.to_string());
                }
            }
        }
        let briefing = worker_briefing(task_id, "worker", Some(task_id), "", "");
        self.set_new_agent_prompt(&briefing, window, cx);
    }

    // ── the agent-kinds table (M13) ─────────────────────────────────

    /// Replace the whole table. `MissionField::AgentKinds` is a set write by design — a short list
    /// edited as one, where a delta would cost a second message and an ordering rule.
    pub fn set_mission_agent_kinds(
        &mut self,
        task_id: TaskId,
        kinds: Vec<AgentKind>,
        cx: &mut Context<Self>,
    ) {
        self.set_mission_field(task_id, MissionField::AgentKinds(kinds), cx);
    }

    /// One field of the record, written the one way (§6.2's Settings tab).
    ///
    /// **The window never writes the record and never journals.** It asks, the host writes, and
    /// `MissionChanged` comes back — which is also why switching execution mode is journaled by
    /// the host (M22) and not from here.
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

    fn mission_kinds(&self, task_id: TaskId) -> Vec<AgentKind> {
        self.project_of_mission(task_id)
            .and_then(|id| self.projects.get(&id))
            .and_then(|open| open.missions.get(&task_id))
            .map(|record| record.agent_kinds.clone())
            .unwrap_or_default()
    }

    /// Add a kind from a profile: the profile's name is the kind's, and the profile is what it
    /// resolves to. A name already in the table is not added twice.
    pub fn add_mission_agent_kind(
        &mut self,
        task_id: TaskId,
        profile_id: String,
        cx: &mut Context<Self>,
    ) {
        let mut kinds = self.mission_kinds(task_id);
        if kinds
            .iter()
            .any(|kind| kind.name.eq_ignore_ascii_case(&profile_id))
        {
            return;
        }
        kinds.push(AgentKind {
            name: profile_id.clone(),
            description: String::new(),
            profile: Some(profile_id),
            ..AgentKind::default()
        });
        self.set_mission_agent_kinds(task_id, kinds, cx);
    }

    pub fn remove_mission_agent_kind(
        &mut self,
        task_id: TaskId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let mut kinds = self.mission_kinds(task_id);
        if index >= kinds.len() {
            return;
        }
        kinds.remove(index);
        self.set_mission_agent_kinds(task_id, kinds, cx);
    }

    /// Point one row at another profile.
    pub fn set_mission_kind_profile(
        &mut self,
        task_id: TaskId,
        index: usize,
        profile_id: String,
        cx: &mut Context<Self>,
    ) {
        let mut kinds = self.mission_kinds(task_id);
        let Some(kind) = kinds.get_mut(index) else {
            return;
        };
        kind.profile = Some(profile_id);
        self.set_mission_agent_kinds(task_id, kinds, cx);
    }

    // A kind's affinity labels (M24) and its free-text name and description are **read** on the
    // table and not written: the scheduler that reads the labels is the wave beside this one, and
    // a free-text edit wants a prompt of its own. `G347` is that gap, and there is deliberately no
    // mutator for it here — a method nobody calls is a control the table does not have.

    // ── the kind picker ─────────────────────────────────────────────

    pub fn open_mission_kind_menu(
        &mut self,
        task_id: TaskId,
        target: KindTarget,
        at: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        self.workbench.mission_kind_menu = Some((task_id, target, at));
        self.open_menu(MenuId::MissionKind, cx);
    }

    pub fn dismiss_mission_kind_menu(&mut self, cx: &mut Context<Self>) {
        self.workbench.mission_kind_menu = None;
        if self.workbench.open_menu == Some(MenuId::MissionKind) {
            self.workbench.open_menu = None;
        }
        cx.notify();
    }

    /// What the kind picker offers, for the target it was opened on: a pending row may be changed
    /// to any of the mission's kinds *or* to a profile outright (the `custom` case), while a
    /// table row only ever resolves to a profile.
    pub fn mission_kind_picks(
        &self,
        task_id: TaskId,
        target: KindTarget,
    ) -> Vec<crate::state::mission::KindPick> {
        use crate::state::mission::KindPick;
        let mut rows: Vec<KindPick> = Vec::new();
        if matches!(target, KindTarget::Pending(_)) {
            rows.extend(
                self.mission_kinds(task_id)
                    .into_iter()
                    .map(|kind| KindPick::Kind(kind.name)),
            );
        }
        if let Some(project_id) = self.project_of_mission(task_id) {
            rows.extend(
                self.workbench
                    .settings
                    .profiles_in(Some(project_id))
                    .into_iter()
                    .map(|profile| KindPick::Profile(profile.id)),
            );
        }
        rows
    }

    pub fn pick_mission_kind_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        use crate::state::mission::KindPick;
        let Some((task_id, target, _)) = self.workbench.mission_kind_menu else {
            return;
        };
        let Some(pick) = self.mission_kind_picks(task_id, target).get(index).cloned() else {
            return;
        };
        self.dismiss_mission_kind_menu(cx);
        match (target, pick) {
            (KindTarget::Pending(id), pick) => {
                let (kind, profile) = match pick {
                    KindPick::Kind(name) => (name, None),
                    // A profile named outright is the `custom` kind — the one case the record's
                    // own vocabulary has a word for.
                    KindPick::Profile(id) => ("custom".to_string(), Some(id)),
                };
                if let Some(project_id) = self.project_of_mission(task_id)
                    && let Some(open) = self.projects.get_mut(&project_id)
                {
                    open.mission_view
                        .spawn_picks
                        .insert(id, SpawnPick { kind, profile });
                }
                cx.notify();
            }
            (KindTarget::Row(row), KindPick::Profile(id)) => {
                self.set_mission_kind_profile(task_id, row, id, cx)
            }
            (KindTarget::Add, KindPick::Profile(id)) => {
                self.add_mission_agent_kind(task_id, id, cx)
            }
            (KindTarget::Row(_) | KindTarget::Add, KindPick::Kind(_)) => {}
        }
    }
}
