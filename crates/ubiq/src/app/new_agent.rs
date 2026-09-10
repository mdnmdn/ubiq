//! The New agent modal's mutators — and the settings page's profile form's, because both are
//! drawn from one [`NewAgentForm`] and a shared body needs shared listeners.
//!
//! Only one of the two is ever up, so every mutator here reaches for whichever it is rather than
//! taking a discriminator: two ways to answer the same question is how the two forms would drift
//! apart again.

use super::*;
use crate::state::new_agent::{NewAgentForm, OpenList, Purpose, Target, fold_preamble};

impl AppState {
    /// The form on screen, whichever raised it. The New agent modal is painted over the settings
    /// page, so it wins when both somehow exist.
    pub fn new_agent_form(&self) -> Option<&NewAgentForm> {
        self.workbench
            .new_agent
            .as_ref()
            .or(self.workbench.settings.profile_form.as_ref())
    }

    pub(super) fn new_agent_form_mut(&mut self) -> Option<&mut NewAgentForm> {
        self.workbench
            .new_agent
            .as_mut()
            .or(self.workbench.settings.profile_form.as_mut())
    }

    /// Raise the New agent modal, opened on the last thing that worked.
    ///
    /// The three lists it offers are asked for again here, for the reason
    /// [`Self::open_new_agent_menu`] asks: a harness installed, an account signed in or a profile
    /// written since the window opened is offered without a restart.
    ///
    /// **Nothing is left saying "choose…" that can be answered from what the window already
    /// knows.** The target comes from the last start, which brings the harness, the identity and —
    /// through the catalogue the pick asks for — the model and the level with it; the mode and the
    /// ceiling are read back on top, because the host remembers neither. A last start naming a
    /// harness this machine no longer has, or a profile since deleted, answers nothing rather than
    /// opening the form on something that would fail as a spawn.
    pub fn open_new_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench.new_agent = Some(NewAgentForm::new(Purpose::Start));
        self.set_new_agent_prompt("", window, cx);
        self.bus.send(Message::ListAgentTypes);
        self.bus.send(Message::ListAccounts);
        self.bus.send(Message::ListProfiles);
        // And what this build can inject. Asked here rather than in
        // [`Self::probe_new_agent_catalogue`] because it is not a question about the harness: the
        // answer is the same for every start, so it is asked once per opening of the form and kept
        // on the window.
        self.bus.send(Message::ListMcps);
        if let Some(target) = self.last_start_target() {
            self.pick_new_agent_target(target, window, cx);
            let last = self.workbench.last_start.clone();
            if let (Some(last), Some(form)) = (last, self.new_agent_form_mut()) {
                if last.mode.is_some() {
                    form.mode = last.mode;
                }
                form.max_subagents = last.max_subagents;
            }
        }
        // The keyboard goes where the typing goes. It also puts the modal on the focus path, which
        // is what lets ⌘⏎ confirm the form from inside the field — see `ui::new_agent::confirmable`.
        let prompt = self.new_agent_prompt.read(cx).focus_handle(cx);
        window.focus(&prompt, cx);
        cx.notify();
    }

    /// The target the last start would be, where it still resolves to something startable.
    ///
    /// A profile wins over the pair it was started from: it is the more specific answer, and it is
    /// what the user picked.
    fn last_start_target(&self) -> Option<Target> {
        let last = self.workbench.last_start.as_ref()?;
        if let Some(profile) = &last.profile
            && self
                .workbench
                .settings
                .profiles
                .iter()
                .any(|it| it.id == *profile)
        {
            return Some(Target::Profile(profile.clone()));
        }
        let harness = self
            .workbench
            .agent_types
            .iter()
            .find(|it| it.id == last.agent_type && it.available && it.chat)?;
        // The target list offers pairs, so a remembered start with no identity has no row to open
        // on — and picking it would leave the identity row, which is not drawn for a pair, as the
        // one answer the form could no longer show.
        let account = last.account.clone()?;
        self.workbench
            .settings
            .accounts_for(&harness.id)
            .iter()
            .any(|it| it.id == account)
            .then(|| Target::Harness {
                agent_type: harness.id.clone(),
                account: Some(account),
            })
    }

    /// Do what the form is for: start the conversation, or write the profile down.
    ///
    /// One method because one keystroke answers both forms — only one is ever up — and which of
    /// the two it means is the form's own purpose rather than the caller's guess.
    pub fn confirm_new_agent_form(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.new_agent_form() else {
            cx.propagate();
            return;
        };
        // The naming prompt over the form has its own field and its own Save; the form underneath
        // must not answer for it.
        if form.naming {
            cx.propagate();
            return;
        }
        match form.purpose {
            Purpose::Start => {
                self.start_new_agent(cx);
            }
            Purpose::Profile => self.save_new_agent_profile(cx),
        }
    }

    /// Dismiss the form without starting anything — and forget where the start was aimed.
    ///
    /// Cancelling has to leave nothing behind: no tab, no column, and no claim on the next
    /// conversation to arrive from somewhere else. Since nothing is created until
    /// `Message::ConversationStarted` lands, dropping the aim is the whole of it.
    pub fn close_new_agent(&mut self, cx: &mut Context<Self>) {
        self.workbench.new_agent = None;
        self.clear_aim();
        cx.notify();
    }

    /// Answer the first question. A bare harness is its own answer; a profile carries every other
    /// answer with it, which is the whole point of having one.
    pub fn pick_new_agent_target(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let profile = match &target {
            Target::Profile(id) => self
                .workbench
                .settings
                .profiles
                .iter()
                .find(|it| it.id == *id)
                .cloned(),
            Target::Harness { .. } => None,
        };
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        let purpose = form.purpose;
        match (&target, profile) {
            (_, Some(profile)) => {
                let models = std::mem::take(&mut form.models);
                *form = NewAgentForm {
                    models,
                    ..NewAgentForm::from_profile(&profile, purpose)
                };
            }
            (
                Target::Harness {
                    agent_type,
                    account,
                },
                None,
            ) => {
                let (agent_type, account) = (agent_type.clone(), account.clone());
                *form = NewAgentForm {
                    target: Some(target.clone()),
                    agent_type,
                    account,
                    ..NewAgentForm::new(purpose)
                };
            }
            // A profile the host has since dropped: the row is gone by the next answer, and until
            // then picking it answers nothing rather than starting something unnamed.
            (Target::Profile(_), None) => return,
        }
        let prompt = self.new_agent_form().map(|it| it.prompt.clone());
        if let Some(prompt) = prompt {
            self.set_new_agent_prompt(&prompt, window, cx);
        }
        self.default_new_agent_mode();
        self.probe_new_agent_catalogue(cx);
        self.close_new_agent_list(window, cx);
    }

    /// Pick the harness. Everything the harness owns goes with it: a mode, a model and a level
    /// named by the old one mean nothing to the new one, and an identity may not be signed in
    /// there at all.
    pub fn pick_new_agent_harness(
        &mut self,
        agent_type: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pick_new_agent_pair(agent_type, None, window, cx)
    }

    /// Pick the harness **and** the identity, which are one question and so one control.
    ///
    /// A harness is only startable as somebody, and the first row already offers the two together
    /// — asking them again as two rows made the override a profile allows read as two decisions
    /// where the start reads as one. Everything the pair narrows is dropped: a model list belongs
    /// to the harness that answered it, and a level belongs to a model.
    pub fn pick_new_agent_pair(
        &mut self,
        agent_type: String,
        account: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        if form.agent_type == agent_type && form.account == account {
            self.close_new_agent_list(window, cx);
            return;
        }
        form.agent_type = agent_type;
        form.account = account;
        // The target names the harness it was picked as; a profile's harness row may say otherwise,
        // and the target stays what it is — a profile with a harness overridden is still that
        // profile.

        form.model = None;
        form.thinking = None;
        form.mode = None;
        form.models.clear();
        self.default_new_agent_mode();
        self.probe_new_agent_catalogue(cx);
        self.close_new_agent_list(window, cx);
    }

    /// Pick the model. The level goes with it — the levels on offer are the model's own — and
    /// falls to whatever that model starts at.
    pub fn pick_new_agent_model(
        &mut self,
        model: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.new_agent_form_mut() {
            let level = form
                .models
                .iter()
                .find(|it| it.id == model)
                .and_then(|it| it.default_level.clone());
            form.model = Some(model);
            form.thinking = level;
        }
        self.close_new_agent_list(window, cx);
    }

    pub fn pick_new_agent_thinking(
        &mut self,
        thinking: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.new_agent_form_mut() {
            form.thinking = thinking;
        }
        self.close_new_agent_list(window, cx);
    }

    pub fn pick_new_agent_mode(
        &mut self,
        mode: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.new_agent_form_mut() {
            form.mode = mode;
        }
        self.close_new_agent_list(window, cx);
    }

    pub fn pick_new_agent_subagents(
        &mut self,
        max: Option<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.new_agent_form_mut() {
            form.max_subagents = max;
        }
        self.close_new_agent_list(window, cx);
    }

    /// Tick or untick one MCP server. Unlike every other answer here the list stays down: a
    /// checklist is filled in with several gestures, and one that closed on the first tick would
    /// have to be reopened for each of the rest.
    pub fn toggle_new_agent_mcp(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.toggle_mcp(&name);
        }
        cx.notify();
    }

    /// Open one of the form's lists, or close it if it is the one already down — exactly one is
    /// ever open, and the filter field they share is cleared and focused on the way, the rule
    /// [`Self::open_picker_menu`] says once for every picker that opts into search.
    pub fn toggle_new_agent_list(
        &mut self,
        list: OpenList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(open) = self.new_agent_form().map(|form| form.open) else {
            return;
        };
        if open == Some(list) {
            return self.close_new_agent_list(window, cx);
        }
        if let Some(form) = self.new_agent_form_mut() {
            form.open = Some(list);
        }
        if list.has_filter() {
            let search = self.picker_search.clone();
            search.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    pub fn dismiss_new_agent_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_new_agent_list(window, cx);
    }

    /// The list goes down and the keyboard comes back to the form.
    ///
    /// **Giving it back is not a nicety.** The filter field every list carries holds the keyboard
    /// while the list is open, and it is unmounted with the list — a focus handle on an element
    /// nothing draws any more is a keyboard nobody owns, and the window's own keys never arrive
    /// at it: Escape stops closing the form and ⌘⏎ stops starting it. The opening prompt is the
    /// form's keyboard rest, the same one [`Self::open_new_agent`] opens on.
    fn close_new_agent_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.open = None;
        }
        let prompt = self.new_agent_prompt.read(cx).focus_handle(cx);
        window.focus(&prompt, cx);
        cx.notify();
    }

    /// Take the form, validate it against what the window actually has, and answer the project,
    /// the form and the profile a start resolves to — or put the form back and answer nothing.
    ///
    /// Shared by [`Self::start_new_agent`] and [`Self::start_new_agent_in_terminal`]: both read the
    /// same target out of the same form and refuse the same two ways, and only what they build from
    /// the answer differs — a conversation for one, a bare pane for the other.
    fn take_startable_new_agent(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(NewAgentForm, ProjectId, Option<String>)> {
        let prompt = self.new_agent_prompt.read(cx).value().to_string();
        let project_id = self.project(cx)?;
        let mut form = self.workbench.new_agent.take()?;
        form.prompt = prompt;
        let Some(target) = form.target.clone() else {
            // Nothing chosen: the button is drawn faint and does nothing, and this is its brace.
            self.workbench.new_agent = Some(form);
            return None;
        };
        // A harness that is not installed here is drawn disabled in the target list; a profile
        // naming one is the same case, and this is what stops either becoming a start that fails
        // as a spawn the user has to interpret.
        if !self
            .workbench
            .agent_types
            .iter()
            .any(|info| info.id == form.agent_type && info.available)
        {
            self.workbench.new_agent = Some(form);
            return None;
        }
        let profile = match &target {
            Target::Profile(id) => Some(id.clone()),
            Target::Harness { .. } => None,
        };
        Some((form, project_id, profile))
    }

    /// Start the conversation the form describes, and answer the id it was given.
    ///
    /// The id is minted here rather than by the host, the way every other start in the window
    /// mints one: it is what the answer is matched against. An empty string is "the harness's
    /// own" — the convention the host already reads `chosen_model` by — so a row left unanswered
    /// says nothing rather than naming a default the interface invented.
    pub fn start_new_agent(&mut self, cx: &mut Context<Self>) -> Option<AgentId> {
        let (form, project_id, profile) = self.take_startable_new_agent(cx)?;
        let agent_id = AgentId::generate();
        self.bus.send(Message::StartConversation {
            agent_id,
            project_id,
            session_id: self.session,
            rel_path: None,
            agent_type: form.agent_type.clone(),
            account: form.account.clone(),
            profile: profile.clone(),
            model: Some(form.model.clone().unwrap_or_default()),
            thinking: Some(form.thinking.clone().unwrap_or_default()),
            mode: Some(form.mode.clone().unwrap_or_default()),
            mcps: form.mcps.clone(),
        });
        // **No turn goes out here.** The ceiling and the opening prompt are held, and the
        // composer's send path folds them into the first thing the user actually says — a
        // transcript that opens on a directive the user never wrote reads as the conversation
        // beginning with someone else's words. See `state::new_agent::fold_preamble`.
        if let Some(preamble) = form.preamble() {
            self.workbench.agent_preambles.insert(agent_id, preamble);
        }
        self.remember_harness_choice(
            &form.agent_type.clone(),
            form.account.as_deref(),
            profile.as_deref(),
            form.mode.as_deref(),
            form.max_subagents,
            cx,
        );
        cx.notify();
        Some(agent_id)
    }

    /// Start the same harness, the same identity and the same levers, but as a real terminal pane
    /// rather than a structured conversation. The pane arrives as `Message::WorkspaceSpawned`
    /// rather than as an id minted here, so there is nothing this returns.
    ///
    /// **No preamble is stashed.** `form.preamble()` exists for a composer to fold into a first
    /// turn, and a terminal pane has no composer — the user types straight into the harness, so
    /// there is no first turn to fold anything in front of. The opening prompt and the subagent
    /// ceiling the form carries are simply not said.
    pub fn start_new_agent_in_terminal(&mut self, cx: &mut Context<Self>) {
        let Some((form, _project_id, profile)) = self.take_startable_new_agent(cx) else {
            return;
        };
        let picks = AgentPicks {
            account: form.account.clone(),
            profile: profile.clone(),
            model: Some(form.model.clone().unwrap_or_default()),
            thinking: Some(form.thinking.clone().unwrap_or_default()),
            mode: Some(form.mode.clone().unwrap_or_default()),
            mcps: form.mcps.clone(),
        };
        self.spawn_pane(Some(form.agent_type.clone()), Vec::new(), picks, cx);
        // **Release the aim.** A form raised from a chat header or the sink wrote down where the
        // conversation it was about to produce should land, and the only place that claim is spent
        // is the `ConversationStarted` arm in `wire`. This start produces a pane instead, so
        // leaving the claim armed would hand the next conversation from anywhere else to a surface
        // that asked for this one — see [`Self::clear_aim`].
        self.clear_aim();
        self.remember_harness_choice(
            &form.agent_type.clone(),
            form.account.as_deref(),
            profile.as_deref(),
            form.mode.as_deref(),
            form.max_subagents,
            cx,
        );
        cx.notify();
    }

    /// Take what a start still owes this conversation, if anything.
    ///
    /// Taken rather than read: a preamble is said once, in front of the first turn the user sends,
    /// and a second turn is the user's own words alone. Called by [`Self::send_prompt`] — see
    /// `crate::state::WorkbenchState::agent_preambles`.
    pub fn take_agent_preamble(&mut self, agent_id: AgentId) -> Option<String> {
        self.workbench.agent_preambles.remove(&agent_id)
    }

    /// Take one turn to a harness — **the only place `Message::PromptAgent` is built.**
    ///
    /// The held preamble goes in front of the first turn an agent receives, and the transcript
    /// still shows only what the user typed, because nothing is appended here: the line is drawn
    /// when the harness echoes it back. One helper rather than a fold at each `bus.send`, because
    /// two folds are two chances to say the directive twice or never — and the queued turn that
    /// leaves from `wire`'s `ConversationUpdate` arm is a first turn as readily as the composer's.
    ///
    /// The preamble sits **outside** `Conversation::compose_prompt`: that composes what the user
    /// said with what the user attached, and the directive is neither.
    /// A harness echoes the turn it was handed, verbatim, and that echo is what the transcript
    /// draws — so the conversation is told what was folded in, and takes it back off the echo.
    pub(super) fn send_prompt(&mut self, agent_id: AgentId, text: String) {
        let text = match self.take_agent_preamble(agent_id) {
            Some(preamble) => {
                let folded = fold_preamble(&preamble, &text);
                // `fold_preamble` puts the *trimmed* typed half in, so that is what the fold ends
                // with — stripping the untrimmed text would miss on any message the user began
                // with a newline, and a missed front arms nothing and shows the directive.
                let front = folded
                    .strip_suffix(text.trim_start())
                    .unwrap_or_default()
                    .to_string();
                if let Some(conversation) = self
                    .projects
                    .values_mut()
                    .find_map(|open| open.conversations.get_mut(&agent_id))
                {
                    conversation.expect_preamble(&front);
                }
                folded
            }
            None => text,
        };
        self.bus.send(Message::PromptAgent { agent_id, text });
    }

    /// Ask what to call this setup. A start form has no name field of its own, so Save profile
    /// raises the window's prompt over it rather than growing a row nothing else uses.
    pub fn open_new_agent_naming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.profile_id_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        if let Some(form) = &mut self.workbench.new_agent {
            form.naming = true;
        }
        cx.notify();
    }

    pub fn close_new_agent_naming(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.workbench.new_agent {
            form.naming = false;
        }
        cx.notify();
    }

    /// Write the form down as a profile, under the name in the name field.
    ///
    /// The host answers with `Profiles`, or with `AccountError` when the id is not a name it can
    /// file — profiles are stored beside accounts and fail the same way.
    pub fn save_new_agent_profile(&mut self, cx: &mut Context<Self>) {
        let id = self.profile_id_input.read(cx).value().trim().to_string();
        let prompt = self.new_agent_prompt.read(cx).value().to_string();
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        form.prompt = prompt;
        if id.is_empty() || form.agent_type.is_empty() {
            return;
        }
        form.naming = false;
        let profile = form.as_profile(id.clone());
        // A start form that has just written a profile is pointed at it: what it offers has not
        // changed, but what a second Save would overwrite now has a name.
        if self.workbench.new_agent.is_some()
            && let Some(form) = self.new_agent_form_mut()
        {
            form.target = Some(Target::Profile(id));
        }
        self.bus.send(Message::SaveProfile { profile });
        cx.notify();
    }

    /// Ask the host what this harness offers. Cheap after the first ask — the host caches the
    /// probe on the harness binary's own version — so every change of harness or identity asks
    /// again rather than drawing the last one's models.
    pub(super) fn probe_new_agent_catalogue(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        if form.agent_type.is_empty() {
            return;
        }
        let agent_type = form.agent_type.clone();
        let account = form.account.clone();
        form.probing = true;
        form.models.clear();
        self.bus.send(Message::ListHarnessCatalogue {
            agent_type,
            account,
        });
        cx.notify();
    }

    /// Fall the mode to the harness's own "ask nothing", which is what the user asked a new agent
    /// to start on. A harness that names no such mode keeps whatever the form already had.
    fn default_new_agent_mode(&mut self) {
        let Some(form) = self.new_agent_form() else {
            return;
        };
        if form.mode.is_some() {
            return;
        }
        let mode = self
            .workbench
            .agent_types
            .iter()
            .find(|it| it.id == form.agent_type)
            .and_then(NewAgentForm::default_mode);
        if let Some(form) = self.new_agent_form_mut() {
            form.mode = mode;
        }
    }

    /// Seed the shared opening-prompt field. Its own method because three callers need it and
    /// each is somewhere the field itself is not in scope.
    pub(super) fn set_new_agent_prompt(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.new_agent_prompt.clone();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
    }
}
