//! The new-mission dialog's mutators, and the composed action behind Start: create the task,
//! promote it to a mission, and launch the chosen assistant on it — the flow
//! `_docs/wip/planning-system.md`'s "new-mission dialog and the assistant launch" describes.
//!
//! **Create-then-launch reuses the board's own precedent for a multi-step create.** `new_task` in
//! `app/board.rs` already carries a description across the wait for `CreateTask`'s id by parking it
//! in `BoardState::pending`, taken in the `TaskCreated` arm once the id lands
//! (`AppState::settle_new_task`). This module is the same shape one level up: `BoardState::pending_mission`
//! holds what a mission still owes once it has an id, and `settle_new_mission` below is what
//! `TaskCreated` calls once it does.
//!
//! Launching the assistant needs no second wait. `Message::StartConversation` carries an
//! `AgentId` the *window* mints (`AgentId::generate()`, the same convention `start_new_agent`
//! already uses), so the id is known before the message is even sent — the host adopts it rather
//! than answering with its own — and `PromptAgent` naming that same id right behind it composes
//! against the host's own "first prompt launches a pending conversation" contract
//! (`crates/ubiq-host/src/coordinator.rs`'s `launch_pending`), not against a race.

use super::*;
use crate::state::new_mission::{Coordinator, NewMissionForm, assistants, mission_briefing};
use ubiq_proto::messages::TaskField;
use ubiq_proto::mission::{AgentKind, MissionField, Phase};
use ubiq_proto::plan::DocumentHandle;
use ubiq_proto::work::Level;

