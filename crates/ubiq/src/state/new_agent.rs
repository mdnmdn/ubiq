//! The form behind "New agent" — and behind the settings page's definition form, which asks the
//! same questions.
//!
//! A definition *is* a saved answer to "how should this agent start", so one form asks it once and
//! [`Purpose`] says what is done with the answer. Everything here is state and small pure
//! readings of it: no element, no colour, and nothing that names a message.

use ubiq_proto::conversation::ConfigChoice;
use ubiq_proto::ids::{ProjectId, TaskId};
use ubiq_proto::messages::{
    AgentDefinition, AgentTypeInfo, CatalogueModel, TAG_COORDINATOR, TAG_DOC, TAG_WORKER,
};
use ubiq_proto::settings::Grant;

/// What the form is for. The same fields answer both questions, so the same form asks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Start a conversation now. Offers definitions in the target picker and a `Start` button.
    Start,
    /// Write a definition. No definition row in the target picker, no `Start`, a name and a `Save`.
    AgentDefinition,
}

/// Which half of the New agent dialog is in front.
///
/// The two answers to "what is being started" were one list with a hairline in it; they are two
/// tabs because they are two different questions — *which saved agent* and *which tool, as whom* —
/// and a list that mixed them made the second read as a footnote to the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NewAgentTab {
    /// The saved setups, one dropdown. What it runs on is shown on the row rather than asked for.
    #[default]
    Agents,
    /// A harness signed into an identity, and every question a bare start answers itself. No
    /// definition is on offer here: that is the other tab.
    Harness,
}

impl NewAgentTab {
    pub fn all() -> [NewAgentTab; 2] {
        [NewAgentTab::Agents, NewAgentTab::Harness]
    }

    pub fn label(self) -> &'static str {
        match self {
            NewAgentTab::Agents => "Agents",
            NewAgentTab::Harness => "Harness",
        }
    }
}

/// The first answer: a harness signed into an identity, a harness on its own, or a saved setup.
/// The groups are whichever ones [`crate::state::WorkbenchState::harness_choices`] has rows for —
/// `Configured`, `Default`, `Defined`, and `Already chosen` for an answer the form holds that the
/// others do not carry — separated, in that order.
///
/// A harness row carries the identity with it rather than leaving it to a second question,
/// because a harness and the account it runs as is the pair the interface calls a harness: the
/// two answers arrive together, and the rows below stop asking for either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Harness {
        agent_type: String,
        /// `None` is "whichever identity the harness would use itself" — a row of its own in the
        /// target list, the `Default` group, which is every installed harness no account has
        /// signed into.
        account: Option<String>,
    },
    AgentDefinition(String),
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
    /// The one list here that is not a picker: several servers may be ticked at once, so it is a
    /// checklist rather than a choice. It uses the same discriminant anyway, because "exactly one
    /// list is down" is the form's rule regardless of what the list is made of.
    Mcps,
    /// The skills checklist, on the MCP one's terms.
    Skills,
}

impl OpenList {
    /// Whether this list draws the filter field the form's pickers share.
    ///
    /// Every [`crate::ui::kit::Picker`] here opts into search and takes the keyboard with it while
    /// it is down. The MCP checklist does not — a handful of built-in servers is shorter than the
    /// field would be — so opening it must leave the keyboard where it was: focusing a field
    /// nothing draws is a keyboard nobody owns, and the window's own Escape never arrives at it.
    pub fn has_filter(self) -> bool {
        !matches!(self, Self::Mcps | Self::Skills)
    }
}

