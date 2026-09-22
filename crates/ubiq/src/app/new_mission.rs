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
use crate::state::new_mission::{NewMissionForm, mission_briefing};
use ubiq_proto::messages::TaskField;
use ubiq_proto::work::Level;

impl AppState {
    /// Raise the dialog, titled with the project's own word for a mission.
    pub fn open_new_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.new_mission = Some(NewMissionForm::new());
        let title = self.new_mission_title_input.clone();
        title.update(cx, |state, cx| state.set_value("", window, cx));
        let description = self.new_mission_description_input.clone();
        description.update(cx, |state, cx| state.set_value("", window, cx));
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

    pub fn pick_new_mission_assistant(&mut self, profile_id: String, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.assistant = Some(profile_id);
            form.open = false;
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
        let Some(assistant_profile) = form.assistant.clone() else {
            return;
        };
        let title = form.title.trim().to_string();
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
                require_plan: form.require_plan,
                assistant_profile,
            });
        }
        self.workbench.new_mission = None;
        cx.notify();
    }

    /// Finish a mission the host has just minted an id for: promote it, write its description,
    /// and start the chosen assistant on it. Called from `TaskCreated`'s arm, the same way
    /// `settle_new_task` is — the id is the host's to mint, so neither the promotion nor the
    /// launch could be sent before it existed.
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

        // `profiles_in` rather than the global list alone: the picker offered this project's own
        // scoped profiles too (G331), so the launch has to resolve against the same list or a
        // project-scoped pick would vanish here.
        let Some(profile) = self
            .workbench
            .settings
            .profiles_in(Some(project_id))
            .into_iter()
            .find(|profile| profile.id == pending.assistant_profile)
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
            mcps: vec!["manage-ubiq-tasks".to_string(), "ubiq-plan".to_string()],
        });

        let briefing = mission_briefing(
            &pending.title,
            &pending.description,
            task_id,
            pending.require_plan,
        );
        self.send_prompt(agent_id, briefing);
    }
}
