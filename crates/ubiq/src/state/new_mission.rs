//! The new-mission dialog's own state.
//!
//! §5's table, widened rather than replaced: title, the requirements editor, attachments, linked
//! tasks, the plan gate, the coordinator, and an optional plan seed — and one composed action:
//! create the task, write the brief onto it, promote it, bring its record into being and start the
//! coordinator. Nothing here sends a message; that is `AppState::start_new_mission`'s job, in
//! `app/new_mission.rs`. This module only holds what was typed and answers what the pickers offer.
//!
//! **The brief is the anchor task's own fields** (M7): the description, the attachments and the
//! linked tasks below are written with `UpdateTask` and `SetTaskField`, the messages a task
//! already has. Nothing new crosses the bus for them, and the full view's *Brief* reads them back
//! off the task rather than off the mission.

use ubiq_proto::ids::TaskId;
use ubiq_proto::messages::ProfileInfo;
use ubiq_proto::work::AgentId;

/// Who is to run the mission (M10 — coordinator and assistant are one role).
///
/// Two ways to name one agent, not two roles: a saved setup to launch, or an agent already running
/// in this window to adopt. They are exclusive by construction rather than by a rule written
/// somewhere — a mission has at most one coordinator at a time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Coordinator {
    /// A profile to launch, by [`ProfileInfo::id`].
    Profile(String),
    /// A conversation already running here, adopted as it is. Nothing is launched for it.
    Running(AgentId),
}

/// Every answer the dialog holds, while it is up.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NewMissionForm {
    pub title: String,
    /// The **requirements**, in markdown — the anchor task's `description` (M7).
    pub description: String,
    /// What the mission is about that is not prose: project paths and `kb:` addresses, exactly as
    /// a task's own attachment list holds them. Written as `TaskField::Attachments`.
    pub attachments: Vec<String>,
    /// The tasks this mission names — the anchor's `references` (M7).
    pub references: Vec<TaskId>,
    /// **The plan gate** (M5): with it on, Refining → In progress is the user's to pass and the
    /// host refuses it while the plan is empty or has open threads. Stored on the record as
    /// `MissionField::RequirePlan`, and said in the coordinator's briefing besides.
    pub require_plan: bool,
    /// The chosen coordinator. `None` until one is picked, and Start is refused until it is.
    pub coordinator: Option<Coordinator>,
    /// Markdown to seed the plan with. Empty is no plan; anything else is saved as the plan's
    /// first revision and starts the mission in `Refining` (M9).
    pub plan_seed: String,
    /// What is typed into the linked-task picker's own filter field.
    pub task_query: String,
    /// Whether the coordinator picker's list is down.
    pub open: bool,
    /// Whether the linked-task picker's list is down.
    pub tasks_open: bool,
    /// Whether the plan seed's editor is showing. Folded away by default: most missions start
    /// without one, and a second text area open on every mission is a dialog that reads as work.
    pub plan_open: bool,
}

impl NewMissionForm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether Start may be pressed: something to call it by, and somebody to run it.
    ///
    /// A blank title is allowed as long as there are requirements to take one from —
    /// [`Self::effective_title`] — exactly as a task drafted with only a description is.
    pub fn ready(&self) -> bool {
        !self.effective_title().is_empty() && self.coordinator.is_some()
    }

    /// What the mission is called: what was typed, or the description's first line when nothing
    /// was. The board's own draft rule, applied one level up — a card is never called nothing.
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

    /// The profile to launch, where the coordinator is one.
    pub fn profile(&self) -> Option<&str> {
        match self.coordinator.as_ref() {
            Some(Coordinator::Profile(id)) => Some(id),
            _ => None,
        }
    }

    /// The running agent to adopt, where the coordinator is one.
    pub fn running(&self) -> Option<AgentId> {
        match self.coordinator.as_ref() {
            Some(Coordinator::Running(id)) => Some(*id),
            _ => None,
        }
    }
}

/// What the coordinator picker's trigger says: the chosen row's label once one is picked, the
/// placeholder until then. Kept as one function so the trigger and the test agree by construction
/// — the bug this once was is a trigger that drew the placeholder unconditionally, ignoring what
/// the form held — the same idiom `kit::multi_label` follows for the multi-select picker.
///
/// `rows` is the picker's own list in its own order: the profiles fit to run one, then the agents
/// already running here (M10's *attach a running agent*).
pub fn coordinator_label(
    form: &NewMissionForm,
    rows: &[(String, Coordinator)],
    placeholder: &str,
) -> String {
    form.coordinator
        .as_ref()
        .and_then(|held| rows.iter().find(|(_, row)| row == held))
        .map(|(label, _)| label.clone())
        .unwrap_or_else(|| placeholder.to_string())
}

/// The profiles fit to run as a planning assistant, in the order the host listed them —
/// [`ProfileInfo::mission_assistant`] ticked. `None`/`Some(false)` are both "not an assistant",
/// the same reading every other filter over this field gives.
pub fn assistants(profiles: &[ProfileInfo]) -> Vec<&ProfileInfo> {
    profiles
        .iter()
        .filter(|profile| profile.mission_assistant == Some(true))
        .collect()
}