/// Every answer the form holds, for either purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewAgentForm {
    pub purpose: Purpose,
    /// Which tab the start dialog is on. Meaningless under [`Purpose::AgentDefinition`], which
    /// draws no tabs — a definition is always written against a harness.
    pub tab: NewAgentTab,
    /// Whether the Agents tab's `Customize` has been pressed: the harness, model and effort the
    /// chosen definition names are drawn as overridable rows instead of as a line on its row.
    /// Reset whenever the tab or the definition changes — a customization belongs to the answer it
    /// was made against.
    pub customize: bool,
    /// `None` is nothing chosen: every row below the first is drawn inert until this is answered.
    pub target: Option<Target>,
    /// What this setup is for, in prose — [`AgentDefinition::description`]. Read by another agent
    /// through the mission MCP, not by the host: unlike every other field here, nothing downstream
    /// acts on it, so it is free-form and may be several lines.
    pub description: String,
    pub agent_type: String,
    pub account: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub mode: Option<String>,
    /// Always false, and the control is disabled: an agent that survives a restart is not built
    /// yet, and a checkbox that lied about it would be worse than one that says "not yet".
    pub persistent: bool,
    /// Whether this setup is fit to run as a planning assistant — [`AgentDefinition::mission_assistant`].
    /// Drawn only under [`Purpose::AgentDefinition`]: a bare start has nothing to save the flag onto, and
    /// the new-mission dialog's picker is what reads it back.
    pub mission_assistant: bool,
    /// What this definition is tagged as — [`AgentDefinition::tags`], in the order they were
    /// added. `coordinator` and `worker` are roles: the host re-asserts the MCP servers each
    /// implies on every save, so the form never has to hold that set itself.
    pub tags: Vec<String>,
    /// Whether the definition is switched off — [`AgentDefinition::disabled`]. Carried through a
    /// round trip so saving an edit never quietly switches one back on.
    pub disabled: bool,
    /// The subagent ceiling this start asks for. `None` says nothing about it at all; `Some(0)`
    /// asks for none, which is a real answer and states itself.
    pub max_subagents: Option<u8>,
    /// The MCP servers Ubiq is asked to inject into this start, by
    /// [`ubiq_proto::mcp::McpInfo::name`] — the slug, which is what a definition stores and what a
    /// start names. Order is the order they were ticked in and is kept stable, so saving a form
    /// twice writes the same list rather than a reshuffle of it.
    ///
    /// What is *on offer* is not here: that is the window's own
    /// [`crate::state::WorkbenchState::mcps`], one list for the build rather than a copy per form.
    pub mcps: Vec<String>,
    /// The skills this start asks for, by catalog id ([`ubiq_proto::catalog::SkillInfo::id`]).
    /// Ticked order is kept, the way [`Self::mcps`] keeps it.
    pub skills: Vec<String>,
    /// The extra read-write folders for an isolated run — [`AgentDefinition::grants`], seeded from
    /// the chosen definition and added to here for this launch. Each `write: true`; ignored by the
    /// host when isolation is off.
    pub grants: Vec<Grant>,
    /// Whether the folders dialog is up over the form, the way [`Self::naming`] is for the name
    /// prompt.
    pub policies: bool,
    /// Whether the last path typed into that dialog was refused, for the line under the field.
    pub grant_invalid: bool,
    /// The **agent prompt** — [`AgentDefinition::prompt`]: standing instructions folded in front
    /// of a run's first turn ([`Self::preamble`]). On the definition form it is typed into a
    /// textarea the window owns and copied in here when the form is read; on a start it is the
    /// chosen definition's own, never edited — what a start says first is its separate initial
    /// prompt, which the window holds and sends as a turn.
    pub prompt: String,
    pub open: Option<OpenList>,
    /// Whether the "what should this be called" prompt is up over the form. Only a start form
    /// ever raises one: a definition form already has its name field.
    pub naming: bool,
    /// What the harness offers, from `Message::HarnessCatalogue`. Empty until one lands.
    pub models: Vec<CatalogueModel>,
    /// Whether a catalogue has been asked for and not yet answered. The model row says so rather
    /// than drawing an empty list, which would read as a harness with no models.
    pub probing: bool,
    /// The project a [`Purpose::AgentDefinition`] form writes its definition into, when it was opened from
    /// one. `None` — the app-wide settings screen — writes a global definition, which is every
    /// definition that existed before scoping. Never a pick inside the form: where a definition lives
    /// is settled by where the user asked for it, and a saved definition is edited in the scope it
    /// was written in ([`AgentDefinition::project`]).
    pub project: Option<ProjectId>,
    /// The task this start is being assigned to, where the form was raised from the board's own
    /// "Assign to an agent" button — `None` for every other way in. Drives the two checkboxes
    /// `ui::new_agent` draws only here, and what [`task_assignment_prompt`] composes the opening
    /// prompt from; nothing about the target, the MCP checklist or any other row changes on its
    /// account — this is the one form both cards share, not a second one.
    pub for_task: Option<TaskId>,
    /// The task assignment's "ask for feedback" checkbox. Ticked is the safer default: an agent
    /// that assumes rather than asks is the one that has to be told to stop later.
    pub ask_for_feedback: bool,
    /// The task assignment's "plan mode" checkbox. A prompt instruction only, for now — see
    /// [`task_assignment_prompt`].
    pub plan_mode: bool,
    /// Text appended after the composed opening prompt — the review line of a *Feedback to an
    /// agent* assignment — so a checkbox toggle's recomposition keeps it.
    pub prompt_suffix: String,
    /// The tags a start was opened narrowed to ([`NewAgentOpen::tags`]): only definitions carrying
    /// at least one of them are offered, and the Harness tab is not drawn. Empty is no narrowing.
    /// Belongs to how the form was opened, so it survives every re-pick and tab change.
    pub tag_filter: Vec<String>,
    /// The servers a start was opened with ticked ([`NewAgentOpen::mcps`]). Like
    /// [`Self::tag_filter`] it belongs to how the form was opened: every re-pick and tab change
    /// rebuilds [`Self::mcps`] from the new answer and then ticks these again
    /// ([`Self::tick_opened_mcps`]).
    pub opened_mcps: Vec<String>,
}

