//! The new-mission dialog's mutators, and the one message behind Start.
//!
//! **Start sends one `CreateMission`.** The host creates the parent task, writes the brief onto it,
//! saves the initial plan as revision one (reading a picked file itself) and asks this window,
//! through `MissionLaunch`, to start the planner. The dialog collects and sends; there is no
//! second step to wait for.

use super::*;
use crate::state::new_mission::{NewMissionForm, PlanSource};
use ubiq_proto::mission::Driver;

/// The most a *Parallel* stepper offers.
const MAX_PARALLEL: usize = 12;

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
        // The planner and worker pickers list definitions; asked fresh so one saved since the
        // window opened is offered without a restart.
        self.bus.send(Message::ListAgentDefinitions);
        cx.notify();
    }

    pub fn close_new_mission(&mut self, cx: &mut Context<Self>) {
        self.workbench.new_mission = None;
        cx.notify();
    }

    /// Shut whichever of the dialog's pickers is down. Escape's first rung over the dialog.
    pub fn close_new_mission_pickers(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(form) = self.workbench.new_mission.as_mut() else {
            return false;
        };
        let any = form.planner_open || form.worker_open || form.tasks_open;
        form.planner_open = false;
        form.worker_open = false;
        form.tasks_open = false;
        if any {
            cx.notify();
        }
        any
    }

    pub fn toggle_new_mission_planner_list(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.planner_open = !form.planner_open;
            form.worker_open = false;
        }
        cx.notify();
    }

    pub fn toggle_new_mission_worker_list(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.worker_open = !form.worker_open;
            form.planner_open = false;
        }
        cx.notify();
    }

    /// What the planner and worker pickers offer: *Project default*, then the project's agent
    /// definitions by id. Built here so the labels drawn and what a pick resolves to cannot
    /// disagree — a pick is read back by index off this same list.
    pub fn new_mission_definitions(&self, cx: &App) -> Vec<Option<String>> {
        let project = self.project(cx);
        std::iter::once(None)
            .chain(
                self.workbench
                    .settings
                    .definitions_in(project)
                    .into_iter()
                    .filter(|definition| !definition.disabled)
                    .map(|definition| Some(definition.id)),
            )
            .collect()
    }

    pub fn pick_new_mission_planner(&mut self, planner: Option<String>, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.planner = planner;
            form.planner_open = false;
        }
        cx.notify();
    }

    pub fn pick_new_mission_worker(&mut self, worker: Option<String>, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.worker = worker;
            form.worker_open = false;
        }
        cx.notify();
    }

    pub fn set_new_mission_driver(&mut self, driver: Driver, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.driver = driver;
        }
        cx.notify();
    }

    /// Nudge *Parallel* by one, within one and [`MAX_PARALLEL`].
    pub fn nudge_new_mission_parallel(&mut self, up: bool, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.parallel = match up {
                true => (form.parallel + 1).min(MAX_PARALLEL),
                false => form.parallel.saturating_sub(1).max(1),
            };
        }
        cx.notify();
    }

    pub fn toggle_new_mission_approve(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.approve = !form.approve;
        }
        cx.notify();
    }

    pub fn toggle_new_mission_more(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.more_open = !form.more_open;
        }
        cx.notify();
    }

    pub fn toggle_new_mission_task_list(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.tasks_open = !form.tasks_open;
            form.planner_open = false;
            form.worker_open = false;
        }
        cx.notify();
    }

    /// Include a task, or take it back off — the picker's own toggle, over a list the dialog holds
    /// rather than a record the host does (there is no mission yet to write to).
    pub fn toggle_new_mission_include(&mut self, task_id: TaskId, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            match form.include.iter().position(|held| *held == task_id) {
                Some(at) => {
                    form.include.remove(at);
                }
                None => form.include.push(task_id),
            }
        }
        cx.notify();
    }

    /// Hang files or knowledge-base documents on the mission being written. A target already
    /// there is not added twice.
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

    /// *Initial plan*: None or Paste. File… goes through the picker and lands in
    /// [`Self::set_new_mission_plan_file`].
    pub fn set_new_mission_plan_source(&mut self, source: PlanSource, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.plan_source = source;
        }
        cx.notify();
    }

    pub fn set_new_mission_plan_file(&mut self, target: String, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_mission.as_mut() {
            form.plan_source = PlanSource::File(target);
        }
        cx.notify();
    }

    /// What the include picker offers, in the order it draws them: the tasks already included
    /// first, then whatever the filter field matches. Capped, because a project's task list has no
    /// ceiling of its own. A mission is not offered itself, nor another mission's parent.
    pub fn new_mission_task_candidates(&self, cx: &App) -> Vec<TaskId> {
        const MAX: usize = 50;
        let Some(work) = self.work(cx) else {
            return Vec::new();
        };
        let Some(form) = self.workbench.new_mission.as_ref() else {
            return Vec::new();
        };
        let needle = form.task_query.trim().to_lowercase();
        let mut rows: Vec<TaskId> = form.include.clone();
        for task in &work.tasks {
            if rows.len() >= MAX {
                break;
            }
            if task.level == Some(ubiq_proto::work::Level::Mission) {
                continue;
            }
            if !rows.contains(&task.id)
                && crate::state::board::BoardState::text_matches(task, work, &needle)
            {
                rows.push(task.id);
            }
        }
        rows
    }

    /// Send the dialog: one `CreateMission`. Refused unless there is something to call the mission
    /// by.
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
        self.bus.send(Message::CreateMission {
            project_id,
            mission: Box::new(form.to_new_mission()),
        });
        self.workbench.new_mission = None;
        cx.notify();
    }
}
