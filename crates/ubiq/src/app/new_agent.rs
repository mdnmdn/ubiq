//! The New agent modal's mutators — and the settings page's definition form's, because both are
//! drawn from one [`NewAgentForm`] and a shared body needs shared listeners.
//!
//! Only one of the two is ever up, so every mutator here reaches for whichever it is rather than
//! taking a discriminator: two ways to answer the same question is how the two forms would drift
//! apart again.

use super::*;
use crate::state::new_agent::{
    NewAgentForm, NewAgentOpen, NewAgentTab, OpenList, Purpose, TASK_ASSIGN_MCPS, Target,
    fold_preamble, task_assignment_prompt, task_feedback_prefix,
};
use crate::state::prefs::LastStart;
use ubiq_proto::messages::{TAG_COORDINATOR, TAG_WORKER};

impl AppState {
    /// The form on screen, whichever raised it. The New agent modal is painted over the settings
    /// page, so it wins when both somehow exist.
    pub fn new_agent_form(&self) -> Option<&NewAgentForm> {
        self.workbench
            .new_agent
            .as_ref()
            .or(self.workbench.settings.definition_form.as_ref())
    }

    pub(super) fn new_agent_form_mut(&mut self) -> Option<&mut NewAgentForm> {
        self.workbench
            .new_agent
            .as_mut()
            .or(self.workbench.settings.definition_form.as_mut())
    }

    /// Raise the New agent modal, opened on the last thing that worked.
    ///
    /// The three lists it offers are asked for again here, for the reason
    /// [`Self::open_new_agent_menu`] asks: a harness installed, an account signed in or a definition
    /// written since the window opened is offered without a restart.
    ///
    /// **Nothing is left saying "choose…" that can be answered from what the window already
    /// knows.** The target comes from the last start, which brings the harness, the identity and —
    /// through the catalogue the pick asks for — the model and the level with it; the mode and the
    /// ceiling are read back on top, because the host remembers neither. A last start naming a
    /// harness this machine no longer has, or a definition since deleted, answers nothing rather than
    /// opening the form on something that would fail as a spawn. The last start is this project's
    /// own where it has one, the machine-wide one where it does not.
    ///
    /// `open` narrows and short-cuts it ([`NewAgentOpen`]): with tags, only definitions carrying
    /// one are offered and the Harness tab is gone; with `autostart`, a target that resolves is
    /// started at once and the dialog never shows — it shows only when nothing resolves.
    pub fn open_new_agent(
        &mut self,
        open: NewAgentOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut form = NewAgentForm::new(Purpose::Start);
        form.tag_filter = open.tags.unwrap_or_default();
        form.opened_mcps = open.mcps;
        form.tick_opened_mcps();
        self.workbench.new_agent = Some(form);
        self.set_new_agent_prompt("", window, cx);
        let initial = open.initial_prompt.unwrap_or_default();
        self.set_new_agent_initial_prompt(&initial, window, cx);
        self.set_new_agent_description("", window, cx);
        self.new_agent_tag_input
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.bus.send(Message::ListAgentTypes);
        self.bus.send(Message::ListAccounts);
        self.bus.send(Message::ListAgentDefinitions);
        // And what this build can inject. Asked here rather than in
        // [`Self::probe_new_agent_catalogue`] because it is not a question about the harness: the
        // answer is the same for every start, so it is asked once per opening of the form and kept
        // on the window.
        self.bus.send(Message::ListMcps);
        // The skills and catalog servers the two checklists offer, from both layers.
        self.ask_catalog_for_form(cx);
        let resolved = self.last_start_target(cx);
        let resolved_any = resolved.is_some();
        if let Some((target, last)) = resolved {
            // The form opens on the tab that asks the question the last start answered: seeding a
            // harness onto the Agents tab would leave its dropdown reading "choose an agent" with
            // the answer sitting under the other tab.
            if let Some(form) = self.workbench.new_agent.as_mut() {
                form.tab = match target {
                    Target::AgentDefinition(_) => NewAgentTab::Agents,
                    Target::Harness { .. } => NewAgentTab::Harness,
                };
            }
            self.pick_new_agent_target(target, window, cx);
            if let (Some(last), Some(form)) = (last, self.new_agent_form_mut()) {
                if last.mode.is_some() {
                    form.mode = last.mode;
                }
                form.max_subagents = last.max_subagents;
            }
        } else if let Some(form) = self.workbench.new_agent.as_mut()
            && !form.tag_filter.is_empty()
        {
            form.tab = NewAgentTab::Agents;
        }
        // Started without a dialog. A start that is refused puts the form back, so the dialog is
        // what the user sees then, exactly as when nothing resolved at all.
        if open.autostart && resolved_any && self.start_new_agent(cx).is_some() {
            cx.notify();
            return;
        }
        // The keyboard goes where the typing goes. It also puts the modal on the focus path, which
        // is what lets ⌘⏎ confirm the form from inside the field — see `ui::new_agent::confirmable`.
        let prompt = self.new_agent_rest_focus(cx);
        window.focus(&prompt, cx);
        cx.notify();
    }