/// How the New agent modal is asked to open — `AppState::open_new_agent`'s one argument.
/// `Default` is the plain dialog, so a caller with nothing to say stays a one-liner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewAgentOpen {
    /// Offer only definitions carrying at least one of these tags, and no bare harness.
    pub tags: Option<Vec<String>>,
    /// Start at once, without showing the dialog, when a target resolves — the last one used in
    /// this project if it passes [`Self::tags`], else the first definition that does. The dialog
    /// is shown when nothing resolves.
    pub autostart: bool,
    /// What the initial-prompt field opens holding — said as the first user turn of the start.
    pub initial_prompt: Option<String>,
    /// MCP servers the form opens with ticked, by name. A name this build cannot inject is simply
    /// not offered, so it stays unticked on screen.
    pub mcps: Vec<String>,
}

impl NewAgentForm {
    /// Whether `definition` passes the form's [`Self::tag_filter`].
    pub fn offers(&self, definition: &AgentDefinition) -> bool {
        self.tag_filter.is_empty() || definition.has_any_tag(&self.tag_filter)
    }
}

/// Whether `path` is one a grant may name: absolute, or `~`-prefixed — what the host resolves.
pub fn is_grant_path(path: &str) -> bool {
    path.starts_with('/') || path == "~" || path.starts_with("~/")
}

/// The MCP servers a task assignment preselects — mirrors
/// `crates/ubiq-host/src/mcp/catalogue::{MANAGE_UBIQ_TASKS, UBIQ_ASK}` by slug. Duplicated as
/// plain strings rather than named constants shared with the host: `crates/ubiq` does not depend
/// on `crates/ubiq-host`, and the slug — not the constant — is the wire contract
/// ([`ubiq_proto::mcp::McpInfo::name`]).
pub const TASK_ASSIGN_MCPS: [&str; 2] = ["manage-ubiq-tasks", "ubiq-ask"];

/// What the **mission/task coordinator** flag implies — mirrors
/// `crates/ubiq-host/src/mcp/catalogue::COORDINATOR_MCPS` by slug, duplicated for
/// `TASK_ASSIGN_MCPS`'s reason: the interface names no host type, and the slug is the contract.
///
/// The host re-adds this set on every save of a definition carrying the flag, so the interface
/// draws these servers as ticked and takes no click on them: a checklist that let one be unticked
/// would be describing a save that is not going to happen.
pub const COORDINATOR_MCPS: [&str; 4] =
    ["ubiq-mission", "ubiq-plan", "manage-ubiq-tasks", "ubiq-kb"];

/// What the **mission/task worker** flag implies — mirrors `catalogue::WORKER_MCPS`, on the same
/// terms as [`COORDINATOR_MCPS`].
pub const WORKER_MCPS: [&str; 4] = ["use-mission", "use-task", "project-info", "ubiq-kb"];

/// What the **`doc`** tag implies — the collaboration tools over a markdown file and the
/// architecture-diagram server, on the same terms as [`COORDINATOR_MCPS`].
pub const DOC_MCPS: [&str; 2] = ["ubiq-doc", "ubiq-archify"];

