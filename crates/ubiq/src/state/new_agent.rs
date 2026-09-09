//! The form behind "New agent" — and behind the settings page's profile form, which asks the
//! same questions.
//!
//! A profile *is* a saved answer to "how should this agent start", so one form asks it once and
//! [`Purpose`] says what is done with the answer. Everything here is state and small pure
//! readings of it: no element, no colour, and nothing that names a message.

use ubiq_proto::conversation::ConfigChoice;
use ubiq_proto::messages::{AgentTypeInfo, CatalogueModel, ProfileInfo};

/// What the form is for. The same fields answer both questions, so the same form asks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Start a conversation now. Offers profiles in the target picker and a `Start` button.
    Start,
    /// Write a profile. No profile row in the target picker, no `Start`, a name and a `Save`.
    Profile,
}

/// The first answer: a harness signed into an identity, or a saved setup. Two groups in one list,
/// separated — the two [`crate::state::WorkbenchState::harness_choices`] already offers.
///
/// A harness row carries the identity with it rather than leaving it to a second question,
/// because a harness and the account it runs as is the pair the interface calls a harness: the
/// two answers arrive together, and the rows below stop asking for either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Harness {
        agent_type: String,
        /// `None` is "whichever identity the harness would use itself". Not on offer in the target
        /// list for now — every row there names an account — but a profile may still carry one.
        account: Option<String>,
    },
    Profile(String),
}

/// Which list is down. One at a time — the window's rule — and held on the form rather than in
/// `open_menu`, because the modal is redrawn from state on every frame and a picker inside one
/// has nowhere else to keep it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenList {
    Target,
    /// The harness *and* the identity: one list, because they are one question.
    Harness,
    Model,
    Thinking,
    Mode,
    Subagents,
}

/// Every answer the form holds, for either purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewAgentForm {
    pub purpose: Purpose,
    /// `None` is nothing chosen: every row below the first is drawn inert until this is answered.
    pub target: Option<Target>,
    pub agent_type: String,
    pub account: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub mode: Option<String>,
    /// Always false, and the control is disabled: an agent that survives a restart is not built
    /// yet, and a checkbox that lied about it would be worse than one that says "not yet".
    pub persistent: bool,
    /// The subagent ceiling this start asks for. `None` says nothing about it at all; `Some(0)`
    /// asks for none, which is a real answer and states itself.
    pub max_subagents: Option<u8>,
    /// The opening prompt. Typed into a textarea the window owns, and copied in here when the
    /// form is read — the same way the profile form reads its name field at save time.
    pub prompt: String,
    pub open: Option<OpenList>,
    /// Whether the "what should this be called" prompt is up over the form. Only a start form
    /// ever raises one: a profile form already has its name field.
    pub naming: bool,
    /// What the harness offers, from `Message::HarnessCatalogue`. Empty until one lands.
    pub models: Vec<CatalogueModel>,
    /// Whether a catalogue has been asked for and not yet answered. The model row says so rather
    /// than drawing an empty list, which would read as a harness with no models.
    pub probing: bool,
}

impl NewAgentForm {
    pub fn new(purpose: Purpose) -> Self {
        Self {
            purpose,
            target: None,
            agent_type: String::new(),
            account: None,
            model: None,
            thinking: None,
            mode: None,
            persistent: false,
            max_subagents: Some(DEFAULT_SUBAGENTS),
            prompt: String::new(),
            open: None,
            naming: false,
            models: Vec::new(),
            probing: false,
        }
    }

    /// The form as a saved setup already answers it. Its own harness is the target, so editing a
    /// profile opens on the profile rather than on the question it has already answered.
    pub fn from_profile(profile: &ProfileInfo, purpose: Purpose) -> Self {
        Self {
            target: Some(match purpose {
                Purpose::Start => Target::Profile(profile.id.clone()),
                Purpose::Profile => Target::Harness {
                    agent_type: profile.agent_type.clone(),
                    account: profile.account.clone(),
                },
            }),
            agent_type: profile.agent_type.clone(),
            account: profile.account.clone(),
            model: profile.model.clone(),
            thinking: profile.thinking.clone(),
            mode: profile.mode.clone(),
            max_subagents: profile.max_subagents,
            prompt: profile.prompt.clone().unwrap_or_default(),
            ..Self::new(purpose)
        }
    }

    /// What the form would be written down as. The id is the name field's, which the form does
    /// not hold: a profile is filed under whatever it is called, and renaming is saving a second.
    pub fn as_profile(&self, id: String) -> ProfileInfo {
        ProfileInfo {
            id,
            agent_type: self.agent_type.clone(),
            account: self.account.clone(),
            model: self.model.clone(),
            mode: self.mode.clone(),
            thinking: self.thinking.clone(),
            max_subagents: self.max_subagents,
            prompt: (!self.prompt.trim().is_empty()).then(|| self.prompt.trim().to_string()),
        }
    }

    /// The reasoning-effort levels the chosen model accepts — empty where the model has none,
    /// which is the normal case, and empty where no model is chosen yet.
    pub fn model_levels(&self) -> &[ConfigChoice] {
        let Some(model) = self.model.as_deref() else {
            return &[];
        };
        self.models
            .iter()
            .find(|it| it.id == model)
            .map(|it| it.levels.as_slice())
            .unwrap_or_default()
    }

    /// The mode a fresh start opens on: the harness's own "ask nothing", where it names one.
    ///
    /// Which id means "all permissions" is the harness's business, not the interface's, so this
    /// reads the field the library fills rather than guessing at a position in `modes`.
    pub fn default_mode(harness: &AgentTypeInfo) -> Option<String> {
        harness.unattended_mode.clone()
    }

