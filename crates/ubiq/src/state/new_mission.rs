//! The new-mission dialog's own state.
//!
//! One draft, one composed action: `Message::CreateMission` carrying a
//! [`ubiq_proto::mission::NewMission`]. Nothing here sends a message; that is
//! `AppState::start_new_mission`'s job, in `app/new_mission.rs`. This module only holds what was
//! typed and turns it into the message body.
//!
//! **The host does everything else** — creating the parent task, writing the brief onto it, saving
//! the initial plan as revision one and launching the planner. The dialog collects.

use ubiq_proto::ids::TaskId;
use ubiq_proto::mission::{Driver, InitialPlan, MissionSettings, NewMission};

/// Where the initial plan comes from: nothing, a file picked through the attachment picker, or
/// text pasted straight in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PlanSource {
    #[default]
    None,
    /// An attachment target: a project-relative path or `kb:{source}:{path}`.
    File(String),
    /// The text typed or pasted into the dialog's plan textarea.
    Paste,
}

/// Every answer the dialog holds, while it is up.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NewMissionForm {
    pub title: String,
    /// The **requirements**, in markdown — the parent task's `description`.
    pub description: String,
    /// Project paths and `kb:` addresses, exactly as a task's own attachment list holds them.
    pub attachments: Vec<String>,
    /// Existing tasks that become the mission's subtasks.
    pub include: Vec<TaskId>,
    pub plan_source: PlanSource,
    /// The pasted plan text, mirrored from the dialog's textarea.
    pub plan_text: String,
    pub driver: Driver,
    /// Planner and worker, by agent-definition id. `None` is the project's default.
    pub planner: Option<String>,
    pub worker: Option<String>,
    pub parallel: usize,
    pub approve: bool,
    /// What is typed into the include-tasks picker's own filter field.
    pub task_query: String,
    pub planner_open: bool,
    pub worker_open: bool,
    pub tasks_open: bool,
    /// Whether the *More* group is unfolded.
    pub more_open: bool,
}

impl NewMissionForm {
    pub fn new() -> Self {
        Self {
            parallel: 1,
            ..Self::default()
        }
    }

    /// Whether Start may be pressed: something to call the mission by.
    pub fn ready(&self) -> bool {
        !self.effective_title().is_empty()
    }

    /// What the mission is called: what was typed, or the description's first line when nothing
    /// was — a card is never called nothing.
    pub fn effective_title(&self) -> String {
        let typed = self.title.trim();
        if !typed.is_empty() {
            return typed.to_string();
        }
        self.description
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(|line| line.trim_start_matches('#').trim().to_string())
            .unwrap_or_default()
    }

    /// The message body. A paste with nothing pasted is no plan.
    pub fn to_new_mission(&self) -> NewMission {
        let initial_plan = match &self.plan_source {
            PlanSource::None => InitialPlan::None,
            PlanSource::File(target) => InitialPlan::File {
                target: target.clone(),
            },
            PlanSource::Paste if self.plan_text.trim().is_empty() => InitialPlan::None,
            PlanSource::Paste => InitialPlan::Paste {
                body: self.plan_text.clone(),
            },
        };
        NewMission {
            title: self.effective_title(),
            description: self.description.clone(),
            attachments: self.attachments.clone(),
            include: self.include.clone(),
            initial_plan,
            settings: MissionSettings {
                driver: self.driver,
                planner: self.planner.clone(),
                worker: self.worker.clone(),
                parallel: self.parallel.max(1),
                approve: self.approve,
                planner_agent: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_needs_a_name() {
        let mut form = NewMissionForm::new();
        assert!(!form.ready());
        form.title = "Ship v2".to_string();
        assert!(form.ready());
        form.title = "   ".to_string();
        assert!(!form.ready());
    }

    #[test]
    fn a_blank_title_is_taken_from_the_requirements() {
        let mut form = NewMissionForm::new();
        form.description = "\n# Payment retries\n\nThey fail twice.".to_string();
        assert_eq!(form.effective_title(), "Payment retries");
    }

    #[test]
    fn the_plan_source_picks_the_initial_plan() {
        let mut form = NewMissionForm::new();
        form.title = "x".into();
        assert_eq!(form.to_new_mission().initial_plan, InitialPlan::None);
        form.plan_source = PlanSource::Paste;
        assert_eq!(form.to_new_mission().initial_plan, InitialPlan::None);
        form.plan_text = "# Plan".into();
        assert!(matches!(
            form.to_new_mission().initial_plan,
            InitialPlan::Paste { .. }
        ));
        form.plan_source = PlanSource::File("wip/a.md".into());
        assert!(matches!(
            form.to_new_mission().initial_plan,
            InitialPlan::File { .. }
        ));
    }
}