/// The opening prompt a task assignment starts the agent on: what to work on, and how it should
/// handle a gap in what it knows.
///
/// **Recomposed whole, not patched**, whenever the task label or either checkbox changes — see
/// `AppState::toggle_new_agent_ask_feedback` / `toggle_new_agent_plan_mode` — so what is in the
/// field always matches what the checkboxes say. A user who has typed over it loses that edit on
/// the next toggle; accepted for now, since the field is short and the checkboxes are answered
/// before the prose usually is.
///
/// **`plan_mode` is a sentence, not a tool.** No MCP server is ticked for it — `TASK_ASSIGN_MCPS`
/// carries only the board and the feedback tool — so an agent told to "work in plan mode" today
/// has no `ubiq-plan` server to write one into. TODO: once T-64's plan-mode follow-up wires the
/// plan tool into this form, tick `ubiq-plan` here too instead of leaving the instruction as
/// words alone.
pub fn task_assignment_prompt(task_label: &str, ask_for_feedback: bool, plan_mode: bool) -> String {
    let mut prompt = format!("Start working on task {task_label}.");
    if ask_for_feedback {
        prompt.push_str(" Ask for feedback with the feedback tool whenever you need it.");
    } else {
        prompt.push_str(" Assume as much as you reasonably can rather than asking for feedback.");
    }
    if plan_mode {
        prompt.push_str(" Work in plan mode: write out your plan before making changes.");
    }
    prompt.push_str(&format!(" {}", task_tracking_instruction(task_label)));
    prompt
}

/// The sentence every task-linked turn carries: keep the card in step with the work, through the
/// board's own MCP server — the status words are the ones `manage-ubiq-tasks` reads.
pub fn task_tracking_instruction(task_label: &str) -> String {
    format!(
        "Keep task {task_label} updated as you go with the manage-ubiq-tasks tools: move it to \
         \"in progress\" when you start, add a comment when you make progress, and move it to \
         \"in review\" when you are done."
    )
}

/// The turn the task panel's *Continue with an agent* sends an agent already linked to the task.
pub fn task_continue_prompt(task_label: &str) -> String {
    format!(
        "Continue working on task {task_label}. {}",
        task_tracking_instruction(task_label)
    )
}

/// What the task panel's *Feedback to an agent* puts in front of the reviewer's own words — left
/// ending on `Feedback: `, so the user types straight after it.
pub fn task_feedback_prefix(task_label: &str) -> String {
    format!("Task {task_label} was reviewed. Feedback: ")
}

impl NewAgentForm {
    pub fn new(purpose: Purpose) -> Self {
        Self {
            purpose,
            tab: NewAgentTab::default(),
            customize: false,
            target: None,
            description: String::new(),
            agent_type: String::new(),
            account: None,
            model: None,
            thinking: None,
            mode: None,
            persistent: false,
            mission_assistant: false,
            tags: Vec::new(),
            disabled: false,
            max_subagents: Some(DEFAULT_SUBAGENTS),
            mcps: Vec::new(),
            skills: Vec::new(),
            grants: Vec::new(),
            policies: false,
            grant_invalid: false,
            prompt: String::new(),
            open: None,
            naming: false,
            models: Vec::new(),
            probing: false,
            project: None,
            for_task: None,
            ask_for_feedback: true,
            plan_mode: false,
            prompt_suffix: String::new(),
            tag_filter: Vec::new(),
            opened_mcps: Vec::new(),
        }
    }

    /// Tick the servers the form was opened with ([`Self::opened_mcps`]) that are not ticked yet,
    /// after what is ticked, keeping its order.
    pub fn tick_opened_mcps(&mut self) {
        for mcp in &self.opened_mcps {
            if !self.mcps.contains(mcp) {
                self.mcps.push(mcp.clone());
            }
        }
    }

    /// The form as a saved setup already answers it. Its own harness is the target, so editing a
    /// definition opens on the definition rather than on the question it has already answered.
    pub fn from_definition(definition: &AgentDefinition, purpose: Purpose) -> Self {
        Self {
            target: Some(match purpose {
                Purpose::Start => Target::AgentDefinition(definition.id.clone()),
                Purpose::AgentDefinition => Target::Harness {
                    agent_type: definition.agent_type.clone(),
                    account: definition.account.clone(),
                },
            }),
            description: definition.description.clone().unwrap_or_default(),
            agent_type: definition.agent_type.clone(),
            account: definition.account.clone(),
            model: definition.model.clone(),
            thinking: definition.thinking.clone(),
            mode: definition.mode.clone(),
            max_subagents: definition.max_subagents,
            mcps: definition.mcps.clone(),
            skills: definition.skills.clone(),
            grants: definition.grants.clone(),
            prompt: definition.prompt.clone().unwrap_or_default(),
            mission_assistant: definition.mission_assistant.unwrap_or(false),
            tags: definition.tags.clone(),
            disabled: definition.disabled,
            // Editing keeps the scope it was found in: a project definition saved from its own
            // project's settings goes back where it came from, and the global form never
            // acquires a project it was not opened with.
            project: definition.project,
            ..Self::new(purpose)
        }
    }