    /// Raise the New agent modal, pre-filled for the board's "Assign to an agent" button —
    /// `T-64`. **The same form and the same open as [`Self::open_new_agent`]**, not a second
    /// mechanism: this calls it whole, then overlays the three things a task assignment answers
    /// that an ordinary start leaves blank — the MCP checklist, the two checkboxes, and the
    /// opening prompt they compose.
    ///
    /// **Aimed at the chat surface**, the way [`Self::open_new_agent_direct`] aims IDE's own `+`
    /// — so the agent this starts lands in a `Chat` tab in the right dock, beside the task it was
    /// assigned from, which is the whole point of `T-100`'s `+` living there too.
    ///
    /// Silently does nothing for a task the project no longer holds — the button that reaches
    /// this is drawn from a `TaskRecord` already in hand, so that is a race with a delete rather
    /// than a caller's mistake.
    ///
    /// **Narrowed to the task-fit definitions** — those tagged `coordinator` or `worker` (T-320) —
    /// and the Start it ends in links the agent to the task and names it by the task's key: see
    /// [`Self::start_new_agent`].
    pub fn assign_task_to_agent(
        &mut self,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tags = vec![TAG_COORDINATOR.to_string(), TAG_WORKER.to_string()];
        self.prefill_task_assignment(task_id, Some(tags), None, window, cx);
    }

    /// The task panel's *Feedback to an agent* when nobody linked can take it: the assignment
    /// dialog, its opening prompt followed by the review line for the user to finish.
    pub fn assign_task_with_feedback(
        &mut self,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(label) = self.task_label(task_id, cx) else {
            return;
        };
        let tags = vec![TAG_COORDINATOR.to_string(), TAG_WORKER.to_string()];
        let suffix = format!("\n\n{}", task_feedback_prefix(&label));
        self.prefill_task_assignment(task_id, Some(tags), Some(suffix), window, cx);
    }

    /// The task's label in a prompt: its key, else its id — an agent has to be able to find it.
    fn task_label(&self, task_id: TaskId, cx: &App) -> Option<String> {
        self.work(cx)
            .and_then(|work| work.task(task_id))
            .map(|task| task.key.clone().unwrap_or_else(|| task_id.to_string()))
    }

    /// The shared pre-fill behind [`Self::assign_task_to_agent`] and the mission's *Any agent*
    /// (which passes no tag filter). `suffix` is appended to the composed opening prompt.
    pub(super) fn prefill_task_assignment(
        &mut self,
        task_id: TaskId,
        tags: Option<Vec<String>>,
        suffix: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(label) = self.task_label(task_id, cx) else {
            return;
        };
        let suffix_text = suffix.unwrap_or_default();
        let prompt = format!("{}{suffix_text}", task_assignment_prompt(&label, true, false));
        self.aim_start(NewAgentSurface::Chat, cx);
        self.open_new_agent(
            NewAgentOpen {
                tags,
                autostart: false,
                initial_prompt: Some(prompt),
                ..NewAgentOpen::default()
            },
            window,
            cx,
        );
        if let Some(form) = self.new_agent_form_mut() {
            form.for_task = Some(task_id);
            form.ask_for_feedback = true;
            form.plan_mode = false;
            form.prompt_suffix = suffix_text;
            for mcp in TASK_ASSIGN_MCPS {
                if !form.mcps.iter().any(|it| it == mcp) {
                    form.mcps.push(mcp.to_string());
                }
            }
        }
    }

