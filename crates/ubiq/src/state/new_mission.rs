//! The new-mission dialog's own state.
//!
//! Four questions — title, description, `require plan`, and which assistant profile to launch —
//! and one composed action: create the task, promote it to a mission, and start the chosen
//! assistant on it. Nothing here sends a message; that is `AppState::start_new_mission`'s job, in
//! `app/new_mission.rs`. This module only holds what was typed and answers what the picker offers.

use ubiq_proto::ids::TaskId;
use ubiq_proto::messages::ProfileInfo;

/// Every answer the dialog holds, while it is up.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct NewMissionForm {
    pub title: String,
    pub description: String,
    /// A reminder shown in the assistant's briefing, not a gate the host enforces — see
    /// [`mission_briefing`]. Kept as a plain flag here; what it means to the assistant is decided
    /// in one place, the briefing text, so that reading is cheap to change.
    pub require_plan: bool,
    /// The chosen assistant, by [`ProfileInfo::id`]. `None` until one is picked from
    /// [`assistants`], and Start is refused until it is.
    pub assistant: Option<String>,
    /// Whether the assistant picker's list is down.
    pub open: bool,
}

impl NewMissionForm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether Start may be pressed: a title, and an assistant to launch.
    pub fn ready(&self) -> bool {
        !self.title.trim().is_empty() && self.assistant.is_some()
    }
}

/// What the assistant picker's trigger says: the chosen profile's id once one is picked, the
/// placeholder until then. Kept as one function so the trigger and the test agree by construction
/// — the bug this once was is a trigger that drew the placeholder unconditionally, ignoring
/// `NewMissionForm::assistant` — the same idiom `kit::multi_label` follows for the multi-select
/// picker.
pub fn assistant_label(
    form: &NewMissionForm,
    candidates: &[&ProfileInfo],
    placeholder: &str,
) -> String {
    form.assistant
        .as_deref()
        .and_then(|id| candidates.iter().find(|profile| profile.id == id))
        .map(|profile| profile.id.clone())
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

/// The mission assistant's opening turn — the briefing `Message::PromptAgent` carries as the
/// conversation's first turn.
///
/// Carries the mission's title, description and id, and — since `require_plan` is a reminder
/// rather than something the host gates on — says so when it is set. Kept in this one function
/// because that reading is an assumption carried over from an earlier slice, not a settled
/// decision, so this is the one place to change if it turns out to be wrong.
pub fn mission_briefing(
    title: &str,
    description: &str,
    task_id: TaskId,
    require_plan: bool,
) -> String {
    let mut text = format!("New mission: {title}\n\nTask id: {task_id}");
    let description = description.trim();
    if !description.is_empty() {
        text.push_str("\n\n");
        text.push_str(description);
    }
    if require_plan {
        text.push_str(
            "\n\nA plan is requested for this mission before work starts on it \u{2014} a \
             reminder to write one, not something enforced.",
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
    fn ready_needs_a_title_and_an_assistant() {
        let mut form = NewMissionForm::new();
        assert!(!form.ready());
        form.title = "Ship v2".to_string();
        assert!(!form.ready(), "no assistant chosen yet");
        form.assistant = Some("reviewer".to_string());
        assert!(form.ready());
        form.title = "   ".to_string();
        assert!(!form.ready(), "whitespace is not a title");
    }

    #[test]
    fn the_briefing_carries_the_title_the_id_and_the_description() {
        let id = TaskId::generate();
        let text = mission_briefing("Ship v2", "Get it out the door.", id, false);
        assert!(text.contains("Ship v2"));
        assert!(text.contains(&id.to_string()));
        assert!(text.contains("Get it out the door."));
        assert!(!text.contains("reminder"));
    }

    #[test]
    fn require_plan_adds_a_reminder_rather_than_a_gate() {
        let id = TaskId::generate();
        let text = mission_briefing("Ship v2", "", id, true);
        assert!(text.contains("reminder"));
    }
}