    /// What the form would be written down as. The id is the name field's, which the form does
    /// not hold: a definition is filed under whatever it is called, and renaming is saving a second.
    pub fn as_definition(&self, id: String) -> AgentDefinition {
        AgentDefinition {
            id,
            description: (!self.description.trim().is_empty())
                .then(|| self.description.trim().to_string()),
            agent_type: self.agent_type.clone(),
            account: self.account.clone(),
            model: self.model.clone(),
            mode: self.mode.clone(),
            thinking: self.thinking.clone(),
            max_subagents: self.max_subagents,
            prompt: (!self.prompt.trim().is_empty()).then(|| self.prompt.trim().to_string()),
            mcps: self.mcps.clone(),
            skills: self.skills.clone(),
            grants: self.grants.clone(),
            // Ticked is written down; unticked writes `None` rather than `Some(false)` — the two
            // read the same to every filter, and `None` is the ordinary "says nothing" shape every
            // other optional field here already uses.
            mission_assistant: self.mission_assistant.then_some(true),
            // What the role tags imply is the host's answer (it re-adds the role's MCP servers
            // on save), not the form's.
            tags: self.tags.clone(),
            disabled: self.disabled,
            // Which root the host writes it into. The scope is not a field the user picks; it
            // is the surface the form was opened from.
            project: self.project,
        }
    }

    /// Add one read-write folder, as typed. Accepts an absolute or `~`-prefixed path and returns
    /// whether it was taken; a path already held is taken once.
    pub fn add_grant(&mut self, path: &str) -> bool {
        let path = path.trim();
        let trimmed = path.trim_end_matches('/');
        let path = if trimmed.is_empty() { path } else { trimmed };
        if !is_grant_path(path) {
            return false;
        }
        if !self.grants.iter().any(|grant| grant.path == path) {
            self.grants.push(Grant {
                path: path.to_string(),
                write: true,
            });
        }
        true
    }

    /// Drop the folder at `index`, if there is one.
    pub fn remove_grant(&mut self, index: usize) {
        if index < self.grants.len() {
            self.grants.remove(index);
        }
    }

    /// Tick or untick one MCP server, by its slug.
    ///
    /// Ticking appends rather than inserting in the catalogue's order: the list is what the user
    /// built, and a set that reordered itself as it was filled in is one nobody can read back.
    pub fn toggle_mcp(&mut self, name: &str) {
        // A server a role flag implies is not the checklist's to remove: the host re-adds it on
        // save, so unticking it would flicker back on the next answer.
        if self.implies_mcp(name) {
            return;
        }
        match self.mcps.iter().position(|it| it == name) {
            Some(at) => {
                self.mcps.remove(at);
            }
            None => self.mcps.push(name.to_string()),
        }
    }

    /// Tick or untick one skill, by its catalog id. Ticked order is kept, like [`Self::mcps`].
    pub fn toggle_skill(&mut self, id: &str) {
        match self.skills.iter().position(|it| it == id) {
            Some(at) => {
                self.skills.remove(at);
            }
            None => self.skills.push(id.to_string()),
        }
    }