/// The coordinator's opening turn — the briefing `Message::PromptAgent` carries as the
/// conversation's first turn.
///
/// Carries the mission's title, description and id, names `ubiq-mission` as what the phase is
/// moved through (M16), and says what `require plan` now *is*: a gate the host enforces on
/// Refining → In progress (M5), not the reminder it was before the phase landed.
///
/// **It does not repeat the brief's attachments or linked tasks.** They are on the anchor task
/// (M7) and the coordinator reads them there; a briefing that copied them would be a second
/// version of the brief going stale the moment either is edited.
pub fn mission_briefing(
    title: &str,
    description: &str,
    task_id: TaskId,
    require_plan: bool,
    has_plan: bool,
) -> String {
    let mut text = format!("New mission: {title}\n\nTask id: {task_id}");
    let description = description.trim();
    if !description.is_empty() {
        text.push_str("\n\n");
        text.push_str(description);
    }
    text.push_str(
        "\n\nYou are this mission's coordinator. Its requirements, attachments and linked tasks \
         are the fields of the task above. Run it through `ubiq-mission`: ask for a phase move \
         with `request_phase`, and the user confirms or declines it.",
    );
    if has_plan {
        text.push_str(
            "\n\nA plan was supplied with the mission and it starts in refining \u{2014} read it \
             and refine it rather than starting a new one.",
        );
    }
    if require_plan {
        text.push_str(
            "\n\nA plan is required: moving to in progress is the user's own gate, and it is \
             refused while the plan is empty or has open threads.",
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, mission_assistant: Option<bool>) -> ProfileInfo {
        ProfileInfo {
            id: id.to_string(),
            agent_type: "claude-code".to_string(),
            account: None,
            model: None,
            mode: None,
            thinking: None,
            max_subagents: None,
            prompt: None,
            mcps: Vec::new(),
            mission_assistant,
            project: None,
        }
    }

    #[test]
    fn only_a_profile_ticked_true_is_offered() {
        let profiles = vec![
            profile("reviewer", Some(true)),
            profile("writer", None),
            profile("planner", Some(false)),
            profile("coach", Some(true)),
        ];
        let ids: Vec<&str> = assistants(&profiles)
            .into_iter()
            .map(|p| p.id.as_str())
            .collect();
        assert_eq!(ids, vec!["reviewer", "coach"]);
    }

    #[test]
    fn ready_needs_a_name_and_a_coordinator() {
        let mut form = NewMissionForm::new();
        assert!(!form.ready());
        form.title = "Ship v2".to_string();
        assert!(!form.ready(), "no coordinator chosen yet");
        form.coordinator = Some(Coordinator::Profile("reviewer".to_string()));
        assert!(form.ready());
        form.title = "   ".to_string();
        assert!(
            !form.ready(),
            "whitespace is not a title and there is no description"
        );
    }

    #[test]
    fn a_blank_title_is_taken_from_the_requirements() {
        let mut form = NewMissionForm::new();
        form.description = "\n# Payment retries\n\nThey fail twice.".to_string();
        form.coordinator = Some(Coordinator::Running(AgentId::generate()));
        assert_eq!(form.effective_title(), "Payment retries");
        assert!(form.ready(), "a description is enough to name a mission by");
    }

    #[test]
    fn the_briefing_carries_the_title_the_id_the_description_and_the_surface() {
        let id = TaskId::generate();
        let text = mission_briefing("Ship v2", "Get it out the door.", id, false, false);
        assert!(text.contains("Ship v2"));
        assert!(text.contains(&id.to_string()));
        assert!(text.contains("Get it out the door."));
        assert!(text.contains("ubiq-mission"));
        assert!(!text.contains("required"));
    }

    #[test]
    fn require_plan_is_said_as_the_gate_it_now_is() {
        let id = TaskId::generate();
        let text = mission_briefing("Ship v2", "", id, true, false);
        assert!(text.contains("A plan is required"));
        assert!(text.contains("gate"));
    }

    #[test]
    fn a_seeded_plan_is_said_so_the_coordinator_refines_rather_than_restarts() {
        let id = TaskId::generate();
        let text = mission_briefing("Ship v2", "", id, false, true);
        assert!(text.contains("refining"));
    }

    #[test]
    fn the_coordinator_trigger_says_what_was_picked() {
        let rows = vec![
            (
                "planner".to_string(),
                Coordinator::Profile("planner".to_string()),
            ),
            (
                "coach".to_string(),
                Coordinator::Profile("coach".to_string()),
            ),
        ];
        let mut form = NewMissionForm::new();
        assert_eq!(
            coordinator_label(&form, &rows, "Choose\u{2026}"),
            "Choose\u{2026}"
        );
        form.coordinator = Some(Coordinator::Profile("coach".to_string()));
        assert_eq!(coordinator_label(&form, &rows, "Choose\u{2026}"), "coach");
        // A pick whose row is gone — a profile deleted while the dialog is up — falls back rather
        // than drawing a stale name.
        form.coordinator = Some(Coordinator::Profile("ghost".to_string()));
        assert_eq!(
            coordinator_label(&form, &rows, "Choose\u{2026}"),
            "Choose\u{2026}"
        );
    }
}