impl AppState {
    /// Raise the dialog, titled with the project's own word for a mission.
    pub fn open_new_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.new_mission = Some(NewMissionForm::new());
        let title = self.new_mission_title_input.clone();
        title.update(cx, |state, cx| state.set_value("", window, cx));
        let description = self.new_mission_description_input.clone();
        description.update(cx, |state, cx| state.set_value("", window, cx));
        let plan = self.new_mission_plan_input.clone();
        plan.update(cx, |state, cx| state.set_value("", window, cx));
        let query = self.new_mission_task_query.clone();
        query.update(cx, |state, cx| state.set_value("", window, cx));
        title.update(cx, |state, cx| state.focus(window, cx));
        // The assistant picker filters this list itself; asked fresh so a profile ticked
        // `mission assistant` since the window opened is offered without a restart.
        self.bus.send(Message::ListProfiles);
        cx.notify();
    }

    pub fn close_new_mission(&mut self, cx: &mut Context<Self>) {
        self.workbench.new_mission = None;
        cx.notify();
    }

    pub fn toggle_new_mission_require_plan(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.require_plan = !form.require_plan;
        }
        cx.notify();
    }

    pub fn toggle_new_mission_assistant_list(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.open = !form.open;
        }
        cx.notify();
    }

    /// Pick who runs the mission — one role, either shape (M10).
    pub fn pick_new_mission_coordinator(
        &mut self,
        coordinator: Coordinator,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.coordinator = Some(coordinator);
            form.open = false;
        }
        cx.notify();
    }

    /// Everyone the coordinator picker offers, in the order it draws them: the profiles ticked as
    /// mission assistants, then every agent already running in this window (M10's *attach a
    /// running agent*).
    ///
    /// One list rather than two controls, because it is one question — and built here rather than
    /// in the row, so the picker's labels and what a pick resolves to cannot disagree.
    pub fn new_mission_coordinators(&self, cx: &App) -> Vec<(String, Coordinator)> {
        let project = self.project(cx);
        let profiles = self.workbench.settings.profiles_in(project);
        let mut rows: Vec<(String, Coordinator)> = assistants(&profiles)
            .into_iter()
            .map(|profile| (profile.id.clone(), Coordinator::Profile(profile.id.clone())))
            .collect();
        if let Some(work) = self.work(cx) {
            for agent in &work.agents {
                rows.push((
                    format!("{} \u{2014} running", self.agent_title(agent)),
                    Coordinator::Running(agent.id),
                ));
            }
        }
        rows
    }

    pub fn toggle_new_mission_task_list(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.tasks_open = !form.tasks_open;
        }
        cx.notify();
    }

    /// Link a task, or take it back off — the references picker's own toggle, over a list the
    /// dialog holds rather than a record the host does (there is no task yet to write to).
    pub fn toggle_new_mission_reference(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            match form.references.iter().position(|held| *held == task_id) {
                Some(at) => {
                    form.references.remove(at);
                }
                None => form.references.push(task_id),
            }
        }
        cx.notify();
    }

    /// Hang files or knowledge-base documents on the mission being written. A target already
    /// there is not added twice — `add_task_attachments`' rule, over the draft instead of the
    /// record.
    pub fn add_new_mission_attachments(&mut self, targets: Vec<String>, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            for target in targets {
                if !target.is_empty() && !form.attachments.contains(&target) {
                    form.attachments.push(target);
                }
            }
        }
        cx.notify();
    }

    pub fn remove_new_mission_attachment(&mut self, target: String, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.attachments.retain(|held| *held != target);
        }
        cx.notify();
    }

    /// What the linked-task picker offers, in the order it draws them: the tasks already linked
    /// first, then whatever the filter field matches — `BoardState::text_matches`, the same
    /// substring rule the board's own filter and the task panel's references picker use.
    ///
    /// **Built here rather than in the row** so the list the picker draws and the list its `on_pick`
    /// reads back by index are the same list by construction — the rule every position-matched
    /// menu in this window follows. Capped, because a project's task list has no ceiling of its own.
    pub fn new_mission_task_candidates(&self, cx: &App) -> Vec<TaskId> {
        const MAX: usize = 50;
        let Some(work) = self.work(cx) else {
            return Vec::new();
        };
        let Some(form) = self.workbench.new_mission.as_ref() else {
            return Vec::new();
        };
        let needle = form.task_query.trim().to_lowercase();
        let mut rows: Vec<TaskId> = form.references.clone();
        for task in &work.tasks {
            if rows.len() >= MAX {
                break;
            }
            if !rows.contains(&task.id)
                && crate::state::board::BoardState::text_matches(task, work, &needle)
            {
                rows.push(task.id);
            }
        }
        rows
    }

    pub fn toggle_new_mission_plan(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.plan_open = !form.plan_open;
        }
        cx.notify();
    }

    /// Send the dialog. Refused unless a title is typed and an assistant is chosen — the dialog's
    /// own `NewMissionForm::ready`.
    ///
    /// A `CreateTask` carries only a title and a session, so the rest — the promotion to
    /// `Level::Mission`, the description and the assistant launch — waits for the id
    /// `TaskCreated` answers with, parked on `BoardState::pending_mission` exactly the way an
    /// ordinary task's description waits on `BoardState::pending`.
    pub fn start_new_mission(&mut self, cx: &mut Context<Self>) {
        let Some(project_id) = self.project(cx) else {
            return;
        };
        let Some(form) = self.workbench.new_mission.clone() else {
            return;
        };
        if !form.ready() {
            return;
        }
        let Some(coordinator) = form.coordinator.clone() else {
            return;
        };
        let title = form.effective_title();
        let session = self.board(cx).and_then(|board| board.session);

        self.bus.send(Message::CreateTask {
            project_id,
            title: title.clone(),
            session,
        });
        if let Some(board) = self.board_mut(cx) {
            board.awaiting_new = true;
            board.pending_mission = Some(crate::state::board::PendingMission {
                title,
                description: form.description.clone(),
                attachments: form.attachments.clone(),
                references: form.references.clone(),
                require_plan: form.require_plan,
                plan_seed: form.plan_seed.clone(),
                coordinator,
            });
        }
        self.workbench.new_mission = None;
        cx.notify();
    }

    /// Finish a mission the host has just minted an id for: promote it, write the brief onto it,
    /// bring its record into being, seed its plan, and put a coordinator on it. Called from
    /// `TaskCreated`'s arm, the same way `settle_new_task` is — the id is the host's to mint, so
    /// none of this could be sent before it existed.
    ///
    /// **The brief is written with the messages a task already has** (M7): `UpdateTask` for the
    /// requirements, `SetTaskField` for the attachments and the linked tasks. Nothing new crosses
    /// the bus for it. `CreateMission` is the one message this wave adds to the sequence — the
    /// record exists in the same act as the card (M17), rather than being inferred the first time
    /// somebody lists the project's missions.
    pub(super) fn settle_new_mission(
        &mut self,
        project_id: ProjectId,
        task_id: TaskId,
        pending: crate::state::board::PendingMission,
    ) {
        self.bus.send(Message::SetTaskField {
            project_id,
            task_id,
            field: TaskField::Level(Some(Level::Mission)),
        });
        if !pending.description.trim().is_empty() {
            self.bus.send(Message::UpdateTask {
                project_id,
                task_id,
                title: None,
                description: Some(pending.description.clone()),
                priority: None,
            });
        }
        if !pending.attachments.is_empty() {
            self.bus.send(Message::SetTaskField {
                project_id,
                task_id,
                field: TaskField::Attachments(
                    pending
                        .attachments
                        .iter()
                        .map(|target| ubiq_proto::work::Attachment::new(target.clone()))
                        .collect(),
                ),
            });
        }
        if !pending.references.is_empty() {
            self.bus.send(Message::SetTaskField {
                project_id,
                task_id,
                field: TaskField::References(pending.references.clone()),
            });
        }

        self.bus.send(Message::CreateMission {
            project_id,
            task_id,
        });
        // **The agent-kinds table is seeded from the project's profiles** (M13). A mission whose
        // table is empty can only be spawned into by naming a profile outright, so the dialog
        // fills it with what the project already has — the same list the panel's picker offers,
        // so the two never disagree about what a kind may resolve to. Edited on the mission's
        // Agents tab afterwards; seeding is a starting point, not a decision.
        let kinds: Vec<AgentKind> = self
            .workbench
            .settings
            .profiles_in(Some(project_id))
            .into_iter()
            .map(|profile| AgentKind {
                name: profile.id.clone(),
                description: String::new(),
                profile: Some(profile.id),
                ..AgentKind::default()
            })
            .collect();
        if !kinds.is_empty() {
            self.bus.send(Message::SetMissionField {
                project_id,
                task_id,
                field: MissionField::AgentKinds(kinds),
            });
        }
        if pending.require_plan {
            self.bus.send(Message::SetMissionField {
                project_id,
                task_id,
                field: MissionField::RequirePlan(true),
            });
        }

        // A mission started *from* a plan is already past its requirements: the markdown is saved
        // as the plan's first revision, stamped `Human` by the host because only the interface
        // sends `SavePlan`, and the mission opens in Refining (M9). `0` is the watermark of a plan
        // nobody has written — which this one, a moment old, certainly is.
        let seeded = !pending.plan_seed.trim().is_empty();
        if seeded {
            self.bus.send(Message::SavePlan {
                doc: DocumentHandle::Plan {
                    project_id,
                    task_id,
                },
                body: pending.plan_seed.clone(),
                expected: 0,
            });
            self.bus.send(Message::SetPhase {
                project_id,
                task_id,
                phase: Phase::Refining,
            });
        }

        let briefing = mission_briefing(
            &pending.title,
            &pending.description,
            task_id,
            pending.require_plan,
            seeded,
        );

        // Either shape of coordinator ends the same way: one `AgentId` on the record, and the
        // briefing as that agent's next turn. An agent already running is adopted rather than
        // launched — it has a conversation, a context and possibly a turn in flight, and
        // `send_prompt` queues behind it.
        let agent_id = match pending.coordinator {
            Coordinator::Running(agent_id) => agent_id,
            Coordinator::Profile(id) => {
                // `profiles_in` rather than the global list alone: the picker offered this
                // project's own scoped profiles too (G331), so the launch has to resolve against
                // the same list or a project-scoped pick would vanish here.
                let Some(profile) = self
                    .workbench
                    .settings
                    .profiles_in(Some(project_id))
                    .into_iter()
                    .find(|profile| profile.id == id)
                else {
                    // The profile the dialog offered is gone by the time the mission exists — the
                    // mission itself still stands; there is just nobody left to launch.
                    return;
                };
                let agent_id = AgentId::generate();
                self.bus.send(Message::StartConversation {
                    agent_id,
                    project_id,
                    session_id: self.session,
                    rel_path: None,
                    agent_type: profile.agent_type.clone(),
                    account: profile.account.clone(),
                    profile: Some(profile.id.clone()),
                    model: None,
                    thinking: None,
                    mode: None,
                    // The coordinator's set, named once in `state::mission` — the panel's
                    // *Spawn ▾* composes a coordinator from the same list, and two copies of it
                    // is how one of the two ways to start a coordinator loses a server.
                    mcps: crate::state::mission::COORDINATOR_MCPS
                        .iter()
                        .map(|it| it.to_string())
                        .collect(),
                    // The dialog's own coordinator: the user asked for it, not an agent.
                    spawned_by: None,
                });
                agent_id
            }
        };

        self.bus.send(Message::SetMissionField {
            project_id,
            task_id,
            field: MissionField::Coordinator(Some(agent_id)),
        });
        self.send_prompt(agent_id, briefing);
    }
}