    /// Whether the form carries `tag`.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|it| it == tag)
    }

    /// Add `tag` if it is not there, take it off if it is. Order is kept, like [`Self::mcps`].
    pub fn toggle_tag(&mut self, tag: &str) {
        match self.tags.iter().position(|it| it == tag) {
            Some(at) => {
                self.tags.remove(at);
            }
            None => self.tags.push(tag.to_string()),
        }
    }

    /// Add one tag as typed — trimmed, and taken once. Returns whether there was anything to add.
    pub fn add_tag(&mut self, tag: &str) -> bool {
        let tag = tag.trim();
        if tag.is_empty() {
            return false;
        }
        if !self.has_tag(tag) {
            self.tags.push(tag.to_string());
        }
        true
    }

    /// Whether one of the two role tags already asks for this server. Such a server is drawn
    /// ticked and inert: what it is ticked by is the tag, not the checklist.
    pub fn implies_mcp(&self, name: &str) -> bool {
        (self.has_tag(TAG_COORDINATOR) && COORDINATOR_MCPS.contains(&name))
            || (self.has_tag(TAG_WORKER) && WORKER_MCPS.contains(&name))
            || (self.has_tag(TAG_DOC) && DOC_MCPS.contains(&name))
    }

    /// Whether this server is ticked at all, by hand or by a role.
    pub fn wants_mcp(&self, name: &str) -> bool {
        self.implies_mcp(name) || self.mcps.iter().any(|it| it == name)
    }

    /// How many servers this start asks for — the roles' included, each counted once. `ubiq-kb`
    /// is in both role sets, and it is one server however many flags name it.
    pub fn mcp_count(&self) -> usize {
        let mut named: Vec<&str> = self.mcps.iter().map(String::as_str).collect();
        for name in COORDINATOR_MCPS.iter().chain(WORKER_MCPS.iter()) {
            if self.implies_mcp(name) && !named.contains(name) {
                named.push(name);
            }
        }
        named.len()
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
    use ubiq_proto::quota::QuotaSource;

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
            acp: false,
            modes: vec![choice("plan"), choice("bypass")],
            unattended_mode: unattended.map(str::to_string),
            keeps_sessions: true,
            quota: QuotaSource::Push,
            shares_home: false,
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
    fn ticking_a_server_appends_and_unticking_removes_it() {
        let mut form = NewAgentForm::new(Purpose::AgentDefinition);
        form.toggle_mcp("project-info");
        form.toggle_mcp("test");
        assert_eq!(form.mcps, vec!["project-info", "test"]);
        form.toggle_mcp("project-info");
        assert_eq!(form.mcps, vec!["test"], "unticking takes it back out");
        assert_eq!(
            form.as_definition("saved".to_string()).mcps,
            vec!["test"],
            "what is ticked is what is written down"
        );
    }

    #[test]
    fn grants_take_absolute_and_home_paths_only_and_round_trip() {
        let mut form = NewAgentForm::new(Purpose::AgentDefinition);
        assert!(form.add_grant("/data/shared/"));
        assert!(form.add_grant("~/notes"));
        assert!(form.add_grant("/data/shared"), "a repeat is taken once");
        assert!(!form.add_grant("relative/dir"));
        assert!(!form.add_grant(""));
        assert_eq!(form.grants.len(), 2);
        assert_eq!(form.grants[0].path, "/data/shared");
        assert!(form.grants.iter().all(|grant| grant.write));

        let definition = form.as_definition("review".into());
        assert_eq!(definition.grants, form.grants);
        let mut again = NewAgentForm::from_definition(&definition, Purpose::AgentDefinition);
        assert_eq!(again.grants, form.grants, "an edit opens with them");
        again.remove_grant(0);
        assert_eq!(again.grants.len(), 1);
    }

    #[test]
    fn ticked_skills_are_written_down_and_read_back() {
        let mut form = NewAgentForm::new(Purpose::AgentDefinition);
        form.toggle_skill("pdf");
        form.toggle_skill("docx");
        form.toggle_skill("pdf");
        assert_eq!(form.skills, vec!["docx"], "unticking takes it back out");

        let definition = form.as_definition("writer".to_string());
        assert_eq!(definition.skills, vec!["docx"]);
        let again = NewAgentForm::from_definition(&definition, Purpose::AgentDefinition);
        assert_eq!(again.skills, vec!["docx"], "an edit opens with them ticked");
    }

    fn a_definition(mission_assistant: Option<bool>) -> AgentDefinition {
        AgentDefinition {
            id: "reviewer".to_string(),
            description: None,
            agent_type: "claude-code".to_string(),
            account: None,
            model: None,
            mode: None,
            thinking: None,
            max_subagents: None,
            prompt: None,
            mcps: Vec::new(),
            skills: Vec::new(),
            mission_assistant,
            tags: Vec::new(),
            disabled: false,
            project: None,
            grants: Vec::new(),
        }
    }

    #[test]
    fn saving_a_definition_carries_the_mission_assistant_flag_forward() {
        // The bug this closes: a definition already marked as a mission assistant, opened and saved
        // again through the form with nothing touched, must not come back out unmarked.
        let form =
            NewAgentForm::from_definition(&a_definition(Some(true)), Purpose::AgentDefinition);
        assert!(
            form.mission_assistant,
            "the form reads the definition's flag"
        );
        assert_eq!(
            form.as_definition("reviewer".to_string()).mission_assistant,
            Some(true),
            "and writes it back rather than dropping it"
        );
    }

    #[test]
    fn ticking_it_from_a_definition_with_none_writes_it_down() {
        let mut form = NewAgentForm::from_definition(&a_definition(None), Purpose::AgentDefinition);
        assert!(!form.mission_assistant);
        form.mission_assistant = true;
        assert_eq!(
            form.as_definition("reviewer".to_string()).mission_assistant,
            Some(true)
        );
    }

    #[test]
    fn an_unticked_flag_writes_none_not_false() {
        let form = NewAgentForm::new(Purpose::AgentDefinition);
        assert_eq!(
            form.as_definition("fresh".to_string()).mission_assistant,
            None,
            "None and Some(false) read the same to a filter, and None is the ordinary shape"
        );
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

    /// `T-64`'s two checkboxes each flip one sentence of the assignment prompt, independently —
    /// every combination names the task once and adds exactly one clause per checkbox.
    #[test]
    fn the_task_assignment_prompt_answers_both_checkboxes() {
        let ask = task_assignment_prompt("T-64", true, false);
        assert!(ask.starts_with("Start working on task T-64."));
        assert!(ask.contains("Ask for feedback"));
        assert!(!ask.contains("Assume as much"));
        assert!(!ask.contains("plan mode"));

        let assume = task_assignment_prompt("T-64", false, false);
        assert!(assume.contains("Assume as much"));
        assert!(!assume.contains("Ask for feedback"));

        let planning = task_assignment_prompt("T-64", true, true);
        assert!(planning.contains("Ask for feedback"));
        assert!(planning.contains("plan mode"));

        // Whatever the checkboxes say, the agent is told to keep the card in step.
        for prompt in [&ask, &assume, &planning] {
            assert!(prompt.contains("\"in progress\""));
            assert!(prompt.contains("\"in review\""));
            assert!(prompt.contains("manage-ubiq-tasks"));
        }
    }

    #[test]
    fn continue_and_feedback_name_the_task() {
        let next = task_continue_prompt("T-320");
        assert!(next.starts_with("Continue working on task T-320."));
        assert!(next.contains("\"in review\""));
        assert_eq!(
            task_feedback_prefix("T-320"),
            "Task T-320 was reviewed. Feedback: "
        );
    }

    /// A role flag ticks its own servers and holds them ticked: the host re-adds them on every
    /// save, so the checklist must neither let one go nor count it twice when both roles name it.
    #[test]
    fn a_role_ticks_its_servers_and_the_checklist_cannot_untick_them() {
        let mut form = NewAgentForm::new(Purpose::AgentDefinition);
        form.toggle_mcp("test");
        assert!(!form.wants_mcp("ubiq-plan"), "no role, no implied server");

        form.toggle_tag("planner");
        assert!(
            !form.implies_mcp("ubiq-plan"),
            "a plain tag implies nothing"
        );
        form.toggle_tag("coordinator");
        assert!(form.implies_mcp("ubiq-plan"));
        assert!(form.wants_mcp("ubiq-plan"));
        form.toggle_mcp("ubiq-plan");
        assert!(
            form.wants_mcp("ubiq-plan") && !form.mcps.iter().any(|it| it == "ubiq-plan"),
            "unticking an implied server does nothing, and it is not written onto the list either"
        );
        assert_eq!(form.mcp_count(), 1 + COORDINATOR_MCPS.len());

        // `ubiq-kb` is in both role sets, and it is one server.
        form.toggle_tag("worker");
        assert_eq!(
            form.mcp_count(),
            1 + COORDINATOR_MCPS.len() + WORKER_MCPS.len() - 1
        );

        // What the form writes down stays what the user ticked: the role's set is the host's to
        // merge back, and writing it here would make an unticked role leave its servers behind.
        assert_eq!(form.as_definition("runner".to_string()).mcps, vec!["test"]);
    }

    /// The MCPs a task assignment preselects are the board and the one tool that waits for a
    /// person — not the plan tool, which `task_assignment_prompt`'s doc comment leaves as a TODO
    /// until a follow-up wires it in.
    #[test]
    fn task_assign_mcps_are_the_board_and_the_feedback_tool() {
        assert_eq!(TASK_ASSIGN_MCPS, ["manage-ubiq-tasks", "ubiq-ask"]);
    }
}