    /// Flip the task assignment's "ask for feedback" checkbox, and recompose the opening prompt
    /// so the field on screen always says what the checkboxes say. See
    /// [`crate::state::new_agent::task_assignment_prompt`].
    pub fn toggle_new_agent_ask_feedback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        form.ask_for_feedback = !form.ask_for_feedback;
        self.resync_task_assignment_prompt(window, cx);
    }

    /// Flip the task assignment's "plan mode" checkbox, for the same reason and the same way.
    pub fn toggle_new_agent_plan_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        form.plan_mode = !form.plan_mode;
        self.resync_task_assignment_prompt(window, cx);
    }

    /// Recompose the opening prompt from the task label and the two checkboxes, and write it into
    /// the field the form's textarea reads. A no-op where the form is not a task assignment at
    /// all — the ordinary New agent form has nothing here to keep in sync.
    fn resync_task_assignment_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.new_agent_form() else {
            return;
        };
        let Some(task_id) = form.for_task else {
            return;
        };
        let (ask_for_feedback, plan_mode) = (form.ask_for_feedback, form.plan_mode);
        let suffix = form.prompt_suffix.clone();
        let Some(label) = self
            .work(cx)
            .and_then(|work| work.task(task_id))
            .map(|task| task.key.clone().unwrap_or_else(|| task_id.to_string()))
        else {
            return;
        };
        let prompt = format!(
            "{}{suffix}",
            task_assignment_prompt(&label, ask_for_feedback, plan_mode)
        );
        self.set_new_agent_initial_prompt(&prompt, window, cx);
    }

    /// The target the form opens on, and the remembered start it came from when it came from one.
    ///
    /// The last start in this project, else the machine-wide one, where it still resolves to
    /// something startable and passes the form's tag filter. A form narrowed to tags offers no
    /// bare harness, so when the remembered start is not a definition it passes the first
    /// definition that does — with no remembered start behind it, so no mode or ceiling either.
    fn last_start_target(&self, cx: &App) -> Option<(Target, Option<LastStart>)> {
        let filter = self
            .workbench
            .new_agent
            .as_ref()
            .map(|form| form.tag_filter.clone())
            .unwrap_or_default();
        let scope = self.new_agent_scope(cx);
        let last = scope
            .and_then(|project| {
                self.workbench
                    .last_start_by_project
                    .get(&project.to_string())
            })
            .or(self.workbench.last_start.as_ref());
        let remembered = last.and_then(|last| {
            self.remembered_target(last, scope, &filter)
                .map(|target| (target, Some(last.clone())))
        });
        if remembered.is_some() || filter.is_empty() {
            return remembered;
        }
        self.workbench
            .settings
            .startable_definitions_in(scope)
            .into_iter()
            .find(|it| it.has_any_tag(&filter))
            .map(|it| (Target::AgentDefinition(it.id), None))
    }

    /// What one remembered start resolves to in `scope`, under `filter`.
    ///
    /// A definition wins over the pair it was started from: it is the more specific answer, and it is
    /// what the user picked. It has to be the same definition — found in the root it was started
    /// from, since a project's `review` and the global `review` are two setups — and still on.
    fn remembered_target(
        &self,
        last: &LastStart,
        scope: Option<ProjectId>,
        filter: &[String],
    ) -> Option<Target> {
        if let Some(definition) = &last.definition
            && let Some(found) = self
                .workbench
                .settings
                .startable_definitions_in(scope)
                .into_iter()
                .find(|it| it.id == *definition && it.project == last.definition_project)
            && (filter.is_empty() || found.has_any_tag(filter))
        {
            return Some(Target::AgentDefinition(definition.clone()));
        }
        if !filter.is_empty() {
            return None;
        }
        let harness = self.workbench.agent_types.iter().find(|it| {
            it.id == last.agent_type
                && it.available
                && it.chat
                && self.workbench.harness_offered(it)
        })?;
        // No identity is a row of its own now — the `Default` group — so a remembered start that
        // named none reopens on it. One that named an identity still has to find it, and not
        // because the list would fail to draw it: `harness_choices` pins whatever a form holds, so
        // a signed-out pair *would* be drawn, under `Already chosen`. It is that the pair is no
        // longer startable — there is no login behind it — and reopening the form on it would
        // offer a start that cannot begin. So the form opens unanswered instead, which is the
        // honest state.
        let Some(account) = last.account.clone() else {
            return Some(Target::Harness {
                agent_type: harness.id.clone(),
                account: None,
            });
        };
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

    /// Do what the form is for: start the conversation, or write the definition down.
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
        // The folders dialog's field answers Enter with Add, and the form stays put.
        if form.policies {
            self.add_new_agent_grant(_window, cx);
            return;
        }
        match form.purpose {
            Purpose::Start => {
                self.start_new_agent(cx);
            }
            Purpose::AgentDefinition => self.save_new_agent_definition(cx),
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

    /// Answer the first question. A bare harness is its own answer; a definition carries every other
    /// answer with it, which is the whole point of having one.
    pub fn pick_new_agent_target(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let definition = match &target {
            // Resolved against what this start is aimed at, not against the global list alone: a
            // project's own setup is offered in the picker, so it has to be findable by the pick.
            Target::AgentDefinition(id) => self
                .workbench
                .settings
                .definitions_in(self.new_agent_scope(cx))
                .into_iter()
                .find(|it| it.id == *id),
            Target::Harness { .. } => None,
        };
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        let purpose = form.purpose;
        // The tab is the question being asked, not part of the answer: picking on one must not
        // move the form to the other. A customization belongs to the answer it was made against,
        // so it goes with the answer.
        let tab = form.tab;
        // How the form was opened is not part of any answer either.
        let tag_filter = form.tag_filter.clone();
        let opened_mcps = form.opened_mcps.clone();
        // Nor is the task it was raised for: a task assignment keeps its task, its two checkboxes
        // and the servers it starts with whichever target is picked — dropping them is a Start
        // that never links the agent to the card (T-320), the reason `set_new_agent_tab` keeps
        // them too.
        let for_task = form.for_task;
        let (ask_for_feedback, plan_mode) = (form.ask_for_feedback, form.plan_mode);
        match (&target, definition) {
            (_, Some(definition)) => {
                let models = std::mem::take(&mut form.models);
                *form = NewAgentForm {
                    models,
                    tab,
                    tag_filter,
                    opened_mcps,
                    ..NewAgentForm::from_definition(&definition, purpose)
                };
            }
            (
                Target::Harness {
                    agent_type,
                    account,
                },
                None,
            ) => {
                // Picking the answer the form already holds is a no-op, exactly as it is on the
                // harness override ([`Self::pick_new_agent_pair`]). Without this the arm below
                // rebuilt the form from defaults, so re-picking your own harness silently threw
                // away the model, the level, the mode, the subagent cap, the MCPs, the skills, the
                // prompt, the description, the scope and the switched-off flag — and a definition
                // form then saved that gutted record over the one being edited. The guard is on
                // the whole target, not on the pair alone: a definition form whose harness
                // override happens to name the same pair is still being re-targeted onto the bare
                // harness, and that is a real change.
                if form.target.as_ref() == Some(&target) {
                    self.close_new_agent_list(window, cx);
                    return;
                }
                let (agent_type, account) = (agent_type.clone(), account.clone());
                *form = NewAgentForm {
                    target: Some(target.clone()),
                    agent_type,
                    account,
                    tab,
                    tag_filter,
                    opened_mcps,
                    ..NewAgentForm::new(purpose)
                };
            }
            // A definition the host has since dropped: the row is gone by the next answer, and until
            // then picking it answers nothing rather than starting something unnamed.
            (Target::AgentDefinition(_), None) => return,
        }
        if let Some(form) = self.new_agent_form_mut() {
            form.tick_opened_mcps();
        }
        if for_task.is_some()
            && let Some(form) = self.new_agent_form_mut()
        {
            form.for_task = for_task;
            form.ask_for_feedback = ask_for_feedback;
            form.plan_mode = plan_mode;
            for mcp in TASK_ASSIGN_MCPS {
                if !form.mcps.iter().any(|it| it == mcp) {
                    form.mcps.push(mcp.to_string());
                }
            }
        }
        let prompt = self.new_agent_form().map(|it| it.prompt.clone());
        if let Some(prompt) = prompt {
            self.set_new_agent_prompt(&prompt, window, cx);
        }
        let description = self.new_agent_form().map(|it| it.description.clone());
        if let Some(description) = description {
            self.set_new_agent_description(&description, window, cx);
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
    /// — asking them again as two rows made the override a definition allows read as two decisions
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
        // The target names the harness it was picked as; a definition's harness row may say otherwise,
        // and the target stays what it is — a definition with a harness overridden is still that
        // definition.

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

    /// Flip whether this setup is fit to run as a planning assistant. Drawn only under
    /// `Purpose::AgentDefinition` — see `ui::new_agent::body`'s mission-assistant row.
    pub fn toggle_new_agent_mission_assistant(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.mission_assistant = !form.mission_assistant;
        }
        cx.notify();
    }

    /// Put one tag on the definition, or take it off. A role tag's MCP servers are ticked by the
    /// tag and shown as such; the host re-asserts them on save, so nothing here writes them onto
    /// the checklist.
    pub fn toggle_new_agent_tag(&mut self, tag: String, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.toggle_tag(&tag);
        }
        cx.notify();
    }

    /// Add the tag typed in the tag field, and clear the field.
    pub fn add_new_agent_tag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tag = self.new_agent_tag_input.read(cx).value().to_string();
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        if form.add_tag(&tag) {
            self.new_agent_tag_input
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Switch this definition off, or back on. A disabled definition is still listed and still
    /// editable — it is simply not offered anywhere a run is started from.
    pub fn toggle_new_agent_disabled(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.disabled = !form.disabled;
        }
        cx.notify();
    }

    /// Move the New agent dialog between its two tabs.
    ///
    /// The answer goes with the question: the two tabs ask different things, and a definition
    /// chosen on one is not an answer to the other. A list left down is closed the way any other
    /// answer closes it.
    pub fn set_new_agent_tab(
        &mut self,
        tab: NewAgentTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.workbench.new_agent.as_mut() else {
            return;
        };
        // A form narrowed to tags offers definitions only: there is no Harness tab to move to.
        if form.tab == tab || (!form.tag_filter.is_empty() && tab == NewAgentTab::Harness) {
            return;
        }
        let tag_filter = std::mem::take(&mut form.tag_filter);
        let opened_mcps = std::mem::take(&mut form.opened_mcps);
        let models = std::mem::take(&mut form.models);
        let purpose = form.purpose;
        // What the *question* answered is dropped; what the form was raised for is not. A task
        // assignment's task, its two checkboxes and the servers it starts with belong to the card
        // the form was opened from, not to which tab is in front.
        let (for_task, project) = (form.for_task, form.project);
        let (ask_for_feedback, plan_mode) = (form.ask_for_feedback, form.plan_mode);
        let mcps = match for_task {
            Some(_) => std::mem::take(&mut form.mcps),
            None => Vec::new(),
        };
        *form = NewAgentForm {
            tab,
            models,
            for_task,
            project,
            ask_for_feedback,
            plan_mode,
            mcps,
            tag_filter,
            opened_mcps,
            ..NewAgentForm::new(purpose)
        };
        form.tick_opened_mcps();
        // The initial prompt is the user's own words for this run, not part of either tab's
        // answer, so it stays — except a task assignment's, which is recomposed from the task and
        // the checkboxes, never patched.
        if for_task.is_some() {
            self.resync_task_assignment_prompt(window, cx);
        }
        self.set_new_agent_prompt("", window, cx);
        self.set_new_agent_description("", window, cx);
        self.close_new_agent_list(window, cx);
        cx.notify();
    }

    /// The Agents tab's `Customize`: draw the harness, model and effort the chosen definition
    /// names as rows that can be overridden, rather than as the line on its row.
    pub fn toggle_new_agent_customize(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.workbench.new_agent.as_mut() {
            form.customize = !form.customize;
        }
        cx.notify();
    }

    /// Which project's definitions this start may name: the Teams toolbar's override where it
    /// holds one this window has, else the window's own project. The same reading
    /// `ui::new_agent::target_rows` draws the list from, so the pick resolves against the list
    /// that was offered.
    pub(super) fn new_agent_scope(&self, cx: &App) -> Option<ProjectId> {
        self.new_agent_project
            .filter(|id| self.window_projects(cx).contains(id))
            .or_else(|| self.project(cx))
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
    /// at it: Escape stops closing the form and ⌘⏎ stops starting it. The prompt field is the
    /// form's keyboard rest, the same one [`Self::open_new_agent`] opens on.
    fn close_new_agent_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.open = None;
        }
        let prompt = self.new_agent_rest_focus(cx);
        window.focus(&prompt, cx);
        cx.notify();
    }

    /// Take the form, validate it against what the window actually has, and answer the project,
    /// the form and the definition a start resolves to — or put the form back and answer nothing.
    ///
    /// Shared by [`Self::start_new_agent`] and [`Self::start_new_agent_in_terminal`]: both read the
    /// same target out of the same form and refuse the same two ways, and only what they build from
    /// the answer differs — a conversation for one, a bare pane for the other.
    fn take_startable_new_agent(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(NewAgentForm, ProjectId, Option<String>)> {
        // The Teams toolbar can aim a start at any project the window holds; every other way in
        // aims at the active one and says so by having no project field at all.
        //
        // **The override is checked, not trusted.** A project can be let go between the form going
        // up and the user confirming it, and a `StartConversation` against one this window no
        // longer holds is a conversation with nowhere to land — so a stale aim falls back to the
        // active project rather than being sent.
        let project_id = self
            .new_agent_project
            .filter(|id| self.projects.contains_key(id))
            .or_else(|| self.project(cx))?;
        // `form.prompt` is the chosen definition's own agent prompt, seeded by the pick and never
        // edited on a start — it is what the preamble folds.
        let form = self.workbench.new_agent.take()?;
        let Some(target) = form.target.clone() else {
            // Nothing chosen: the button is drawn faint and does nothing, and this is its brace.
            self.workbench.new_agent = Some(form);
            return None;
        };
        // A harness that is not installed here is drawn disabled in the target list; a definition
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
        let definition = match &target {
            Target::AgentDefinition(id) => Some(id.clone()),
            Target::Harness { .. } => None,
        };
        // Spent here rather than when the answer lands, unlike the two surface aims: this one is
        // read at send time, so once the message carries it there is nothing left to wait for —
        // and a start that leaves it standing would file the *next* form's start in the same
        // project. The refusals above are before this on purpose: they put the form back up, and
        // the project it is still being composed against goes back with it.
        self.new_agent_project = None;
        Some((form, project_id, definition))
    }

    /// Start the conversation the form describes, and answer the id it was given.
    ///
    /// The id is minted here rather than by the host, the way every other start in the window
    /// mints one: it is what the answer is matched against. An empty string is "the harness's
    /// own" — the convention the host already reads `chosen_model` by — so a row left unanswered
    /// says nothing rather than naming a default the interface invented.
    pub fn start_new_agent(&mut self, cx: &mut Context<Self>) -> Option<AgentId> {
        let initial = self
            .new_agent_initial_prompt
            .read(cx)
            .value()
            .trim()
            .to_string();
        let (form, project_id, definition) = self.take_startable_new_agent(cx)?;
        let agent_id = AgentId::generate();
        self.bus.send(Message::StartConversation {
            agent_id,
            project_id,
            session_id: self.session,
            rel_path: None,
            agent_type: form.agent_type.clone(),
            account: form.account.clone(),
            definition: definition.clone(),
            model: Some(form.model.clone().unwrap_or_default()),
            thinking: Some(form.thinking.clone().unwrap_or_default()),
            mode: Some(form.mode.clone().unwrap_or_default()),
            mcps: form.mcps.clone(),
            skills: form.skills.clone(),
            grants: form.grants.clone(),
            // The user started this one, so nobody asked for it. `spawned_by` is only ever set
            // where a window answers a `MissionSpawnRequest`.
            spawned_by: None,
        });
        // **No turn goes out here.** The ceiling and the agent prompt are held, and the send path
        // folds them into the first thing the user actually says — a transcript that opens on a
        // directive the user never wrote reads as the conversation beginning with someone else's
        // words. See `state::new_agent::fold_preamble`.
        if let Some(preamble) = form.preamble() {
            self.workbench.agent_preambles.insert(agent_id, preamble);
        }
        // An initial prompt *is* the user's first turn. Parked until the conversation exists —
        // the `ConversationStarted` arm sends it through `send_prompt`, so the preamble is folded
        // in front of it and the transcript knows to take that front back off the echo.
        if !initial.is_empty() {
            self.workbench
                .agent_initial_prompts
                .insert(agent_id, initial);
        }
        // A task assignment links the agent to its task and names it after the card — both owed
        // until `ConversationStarted`, when there is a `WorkAgent` to assign and to rename (T-320).
        if let Some(task_id) = form.for_task {
            self.workbench
                .agent_assignments
                .insert(agent_id, (project_id, task_id));
            if let Some(name) = self
                .projects
                .get(&project_id)
                .and_then(|open| open.work.task(task_id))
                .map(task_agent_name)
            {
                self.workbench.agent_names.insert(agent_id, name);
            }
        }
        self.remember_harness_choice(
            project_id,
            &form.agent_type.clone(),
            form.account.as_deref(),
            definition.as_deref(),
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
        let Some((form, project_id, definition)) = self.take_startable_new_agent(cx) else {
            return;
        };
        let picks = AgentPicks {
            account: form.account.clone(),
            definition: definition.clone(),
            model: Some(form.model.clone().unwrap_or_default()),
            thinking: Some(form.thinking.clone().unwrap_or_default()),
            mode: Some(form.mode.clone().unwrap_or_default()),
            mcps: form.mcps.clone(),
            skills: form.skills.clone(),
            grants: form.grants.clone(),
        };
        self.spawn_pane(Some(form.agent_type.clone()), Vec::new(), picks, cx);
        // **Release the aim.** A form raised from a chat header or the sink wrote down where the
        // conversation it was about to produce should land, and the only place that claim is spent
        // is the `ConversationStarted` arm in `wire`. This start produces a pane instead, so
        // leaving the claim armed would hand the next conversation from anywhere else to a surface
        // that asked for this one — see [`Self::clear_aim`].
        self.clear_aim();
        self.remember_harness_choice(
            project_id,
            &form.agent_type.clone(),
            form.account.as_deref(),
            definition.as_deref(),
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

    /// Ask what to call this setup. A start form has no name field of its own, so Save definition
    /// raises the window's prompt over it rather than growing a row nothing else uses.
    pub fn open_new_agent_naming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.definition_id_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        if let Some(form) = &mut self.workbench.new_agent {
            form.naming = true;
        }
        cx.notify();
    }

    /// Raise the extra-folders dialog over the form.
    pub fn open_new_agent_policies(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.new_agent_grant_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        if let Some(form) = self.new_agent_form_mut() {
            form.policies = true;
            form.grant_invalid = false;
        }
        cx.notify();
    }

    pub fn close_new_agent_policies(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.policies = false;
            form.grant_invalid = false;
        }
        cx.notify();
    }

    /// Add the path in the dialog's field, or say it is refused.
    pub fn add_new_agent_grant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.new_agent_grant_input.read(cx).value().to_string();
        if path.trim().is_empty() {
            return;
        }
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        form.grant_invalid = !form.add_grant(&path);
        if !form.grant_invalid {
            self.new_agent_grant_input
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
    }

    pub fn remove_new_agent_grant(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(form) = self.new_agent_form_mut() {
            form.remove_grant(index);
        }
        cx.notify();
    }

    pub fn close_new_agent_naming(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.workbench.new_agent {
            form.naming = false;
        }
        cx.notify();
    }

    /// Write the form down as a definition, under the name in the name field.
    ///
    /// The host answers with `AgentDefinitions`, or with `AccountError` when the id is not a name it can
    /// file — definitions are stored beside accounts and fail the same way.
    pub fn save_new_agent_definition(&mut self, cx: &mut Context<Self>) {
        let id = self.definition_id_input.read(cx).value().trim().to_string();
        let prompt = self.new_agent_prompt.read(cx).value().to_string();
        let description = self.new_agent_description.read(cx).value().to_string();
        let Some(form) = self.new_agent_form_mut() else {
            return;
        };
        // Only the definition form draws the agent-prompt field. A start form saving its bare
        // harness has none on screen, so it writes what it holds — the initial prompt is a turn
        // for this one run, not something the definition remembers.
        if form.purpose == Purpose::AgentDefinition {
            form.prompt = prompt;
        }
        form.description = description;
        if id.is_empty() || form.agent_type.is_empty() {
            return;
        }
        form.naming = false;
        let definition = form.as_definition(id.clone());
        // A start form that has just written a definition is pointed at it: what it offers has not
        // changed, but what a second Save would overwrite now has a name.
        if self.workbench.new_agent.is_some()
            && let Some(form) = self.new_agent_form_mut()
        {
            form.target = Some(Target::AgentDefinition(id));
        }
        self.bus.send(Message::SaveAgentDefinition { definition });
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

    /// Seed the definition form's agent-prompt field. Its own method because several callers
    /// need it and each is somewhere the field itself is not in scope.
    pub(super) fn set_new_agent_prompt(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.new_agent_prompt.clone();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
    }

    /// Seed the New agent modal's initial-prompt field, on [`Self::set_new_agent_prompt`]'s
    /// footing — a task assignment and a mission spawn pre-fill it with what to start on.
    pub(super) fn set_new_agent_initial_prompt(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.new_agent_initial_prompt.clone();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
    }

    /// Where the form's keyboard rests: the field the user types the words into — the agent
    /// prompt on a definition form, the initial prompt on a start.
    fn new_agent_rest_focus(&self, cx: &App) -> gpui::FocusHandle {
        match self.new_agent_form().map(|form| form.purpose) {
            Some(Purpose::AgentDefinition) => self.new_agent_prompt.read(cx).focus_handle(cx),
            _ => self.new_agent_initial_prompt.read(cx).focus_handle(cx),
        }
    }

    /// Seed the definition form's description field, on [`Self::set_new_agent_prompt`]'s own
    /// footing.
    pub(super) fn set_new_agent_description(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.new_agent_description.clone();
        input.update(cx, |state, cx| state.set_value(text, window, cx));
    }

    /// The Teams toolbar's split button `+` (M15, §9), pressed: New agent.
    ///
    /// Two shapes, because the question only exists when there is more than one answer: a canvas
    /// spanning several projects is asked which one first, through the split button's own menu at
    /// its [`TeamsCreateStage::AgentProject`] stage — `at` is where the `+` was clicked, so the
    /// list hangs off it exactly as the chevron's does. Anything else — the project span, or a
    /// window holding one project — goes straight to the form.
    pub fn open_teams_add_agent(
        &mut self,
        at: (f32, f32),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.teams_project_choice(cx) {
            self.open_teams_create_stage(at, TeamsCreateStage::AgentProject, cx);
            return;
        }
        self.start_teams_agent(None, window, cx);
    }

    /// The split button's chevron (M15, §9), pressed: the fixed choice between the two things it
    /// makes — *New agent* and *New mission*.
    pub fn open_teams_create_menu(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        self.open_teams_create_stage(at, TeamsCreateStage::Kind, cx);
    }

    /// Raise the split button's menu at a given stage, replacing whatever menu was open — the
    /// same manual close-then-set `open_new_agent_menu` uses, because `open_menu` does not know
    /// about the extra state a multi-stage menu carries.
    fn open_teams_create_stage(
        &mut self,
        at: (f32, f32),
        stage: TeamsCreateStage,
        cx: &mut Context<Self>,
    ) {
        if self.workbench.open_menu.is_some() {
            self.close_menu(cx);
        }
        self.workbench.open_menu = Some(MenuId::TeamsCreate);
        self.workbench.teams_create_menu = Some(TeamsCreateMenu { at, stage });
        cx.notify();
    }

    /// One row of the split button's menu, clicked — matched by position and by
    /// [`TeamsCreateMenu::stage`], the same rule every position-matched menu in this window
    /// follows.
    pub fn pick_teams_create_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.workbench.teams_create_menu else {
            return;
        };
        match menu.stage {
            // *New agent* / *New mission* (§9's last bullet, M15). Each asks the same project
            // question the other does, at the project stage below, when the canvas spans more
            // than one.
            TeamsCreateStage::Kind => match index {
                0 => {
                    self.close_menu(cx);
                    self.open_teams_add_agent(menu.at, window, cx);
                }
                1 => {
                    if self.teams_project_choice(cx) {
                        self.workbench.teams_create_menu = Some(TeamsCreateMenu {
                            at: menu.at,
                            stage: TeamsCreateStage::MissionProject,
                        });
                        cx.notify();
                    } else {
                        self.close_menu(cx);
                        self.open_new_mission(window, cx);
                    }
                }
                _ => {}
            },
            TeamsCreateStage::AgentProject => {
                let picked = self.window_projects(cx).get(index).copied();
                self.close_menu(cx);
                let Some(project) = picked else {
                    return;
                };
                self.start_teams_agent(Some(project), window, cx);
            }
            // *New mission* raises the dialog for the project picked — which means pointing the
            // window at it first (`AppState::activate_project`): the dialog and the board flow
            // behind it both read "the project on screen" rather than naming one of their own, the
            // same way every other project-scoped dialog in this window does.
            TeamsCreateStage::MissionProject => {
                let picked = self.window_projects(cx).get(index).copied();
                self.close_menu(cx);
                let Some(project) = picked else {
                    return;
                };
                self.activate_project(project, cx);
                self.open_new_mission(window, cx);
            }
        }
    }

    /// Aim a start at the Teams canvas, in the project named — or in the active one, for a window
    /// with nothing to choose between.
    ///
    /// [`NewAgentSurface::Agents`] is the aim, and it writes nothing: a conversation with no other
    /// claim on it opens a card wherever the work it belongs to is already drawn, which on this
    /// screen is the canvas. It is not a chat tab and not the sink, and neither of those flags may
    /// be left standing behind this button.
    ///
    /// **The order matters.** `aim_start` begins by clearing the aim — the project included — so
    /// the override is written after it, not before.
    fn start_teams_agent(
        &mut self,
        project: Option<ProjectId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.aim_start(NewAgentSurface::Agents, cx);
        self.new_agent_project = project;
        self.open_new_agent(NewAgentOpen::default(), window, cx);
    }
}

/// What an agent assigned to `task` is called: the task's key (`T-320`), else its title cut short
/// — never an id, since this is the agent's label on every surface.
pub(super) fn task_agent_name(task: &ubiq_proto::work::TaskRecord) -> String {
    task.key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| super::board::stand_in_title(&task.title))
}