    /// What the start has to say to the harness before the user says anything: the subagent
    /// ceiling as a directive, then the opening prompt the form was given.
    ///
    /// The ceiling is never a launch flag — no harness has one — so the only way to ask for it is
    /// to say so, in the strongest words a turn has. Either half may be absent; `None` is the
    /// case where there is nothing at all to say.
    ///
    /// **This is not a turn.** Sending it at start time opens the transcript on a directive the
    /// user never wrote, which is what the reader sees first and reads as the conversation's
    /// beginning. It is held instead, and [`fold_preamble`] puts it in front of the first thing
    /// the user actually sends — the harness reads it, the transcript does not show it.
    pub fn preamble(&self) -> Option<String> {
        let mut turn = String::new();
        if let Some(max) = self.max_subagents {
            turn.push_str(&format!(
                "**IMPORTANT: RUN AT MOST {max} SUBAGENTS AT THE SAME TIME.**"
            ));
        }
        let prompt = self.prompt.trim();
        if !prompt.is_empty() {
            if !turn.is_empty() {
                turn.push_str("\n\n");
            }
            turn.push_str(prompt);
        }
        (!turn.is_empty()).then_some(turn)
    }
}

/// What the subagent picker opens on. Three is the working default the directive is written for.
pub const DEFAULT_SUBAGENTS: u8 = 3;

/// Put a held preamble in front of the turn the user typed, as one text.
///
/// The harness is handed both; only the typed half is what the conversation shows, because that
/// is the half the user wrote. A blank line between them is what makes the directive read as a
/// standing instruction above the work rather than as its first sentence.
pub fn fold_preamble(preamble: &str, typed: &str) -> String {
    match (preamble.trim(), typed.trim_start()) {
        ("", typed) => typed.to_string(),
        (preamble, "") => preamble.to_string(),
        (preamble, typed) => format!("{preamble}\n\n{typed}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(value: &str) -> ConfigChoice {
        ConfigChoice {
            value: value.to_string(),
            name: value.to_string(),
            description: None,
            group: None,
        }
    }

    fn harness(unattended: Option<&str>) -> AgentTypeInfo {
        AgentTypeInfo {
            id: "claude-code".to_string(),
            label: "Claude Code".to_string(),
            command: "claude".to_string(),
            available: true,
            chat: true,
            modes: vec![choice("plan"), choice("bypass")],
            unattended_mode: unattended.map(str::to_string),
            keeps_sessions: true,
        }
    }

    fn form_with_models() -> NewAgentForm {
        let mut form = NewAgentForm::new(Purpose::Start);
        form.models = vec![
            CatalogueModel {
                id: "sonnet".to_string(),
                description: None,
                default: true,
                levels: vec![choice("low"), choice("high")],
                default_level: Some("low".to_string()),
            },
            CatalogueModel {
                id: "haiku".to_string(),
                description: None,
                default: false,
                levels: Vec::new(),
                default_level: None,
            },
        ];
        form
    }

    #[test]
    fn levels_belong_to_the_chosen_model() {
        let mut form = form_with_models();
        assert!(
            form.model_levels().is_empty(),
            "nothing chosen, nothing on offer"
        );
        form.model = Some("sonnet".to_string());
        assert_eq!(form.model_levels().len(), 2);
        form.model = Some("haiku".to_string());
        assert!(
            form.model_levels().is_empty(),
            "a model with no knob offers none"
        );
        form.model = Some("gone".to_string());
        assert!(
            form.model_levels().is_empty(),
            "a model the catalogue lost offers none"
        );
    }

    #[test]
    fn the_default_mode_is_the_harnesss_own_word_for_it() {
        assert_eq!(
            NewAgentForm::default_mode(&harness(Some("bypass"))),
            Some("bypass".to_string())
        );
        assert_eq!(NewAgentForm::default_mode(&harness(None)), None);
    }

    #[test]
    fn the_preamble_states_the_ceiling_then_the_prompt() {
        let mut form = NewAgentForm::new(Purpose::Start);
        form.prompt = "  Plan the migration.  ".to_string();
        assert_eq!(
            form.preamble().as_deref(),
            Some("**IMPORTANT: RUN AT MOST 3 SUBAGENTS AT THE SAME TIME.**\n\nPlan the migration.")
        );

        form.prompt = String::new();
        assert_eq!(
            form.preamble().as_deref(),
            Some("**IMPORTANT: RUN AT MOST 3 SUBAGENTS AT THE SAME TIME.**"),
            "a ceiling with no prompt is still worth saying"
        );

        form.max_subagents = Some(0);
        assert!(
            form.preamble()
                .is_some_and(|turn| turn.contains("AT MOST 0")),
            "none at all is an answer, not an absence"
        );

        form.max_subagents = None;
        assert_eq!(form.preamble(), None, "nothing to say, no turn");

        form.prompt = "Go".to_string();
        assert_eq!(form.preamble().as_deref(), Some("Go"));
    }

    #[test]
    fn a_held_preamble_goes_in_front_of_what_the_user_typed() {
        assert_eq!(
            fold_preamble("Rule.", "Do the thing."),
            "Rule.\n\nDo the thing."
        );
        assert_eq!(fold_preamble("", "Do the thing."), "Do the thing.");
        assert_eq!(fold_preamble("Rule.", ""), "Rule.");
    }
}
