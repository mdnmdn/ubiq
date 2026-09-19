use super::*;

use ubiq_proto::work::AgentId;

use crate::state::conversation::Conversation;

impl AppState {
    pub fn set_sink_section(&mut self, section: SinkSection, cx: &mut Context<Self>) {
        self.sink.section = section;
        if section == SinkSection::Messages {
            self.watch_tape(cx);
        }
        cx.notify();
    }

    /// Start listening to the bus tape, once, the first time the messages page is opened.
    ///
    /// The log console is nudged from `boot.rs` because every window has one; this page is a
    /// bench almost no window opens, so it pays for its own subscription when it is first looked
    /// at. The loop is the console's: a nudge carries nothing, a burst coalesces into one redraw,
    /// and the timer keeps an entry emitted while drawing from redrawing its own frame forever.
    fn watch_tape(&mut self, cx: &mut Context<Self>) {
        if self.sink.messages.subscribed {
            return;
        }
        self.sink.messages.subscribed = true;
        let nudges = ubiq_proto::bus::tape().subscribe();
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            while nudges.recv_async().await.is_ok() {
                while nudges.try_recv().is_ok() {}
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(120))
                    .await;
            }
        })
        .detach();
    }

    /// Every conversation the window holds, across every open project, as `(id, name)`. The sink
    /// has no project of its own, so this is where its chat half gets something to draw.
    pub fn sink_conversations(&self) -> Vec<(AgentId, String)> {
        let mut all: Vec<(AgentId, String)> = self
            .projects
            .values()
            .flat_map(|open| open.conversations.values())
            .map(|conversation| {
                (
                    conversation.id,
                    conversation
                        .title
                        .clone()
                        .unwrap_or_else(|| conversation.harness.clone()),
                )
            })
            .collect();
        // A map has no order and the pill row must not shuffle between frames.
        all.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
        all
    }

    /// One of them, with the composer slot its surface types into. The slot comes from its own
    /// project's chat pool — the sink is one more surface borrowing a free chat slot, which is
    /// exactly what a chat tab does.
    pub fn sink_conversation(&self, agent: AgentId) -> Option<&Conversation> {
        self.projects
            .values()
            .find_map(|open| open.conversations.get(&agent))
    }

    /// The conversation the bench is reading: the one it was pointed at while that one still
    /// exists, and otherwise the first there is — so a window that starts an agent while the page
    /// is open fills in without a click, and one whose agent ended does not go blank.
    ///
    /// One answer, read by the page *and* by `agent_for_slot`, so the composer can never send to a
    /// conversation other than the one on screen.
    pub fn sink_agent(&self) -> Option<AgentId> {
        let held = |agent: &AgentId| {
            self.projects
                .values()
                .any(|open| open.conversations.contains_key(agent))
        };
        self.sink
            .messages
            .agent
            .filter(held)
            .or_else(|| self.sink_conversations().first().map(|(id, _)| *id))
    }

    /// Start or attach a conversation from the bench, through the same `+` menu the agents
    /// screen raises. What a pick does lands back here: the menu carries which surface asked.
    pub fn start_sink_chat(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        self.open_new_agent_menu(at, crate::state::NewAgentSurface::Sink, cx);
    }

    pub fn set_sink_conversation(&mut self, agent: AgentId, cx: &mut Context<Self>) {
        self.sink.messages.agent = Some(agent);
        cx.notify();
    }

    /// Which body the tape shows: the harness's original line, or the bus message.
    pub fn set_sink_message_original(&mut self, original: bool, cx: &mut Context<Self>) {
        self.sink.messages.original = original;
        cx.notify();
    }

    /// Read one entry in the viewer under the list. Picking the one already up puts it away, so
    /// the list can be read with nothing selected.
    pub fn select_sink_message(&mut self, seq: u64, cx: &mut Context<Self>) {
        self.sink.messages.selected = match self.sink.messages.selected {
            Some(open) if open == seq => None,
            _ => Some(seq),
        };
        cx.notify();
    }

    pub fn toggle_sink_message_follow(&mut self, cx: &mut Context<Self>) {
        self.sink.messages.follow = !self.sink.messages.follow;
        cx.notify();
    }

    /// Write the ring to a file, and keep the path where the toolbar can show it. The tape names
    /// the folder — `UBIQ_TAPE_DIR`, or the machine's temporary one — because a path is the
    /// coordinator's business and not the window's.
    pub fn dump_sink_messages(&mut self, cx: &mut Context<Self>) {
        self.sink.messages.dumped = match ubiq_proto::bus::tape().dump() {
            Ok(path) => {
                let path = path.display().to_string();
                tracing::info!(%path, "bus tape dumped");
                Some(path)
            }
            Err(error) => Some(format!("dump failed: {error}")),
        };
        cx.notify();
    }

    pub fn clear_sink_messages(&mut self, cx: &mut Context<Self>) {
        ubiq_proto::bus::tape().clear();
        self.sink.messages.selected = None;
        self.sink.messages.dumped = None;
        cx.notify();
    }

    /// Put one fixture into one of its viewer's layouts. A viewer with no preview keeps its source,
    /// which is [`SinkState::set_layout`]'s rule rather than this method's.
    pub fn set_sink_layout(
        &mut self,
        doc: &'static SinkDoc,
        layout: ViewLayout,
        cx: &mut Context<Self>,
    ) {
        self.sink.set_layout(doc, layout);
        cx.notify();
    }

    /// The buffer one fixture is edited in. The document is a constant and the buffer is what has
    /// been typed into it, which is what lets the preview follow the source half of a split.
    pub fn sink_buffer(&self, key: &str) -> Option<&Entity<EditorState>> {
        self.sink_buffers.get(key)
    }

    /// The A2UI page's payload buffer. An `Option` for the same reason [`Self::sink_buffer`] is:
    /// the page can be asked to draw in the frame before the window's buffers exist.
    pub fn a2ui_buffer_state(&self) -> Option<&Entity<EditorState>> {
        Some(&self.a2ui_buffer)
    }

    pub fn toggle_sink_facet(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(facet) = self.sink.facets.get_mut(index) {
            *facet = !*facet;
        }
        cx.notify();
    }

    pub fn set_sink_files_tree(&mut self, tree: bool, cx: &mut Context<Self>) {
        self.sink.files_tree = tree;
        cx.notify();
    }

    pub fn set_sink_choice(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.choice = index;
        cx.notify();
    }

    pub fn nudge_sink(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.nudge(delta);
        cx.notify();
    }

    /// What the style page's slider drags: the same level the stepper nudges, so the two controls
    /// and the meter between them are one value read three ways.
    pub fn set_sink_level(&mut self, level: f32, cx: &mut Context<Self>) {
        self.sink.level = level.round().clamp(0.0, 100.0) as u8;
        cx.notify();
    }

    /// Move the slider's thumb to whatever the level now is.
    ///
    /// The slider owns its own position — that is what adopting the library's state buys — so a
    /// nudge from the stepper beside it has to be pushed in, or the page would show one value in
    /// two places and disagree with itself.
    pub fn sync_sink_slider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let level = self.sink.level as f32;
        let slider = self.sink_slider.clone();
        slider.update(cx, |state, cx| state.set_value(level, window, cx));
    }

    pub fn toggle_sink_disclosure(&mut self, cx: &mut Context<Self>) {
        self.sink.disclosed = !self.sink.disclosed;
        cx.notify();
    }

    /// The style reference's demo menu. It closes on the pick like every other menu in the window,
    /// because that is the behaviour being demonstrated.
    pub fn pick_sink_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.picked = index;
        self.workbench.open_menu = None;
        cx.notify();
    }

    /// The A2UI page's example picker. Choosing one replaces the editor's text, and the surface is
    /// rebuilt from it — the payload is still the only place a drawn surface comes from.
    pub fn pick_a2ui_example(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(example) = crate::state::a2ui::EXAMPLES.get(index) else {
            return;
        };
        self.sink.a2ui.example = index;
        self.workbench.open_menu = None;
        self.a2ui_buffer
            .update(cx, |state, cx| state.set_value(example.source, window, cx));
        // `set_value` is not promised to emit a change, and `reload_a2ui` is idempotent, so the
        // page is rebuilt here rather than left to a subscription that may not fire.
        self.reload_a2ui(window, cx);
    }

    /// Rebuild the drawn surface from whatever the payload editor now holds.
    ///
    /// **A payload that does not parse leaves everything alone but the error.** A reader typing
    /// into the JSON breaks it on most keystrokes, and dropping the surface on each one would drop
    /// every field buffer with it — throwing away what they had typed into the drawing and taking
    /// the keyboard with it. So the banner appears above the last surface that parsed, and the
    /// surface is only replaced when there is one to replace it with.
    ///
    /// **A payload that does parse resets the data model**, because the payload carries one: a
    /// re-parse is the surface being created again, and keeping values at pointers the new payload
    /// may not have is worse than starting from what it says. The action log survives — it is a
    /// record of what was sent, not state of the surface.
    pub fn reload_a2ui(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let source = self.a2ui_buffer.read(cx).value().to_string();
        let parsed = match crate::state::a2ui::parse(&source) {
            Ok(parsed) => parsed,
            Err(reason) => {
                self.sink.a2ui.live.error = Some(reason);
                cx.notify();
                return;
            }
        };

        {
            let live = &mut self.sink.a2ui.live;
            live.error = None;
            live.tabs.clear();
            live.modal = None;
            live.invalid.clear();
            // Clearing the map is the whole teardown: a `Subscription` unsubscribes when it drops,
            // so a field from the payload being replaced cannot still be writing into the model.
            live.fields.clear();
            live.surface = Some(parsed.surface.clone());
            live.surface_id = parsed.surface_id.clone();
            live.send_data_model = parsed.send_data_model;
            live.model = parsed.model.clone();
        }

        // One buffer per text input, allocated here rather than while drawing: a render that
        // allocated an entity would allocate a fresh one every frame.
        for slot in crate::state::a2ui::live::input_slots(&parsed) {
            let input = cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).default_value(&slot.seed)
            });
            let path = slot.path.clone();
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &gpui_component::input::InputEvent, _window, cx| {
                    if matches!(event, gpui_component::input::InputEvent::Change) {
                        let typed = input.read(cx).value().to_string();
                        this.sink
                            .a2ui
                            .live
                            .set(&path, serde_json::Value::String(typed));
                        cx.notify();
                    }
                },
            );
            self.sink.a2ui.live.fields.insert(
                slot.key,
                crate::state::a2ui::live::Field::new(input, slot.path, subscription),
            );
        }

        cx.notify();
    }

    /// Write one value into the drawn surface's data model, at a pointer the renderer resolved.
    ///
    /// Everything a control does ends here: a tick, a choice, a nudge of a slider. It is the same
    /// funnel a keystroke goes through, and it is local — **nothing here reaches an agent.** The
    /// model crosses that boundary at exactly one moment, which is when an action fires.
    pub fn set_a2ui_value(
        &mut self,
        pointer: String,
        value: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        self.sink.a2ui.live.set(&pointer, value);
        cx.notify();
    }

    /// A button on a drawn surface was clicked: send what it says, or refuse to.
    ///
    /// A failing check disables the action rather than firing it, which is the catalog's own
    /// instruction — so a blocked click records which inputs refused it and produces no message.
    /// An action carrying a function call is **reported and not run**: `openUrl` opens nothing,
    /// because nothing drawn from an agent's payload may move this window or reach the network
    /// (`D115`).
    pub fn fire_a2ui_action(&mut self, key: String, cx: &mut Context<Self>) {
        use crate::state::a2ui::{Action, action, live, scoped};

        let Some(parsed) = self.sink.a2ui.live.parsed() else {
            return;
        };
        let (component_id, item, index) = live::Live::scope_of(&key);
        let scope = live::scope(item.as_ref(), index);

        let Some(component) = parsed.surface.get(&component_id) else {
            return;
        };
        let Some(what) = component.kind.button_action() else {
            return;
        };

        let failures = action::blocking_checks(&parsed);
        if !failures.is_empty() {
            let count = failures.len();
            self.sink.a2ui.live.invalid = failures.into_iter().collect();
            self.sink.a2ui.live.record(format!(
                "blocked: {count} check{} failed, nothing was sent",
                if count == 1 { "" } else { "s" }
            ));
            cx.notify();
            return;
        }
        self.sink.a2ui.live.invalid.clear();

        let entry = match what {
            Action::Event(event) => {
                let envelope = action::envelope(
                    &parsed.surface_id,
                    &component_id,
                    event,
                    &parsed.model,
                    scope,
                    &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    parsed.send_data_model,
                );
                serde_json::to_string_pretty(&envelope).unwrap_or_default()
            }
            // Reported, never performed. The one place that rule could be broken is the one place
            // it is kept.
            Action::Call(call) => format!(
                "local call: {}({}) — reported, not performed",
                call.call,
                call.args.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
            Action::Unknown(value) => format!(
                "action in a shape this build does not know: {}",
                serde_json::to_string(value).unwrap_or_default()
            ),
        };
        let _ = scoped(&component_id, scope);
        self.sink.a2ui.live.record(entry);
        cx.notify();
    }

    /// Empty the A2UI page's record of what it has sent.
    pub fn clear_a2ui_log(&mut self, cx: &mut Context<Self>) {
        self.sink.a2ui.live.log.clear();
        cx.notify();
    }

    /// Bring the data model or the action log forward under the payload.
    pub fn show_a2ui_pane(&mut self, pane: crate::state::sink::A2uiPane, cx: &mut Context<Self>) {
        self.sink.a2ui.pane = pane;
        cx.notify();
    }

    /// Bring one tab of a drawn `Tabs` forward. Navigation inside the preview, not a value: it
    /// writes nothing into the data model.
    pub fn select_a2ui_tab(&mut self, id: String, index: usize, cx: &mut Context<Self>) {
        self.sink.a2ui.live.tabs.insert(id, index);
        cx.notify();
    }

    /// Disclose a drawn `Modal`'s content, or fold the open one away. Only one is open at a time,
    /// the same rule the window's own overlays follow.
    pub fn toggle_a2ui_modal(&mut self, id: String, cx: &mut Context<Self>) {
        let live = &mut self.sink.a2ui.live;
        live.modal = if live.modal.as_deref() == Some(id.as_str()) {
            None
        } else {
            Some(id)
        };
        cx.notify();
    }

    pub fn open_sink_modal(&mut self, modal: SinkModal, cx: &mut Context<Self>) {
        // A modal takes the window's attention, so it closes whatever menu was down: two things
        // claiming an outside click is how a dismissal races itself.
        self.workbench.open_menu = None;
        self.sink.modal = Some(modal);
        cx.notify();
    }

    pub fn close_sink_modal(&mut self, cx: &mut Context<Self>) {
        self.sink.modal = None;
        cx.notify();
    }

    pub fn set_sink_settings_nav(&mut self, nav: SettingsNav, cx: &mut Context<Self>) {
        self.sink.settings.nav = nav;
        cx.notify();
    }

    pub fn set_sink_settings_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.theme = index;
        cx.notify();
    }

    pub fn toggle_sink_accent_follows(&mut self, cx: &mut Context<Self>) {
        self.sink.settings.accent_follows = !self.sink.settings.accent_follows;
        cx.notify();
    }

    pub fn set_sink_settings_density(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.density = index;
        cx.notify();
    }

    pub fn nudge_sink_font(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.settings.nudge_font(delta);
        cx.notify();
    }

    pub fn toggle_sink_reduce_motion(&mut self, cx: &mut Context<Self>) {
        self.sink.settings.reduce_motion = !self.sink.settings.reduce_motion;
        cx.notify();
    }

    pub fn set_sink_permission(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.permission = index;
        cx.notify();
    }

    pub fn nudge_sink_agents(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.settings.nudge_agents(delta);
        cx.notify();
    }

    pub fn nudge_sink_warn(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.settings.nudge_warn(delta);
        cx.notify();
    }

    pub fn toggle_sink_retry(&mut self, cx: &mut Context<Self>) {
        self.sink.settings.retry = !self.sink.settings.retry;
        cx.notify();
    }

    pub fn nudge_sink_idle(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.sink.settings.nudge_idle(delta);
        cx.notify();
    }

    pub fn toggle_sink_harness(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.toggle_harness(index);
        cx.notify();
    }

    pub fn toggle_sink_harness_open(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.toggle_open(index);
        cx.notify();
    }

    pub fn open_sink_settings_menu(&mut self, which: SettingsMenu, cx: &mut Context<Self>) {
        self.workbench.open_menu = Some(MenuId::SinkSettings);
        self.sink.settings.menu = Some(which);
        cx.notify();
    }

    pub fn pick_sink_settings_menu(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.sink.settings.menu {
            Some(SettingsMenu::Auth) => self.sink.settings.auth = index,
            Some(SettingsMenu::Model) => self.sink.settings.model = index,
            Some(SettingsMenu::Thinking) => self.sink.settings.thinking = index,
            Some(SettingsMenu::Mode) => self.sink.settings.mode = index,
            None => {}
        }
        self.workbench.open_menu = None;
        self.sink.settings.menu = None;
        cx.notify();
    }

    pub fn add_sink_env(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pair = self.sink_harness_env.read(cx).value().to_string();
        self.sink.settings.add_env(pair);
        let input = self.sink_harness_env.clone();
        input.update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    pub fn remove_sink_env(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.settings.remove_env(index);
        cx.notify();
    }

    pub fn set_sink_project_nav(&mut self, nav: ProjectNav, cx: &mut Context<Self>) {
        // The live dialog keeps its own nav on `ProjectSettings`: the sink page must not
        // reopen wherever the dialog was left. Only an existing project can carry tools, a drone
        // origin or a knowledge base, so those three are the navs a live dialog answers to besides
        // General — a folder not yet in the catalogue has no record to attach any of them to, and
        // stays there. The same three `ui::sink::project::nav` draws enabled.
        if let Some(settings) = self.workbench.project_settings.as_mut() {
            let editing = matches!(settings.mode, ProjectSettingsMode::Edit { .. });
            if nav == ProjectNav::General
                || (matches!(nav, ProjectNav::Tools | ProjectNav::Remote | ProjectNav::Kb)
                    && editing)
            {
                settings.nav = nav;
                cx.notify();
            }
            return;
        }
        self.sink.project.nav = nav;
        cx.notify();
    }

    /// Which colour the two project forms are editing: the live dialog's when it is up, the
    /// sink's fixture otherwise. One field, so the maths behind it is written once.
    pub(crate) fn colour_field(&mut self) -> &mut ColourField {
        match self.workbench.project_settings.as_mut() {
            Some(settings) => &mut settings.colour,
            None => &mut self.sink.project.colour,
        }
    }

    /// Which drone draft the two project forms are editing, on `colour_field`'s terms exactly.
    pub(crate) fn drone_field(&mut self) -> &mut DroneField {
        match self.workbench.project_settings.as_mut() {
            Some(settings) => &mut settings.drone,
            None => &mut self.sink.project.drone,
        }
    }

    /// Pin the form to a drone, or bring it home. Nothing is sent until Save: unlike the index
    /// pills, changing where a project lives is not something to act on mid-sentence.
    pub fn set_project_on_drone(&mut self, on_drone: bool, cx: &mut Context<Self>) {
        self.drone_field().on_drone = on_drone;
        cx.notify();
    }

    /// Which saved SSH profile the form's drone is reached through.
    pub fn set_project_drone_profile(&mut self, profile: SshProfileId, cx: &mut Context<Self>) {
        let field = self.drone_field();
        field.profile = Some(profile);
        field.on_drone = true;
        cx.notify();
    }

    /// The drone's lifetime, on the connect modal's own three presets.
    pub fn set_project_drone_preset(&mut self, preset: DronePreset, cx: &mut Context<Self>) {
        self.drone_field().preset = preset;
        cx.notify();
    }

    /// Send the Remote panel's draft, if it says anything the record does not.
    ///
    /// Nothing dials here. A drone is launched when the project is *opened* —
    /// `AppState::ensure_drone` — which is the moment there is something for one to serve; saving
    /// is only the record changing.
    pub fn save_project_drone(&mut self, project: ProjectId, cx: &mut Context<Self>) {
        let root = self
            .project_remote_root_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        let current = WindowRegistry::read(cx)
            .project(project)
            .and_then(|snapshot| snapshot.record.runs_on.clone());
        let Some(change) = self.drone_field().change(&root, current.as_ref()) else {
            return;
        };
        self.set_project_runs_on(project, change, cx);
    }

    /// Print that field's colour into whichever hex input belongs to it.
    fn sync_colour_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.workbench.project_settings.is_some() {
            self.sync_project_form_hex(window, cx);
        } else {
            self.sync_sink_project_hex(window, cx);
        }
    }

    pub fn set_sink_project_colour(
        &mut self,
        colour: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.colour_field().set_swatch(colour);
        self.sync_colour_hex(window, cx);
        cx.notify();
    }

    pub fn toggle_sink_colour_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.colour_field().toggle_picker();
        self.sync_colour_hex(window, cx);
        cx.notify();
    }

    pub fn set_sink_project_hsv(
        &mut self,
        hue: f32,
        sat: f32,
        val: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.colour_field().set_hsv(hue, sat, val);
        self.sync_colour_hex(window, cx);
        cx.notify();
    }

    pub(super) fn apply_sink_project_hex(&mut self, cx: &mut Context<Self>) {
        let text = self.sink_project_hex.read(cx).value();
        let Some(rgb) = crate::state::sink::parse_hex(text.as_ref()) else {
            return;
        };
        if self.sink.project.colour.apply_hex(rgb) {
            cx.notify();
        }
    }

    fn sync_sink_project_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = crate::state::sink::hex_string(self.sink.project.colour.rgb());
        let input = self.sink_project_hex.clone();
        input.update(cx, |input, cx| input.set_value(&hex, window, cx));
    }

    pub fn reset_sink_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sink.project.reset();
        let name = self.sink_project_name.clone();
        let about = self.sink_project_about.clone();
        name.update(cx, |input, cx| {
            input.set_value(crate::state::sink::PROJECT_NAME, window, cx)
        });
        about.update(cx, |input, cx| {
            input.set_value(crate::state::sink::PROJECT_ABOUT, window, cx)
        });
        self.sync_sink_project_hex(window, cx);
        cx.notify();
    }

    /// Choose which dialect the script page runs: JavaScript as written, or TypeScript compiled
    /// to JavaScript first. A run and a validate both read this.
    ///
    /// The previous run and report are from the other dialect, so they are cleared with the
    /// panel: a page showing a JavaScript outcome beside a TypeScript toggle would be showing the
    /// mode it was not in.
    pub fn set_sink_script_dialect(
        &mut self,
        dialect: crate::state::script::Dialect,
        cx: &mut Context<Self>,
    ) {
        self.sink.script.dialect = dialect;
        self.forget_sink_script_answer();
        cx.notify();
    }

    /// Disclose the oxc settings panel, or fold it away.
    pub fn toggle_sink_script_settings(&mut self, cx: &mut Context<Self>) {
        self.sink.script.settings_open = !self.sink.script.settings_open;
        cx.notify();
    }

    /// Which of the two things the right-hand panel shows: the reference, or the script's panel.
    pub fn set_sink_script_pane(
        &mut self,
        pane: crate::state::sink::ScriptPane,
        cx: &mut Context<Self>,
    ) {
        self.sink.script.pane = pane;
        cx.notify();
    }

    /// Which ECMAScript level the transformer lowers to.
    pub fn set_sink_script_target(
        &mut self,
        target: crate::state::script::EsTarget,
        cx: &mut Context<Self>,
    ) {
        self.sink.script.options.target = target;
        self.forget_sink_script_answer();
        cx.notify();
    }

    /// One of the settings panel's four switches. A closure rather than four near-identical
    /// methods: the panel names the field, and every one of them invalidates the same answer.
    pub fn toggle_sink_script_option(
        &mut self,
        pick: fn(&mut crate::state::script::OxcOptions) -> &mut bool,
        cx: &mut Context<Self>,
    ) {
        let flag = pick(&mut self.sink.script.options);
        *flag = !*flag;
        self.forget_sink_script_answer();
        cx.notify();
    }

    /// How many problems one report names. Stepped rather than typed, because the only values
    /// worth having are "a handful" and "all of them".
    pub fn step_sink_script_max_errors(&mut self, by: i32, cx: &mut Context<Self>) {
        let current = self.sink.script.options.max_errors as i32;
        self.sink.script.options.max_errors = (current + by).clamp(1, 200) as usize;
        cx.notify();
    }

    /// Put the settings back where they started.
    pub fn reset_sink_script_options(&mut self, cx: &mut Context<Self>) {
        self.sink.script.options = crate::state::script::OxcOptions::default();
        self.forget_sink_script_answer();
        cx.notify();
    }

    /// The script page's example picker. Choosing one seeds both buffers, so the example is the
    /// program — a picker that only moved a selector would have nothing to run.
    pub fn pick_sink_script_example(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(example) = crate::state::sink::SCRIPT_EXAMPLES.get(index) else {
            return;
        };
        self.sink.script.example = index;
        self.sink.script.dialect = example.dialect;
        self.forget_sink_script_answer();
        self.workbench.open_menu = None;
        self.script_prelude
            .update(cx, |state, cx| state.set_value(example.prelude, window, cx));
        self.script_buffer
            .update(cx, |state, cx| state.set_value(example.source, window, cx));
        cx.notify();
    }

    /// Drop everything the last run and the last validation left behind.
    ///
    /// One place rather than five, because they always go together: an outcome, a report and a
    /// drawn panel all describe the same program in the same mode, and keeping any one of them
    /// past a change to that program or that mode is how a page starts lying about what it ran.
    fn forget_sink_script_answer(&mut self) {
        self.sink.script.report = None;
        self.sink.script.outcome = None;
        self.sink.script.ui = None;
        self.sink.script.a2ui = crate::state::a2ui::live::Live::default();
    }

    /// Everything the interface knows that a script may read, serialised here, on this thread,
    /// before the interpreter exists.
    ///
    /// A snapshot rather than a handle is the whole of the capability model: the interpreter runs
    /// on a thread of its own and cannot be given anything that would let it re-enter the
    /// interface, so what it reads is JSON that stopped changing the moment Run was pressed.
    /// Nothing here is a projection the interface does not already draw somewhere.
    fn script_facts(&self, cx: &Context<Self>) -> crate::state::script::HostFacts {
        use serde_json::json;

        let project = self
            .project_snapshot(cx)
            .map(|snapshot| {
                json!({
                    "id": snapshot.record.id.to_string(),
                    "name": snapshot.record.name,
                    "root": snapshot.record.path,
                    "open": true,
                    "panes": snapshot.open_panes,
                })
            })
            .unwrap_or(serde_json::Value::Null);

        let projects = serde_json::Value::from_iter(
            crate::state::WindowRegistry::read(cx)
                .all()
                .map(|snapshot| {
                    json!({
                        "id": snapshot.record.id.to_string(),
                        "name": snapshot.record.name,
                        "root": snapshot.record.path,
                    })
                })
                .collect::<Vec<_>>(),
        );

        let tasks = serde_json::Value::from_iter(
            self.work(cx)
                .into_iter()
                .flat_map(|work| work.tasks.iter())
                .map(|task| {
                    json!({
                        "id": task.id.to_string(),
                        "title": task.title,
                        "status": format!("{:?}", task.status).to_lowercase(),
                        "priority": format!("{:?}", task.priority).to_lowercase(),
                        "labels": task.labels.iter().map(|label| label.name.clone())
                            .collect::<Vec<_>>(),
                        "steps": task.steps.len(),
                        "updatedAt": task.updated_at.to_rfc3339(),
                    })
                })
                .collect::<Vec<_>>(),
        );

        // Every model of every configured provider, flattened: a script asking what it can reach
        // wants the list, not the provider tree the settings screen draws.
        let settings = &self.workbench.settings;
        let models = serde_json::Value::from_iter(
            settings
                .ai_providers
                .iter()
                .flat_map(|info| {
                    let list = settings.ai_models.get(&info.provider.id);
                    list.into_iter()
                        .flat_map(|list| list.models.iter())
                        .map(move |model| {
                            json!({
                                "id": model.id,
                                "name": model.label,
                                "provider": info.provider.name,
                                "contextTokens": model.context_tokens,
                                // A local endpoint is an OpenAI-compatible one pointed at this
                                // machine; there is no other way to tell, and the distinction is
                                // what a script asking about local inference means.
                                "local": info.provider.base_url.as_deref().is_some_and(|url| {
                                    url.contains("localhost") || url.contains("127.0.0.1")
                                }),
                            })
                        })
                })
                .collect::<Vec<_>>(),
        );

        let search = json!({
            "query": self.search.query.read(cx).value().to_string(),
            "kind": if self.search.regex { "regex" } else { "literal" },
            "caseSensitive": self.search.case_sensitive,
            "finished": self.search.finished,
            "truncated": self.search.truncated,
            "totalHits": self.search.total_hits,
            "results": self.search.results.iter().map(|file| json!({
                "path": file.rel_path,
                "hits": file.hits.iter().map(|hit| json!({
                    "line": hit.line,
                    "text": hit.text.trim(),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });

        crate::state::script::HostFacts {
            project,
            projects,
            tasks,
            models,
            search,
        }
    }

    /// Evaluate the script page's two buffers and keep what the interpreter said.
    ///
    /// The prelude goes first and the program second, in one context, which is the whole of what
    /// the page claims: whatever the prelude defines is what the program may call. Both are read
    /// here rather than mirrored into state, because the buffer is the program — a copy kept
    /// beside it could only ever be a stale one.
    ///
    /// **The UI thread never waits on the interpreter.** The evaluation runs on a thread of its
    /// own, bounded by the total budget in the module, and the answer lands through a spawned
    /// task. A run that declared a panel has it drawn; an outcome that lands after a newer Run is
    /// discarded — that is what [`crate::state::ScriptDemo::seq`] is for.
    pub fn run_sink_script(&mut self, cx: &mut Context<Self>) {
        self.start_sink_script(None, cx);
    }

    /// A click on the declared panel, as a fresh run.
    ///
    /// Nothing survives a run, so there is no closure left to call: the script is evaluated again
    /// from the top with the event in hand, and the handler its second declaration registers is
    /// called then. The console is not cleared for a replay — the point of the panel is to watch
    /// the lines accumulate press by press, which is what makes the replay visible at all.
    pub fn replay_sink_script(
        &mut self,
        event: crate::state::script::ScriptEvent,
        cx: &mut Context<Self>,
    ) {
        self.start_sink_script(Some(event), cx);
    }

    /// The run machinery both buttons and every panel click end in.
    fn start_sink_script(
        &mut self,
        event: Option<crate::state::script::ScriptEvent>,
        cx: &mut Context<Self>,
    ) {
        if self.sink.script.running {
            return;
        }
        let run = crate::state::script::Run {
            prelude: self.script_prelude.read(cx).value().to_string(),
            source: self.script_buffer.read(cx).value().to_string(),
            dialect: self.sink.script.dialect,
            options: self.sink.script.options,
            facts: self.script_facts(cx),
            event,
        };

        self.sink.script.report = None;
        self.sink.script.seq += 1;
        let seq = self.sink.script.seq;
        self.sink.script.running = true;
        cx.notify();

        let task = cx.background_spawn(async move { crate::state::script::eval(run) });
        cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
            let outcome = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.sink.script.seq != seq {
                    return;
                }
                this.sink.script.running = false;
                match outcome.ui.clone() {
                    Some(ui) => {
                        let payload = ui.payload.clone();
                        this.sink.script.ui = Some(ui);
                        this.land_sink_script_panel(payload, window, cx);
                    }
                    // A run that withdrew its panel — or never declared one — leaves nothing to
                    // draw, and the pane says so rather than showing the last run's surface.
                    None => {
                        this.sink.script.ui = None;
                        this.sink.script.a2ui = crate::state::a2ui::live::Live::default();
                    }
                }
                this.sink.script.outcome = Some(outcome);
                cx.notify();
            });
        })
        .detach();
    }

    /// Rebuild the script page's panel from the payload the last run declared.
    ///
    /// The mirror of [`Self::reload_a2ui`], written against `sink.script.a2ui` rather than the
    /// A2UI page's `Live` so the two stay independent: the script page's surface comes from
    /// `ubiq.sink.ui`, and nothing the A2UI page rebuilt from its own payload editor may touch it.
    /// A payload that does not parse leaves the previous surface alone and sets the error.
    pub fn land_sink_script_panel(
        &mut self,
        payload: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parsed = match crate::state::a2ui::parse(&payload) {
            Ok(parsed) => parsed,
            Err(reason) => {
                self.sink.script.a2ui.error = Some(reason);
                cx.notify();
                return;
            }
        };

        {
            let live = &mut self.sink.script.a2ui;
            live.error = None;
            live.tabs.clear();
            live.modal = None;
            live.invalid.clear();
            // Clearing the map is the whole teardown: a `Subscription` unsubscribes when it drops,
            // so a field from the payload being replaced cannot still be writing into the model.
            live.fields.clear();
            live.surface = Some(parsed.surface.clone());
            live.surface_id = parsed.surface_id.clone();
            live.send_data_model = parsed.send_data_model;
            live.model = parsed.model.clone();
        }

        // One buffer per text input, allocated here rather than while drawing: a render that
        // allocated an entity would allocate a fresh one every frame.
        for slot in crate::state::a2ui::live::input_slots(&parsed) {
            let input = cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).default_value(&slot.seed)
            });
            let path = slot.path.clone();
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &gpui_component::input::InputEvent, _window, cx| {
                    if matches!(event, gpui_component::input::InputEvent::Change) {
                        let typed = input.read(cx).value().to_string();
                        this.sink
                            .script
                            .a2ui
                            .set(&path, serde_json::Value::String(typed));
                        cx.notify();
                    }
                },
            );
            self.sink.script.a2ui.fields.insert(
                slot.key,
                crate::state::a2ui::live::Field::new(input, slot.path, subscription),
            );
        }

        cx.notify();
    }

    /// Forget the last run. The buffers are untouched — this clears the console, not the program.
    pub fn clear_sink_script(&mut self, cx: &mut Context<Self>) {
        self.forget_sink_script_answer();
        cx.notify();
    }

    /// Syntax-check both buffers without running either, and say so in the console.
    ///
    /// **The console is cleared first.** A report drawn under the previous run's output would read
    /// as part of it, and the verdict this button exists to give — *nothing is wrong* — is a line
    /// nobody would find at the bottom of a log. So the console holds exactly one thing after
    /// Validate: what oxc said about the program as it stands, whether that is a list of problems
    /// or the one line saying there are none.
    ///
    /// Synchronous on purpose: oxc is a parse and a transform, fast enough that a button click can
    /// wait for it, and a report that landed asynchronously could land against an edited buffer.
    pub fn validate_sink_script(&mut self, cx: &mut Context<Self>) {
        let prelude = self.script_prelude.read(cx).value().to_string();
        let source = self.script_buffer.read(cx).value().to_string();
        self.sink.script.outcome = None;
        self.sink.script.report = Some(crate::state::script::validate(
            &prelude,
            &source,
            self.sink.script.dialect,
            &self.sink.script.options,
        ));
        cx.notify();
    }

    /// Write one value into the preview surface's data model, at a pointer the renderer resolved.
    /// Everything a control in the preview does ends here, exactly as [`Self::set_a2ui_value`]
    /// ends every control on the A2UI page.
    pub fn set_script_a2ui_value(
        &mut self,
        pointer: String,
        value: serde_json::Value,
        cx: &mut Context<Self>,
    ) {
        self.sink.script.a2ui.set(&pointer, value);
        cx.notify();
    }

    /// Bring one tab of a drawn `Tabs` forward in the script preview. Navigation inside the
    /// surface, not a value: it writes nothing into the data model.
    pub fn select_script_a2ui_tab(&mut self, id: String, index: usize, cx: &mut Context<Self>) {
        self.sink.script.a2ui.tabs.insert(id, index);
        cx.notify();
    }

    /// Disclose a drawn `Modal`'s content in the script preview, or fold the open one away.
    pub fn toggle_script_a2ui_modal(&mut self, id: String, cx: &mut Context<Self>) {
        let live = &mut self.sink.script.a2ui;
        live.modal = if live.modal.as_deref() == Some(id.as_str()) {
            None
        } else {
            Some(id)
        };
        cx.notify();
    }

    /// A button on the script's panel was clicked: send what it says, or refuse to — and then
    /// hand the event back to the script that declared the panel.
    ///
    /// The mirror of [`Self::fire_a2ui_action`], with the same rules — a failing check blocks, and
    /// a function call is reported and never performed (`D115`) — plus the one thing this panel
    /// has that the A2UI page's has not: an author. A declaration made with a handler re-runs the
    /// script with the event in hand, because nothing survives a run and there is no closure left
    /// to call. A declaration made without one does not, and the log says why.
    pub fn fire_script_a2ui_action(&mut self, key: String, cx: &mut Context<Self>) {
        use crate::state::a2ui::{Action, action, live, scoped};

        let Some(parsed) = self.sink.script.a2ui.parsed() else {
            return;
        };
        let (component_id, item, index) = live::Live::scope_of(&key);
        let scope = live::scope(item.as_ref(), index);

        let Some(component) = parsed.surface.get(&component_id) else {
            return;
        };
        let Some(what) = component.kind.button_action() else {
            return;
        };

        let failures = action::blocking_checks(&parsed);
        if !failures.is_empty() {
            let count = failures.len();
            self.sink.script.a2ui.invalid = failures.into_iter().collect();
            self.sink.script.a2ui.record(format!(
                "blocked: {count} check{} failed, nothing was sent",
                if count == 1 { "" } else { "s" }
            ));
            cx.notify();
            return;
        }
        self.sink.script.a2ui.invalid.clear();

        let mut replay = None;
        let entry = match what {
            Action::Event(event) => {
                let envelope = action::envelope(
                    &parsed.surface_id,
                    &component_id,
                    event,
                    &parsed.model,
                    scope,
                    &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                    parsed.send_data_model,
                );
                replay = Some(crate::state::script::ScriptEvent {
                    name: event.name.clone(),
                    surface_id: parsed.surface_id.clone(),
                    component_id: component_id.clone(),
                });
                serde_json::to_string_pretty(&envelope).unwrap_or_default()
            }
            // Reported, never performed. The one place that rule could be broken is the one place
            // it is kept.
            Action::Call(call) => format!(
                "local call: {}({}) — reported, not performed",
                call.call,
                call.args.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
            Action::Unknown(value) => format!(
                "action in a shape this build does not know: {}",
                serde_json::to_string(value).unwrap_or_default()
            ),
        };
        let _ = scoped(&component_id, scope);
        self.sink.script.a2ui.record(entry);

        let handled = self
            .sink
            .script
            .ui
            .as_ref()
            .map(|ui| ui.handler)
            .unwrap_or(false);
        match (replay, handled) {
            (Some(event), true) => self.replay_sink_script(event, cx),
            (Some(event), false) => {
                self.sink.script.a2ui.record(format!(
                    "{}: the declaration carried no handler, so nothing was replayed",
                    event.name
                ));
                cx.notify();
            }
            _ => cx.notify(),
        }
    }

    pub fn set_sink_pick_kind(&mut self, kind: PickKind, cx: &mut Context<Self>) {
        self.sink.picker.kind = kind;
        cx.notify();
    }

    pub fn set_sink_pick_count(&mut self, count: PickerCount, cx: &mut Context<Self>) {
        self.sink.picker.count = count;
        cx.notify();
    }

    pub fn set_sink_pick_commit(&mut self, commit: Commit, cx: &mut Context<Self>) {
        self.sink.picker.commit = commit;
        cx.notify();
    }

    pub fn set_sink_pick_modal(&mut self, modal: bool, cx: &mut Context<Self>) {
        self.sink.picker.modal = modal;
        cx.notify();
    }

    pub fn set_sink_pick_view(&mut self, view: PickerView, cx: &mut Context<Self>) {
        self.sink.picker.view = view;
        cx.notify();
    }

    pub fn set_sink_pick_root(&mut self, root: usize, cx: &mut Context<Self>) {
        self.sink.picker.root = root;
        cx.notify();
    }

    pub fn set_sink_pick_pattern(&mut self, pattern: usize, cx: &mut Context<Self>) {
        self.sink.picker.pattern = pattern;
        cx.notify();
    }

    // ---- teamsim: the teams graph's arrangements, against a scenario that can be edited -----
    //
    // Every one of these is a call into `state::teamsim`, because the page draws and mutates
    // nothing itself. The scenario is the source of truth and the layout follows it; only `Tidy`
    // throws hand positions away.

    pub fn load_teamsim_preset(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.teamsim.load(index);
        self.workbench.open_menu = None;
        cx.notify();
    }

    /// Choose the arrangement. It tidies, because that is what choosing one means.
    pub fn set_teamsim_algo(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(algo) = crate::state::layout::Algo::ALL.get(index).copied() {
            self.sink.teamsim.sim.set_algo(algo);
        }
        self.workbench.open_menu = None;
        cx.notify();
    }

    /// Throw every hand position away and arrange the whole graph again — the one thing that does.
    pub fn tidy_teamsim(&mut self, cx: &mut Context<Self>) {
        self.sink.teamsim.sim.tidy();
        cx.notify();
    }

    pub fn zoom_teamsim(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.sink.teamsim.zoom_by(delta);
        cx.notify();
    }

    pub fn reset_teamsim_zoom(&mut self, cx: &mut Context<Self>) {
        self.sink.teamsim.zoom = 1.0;
        cx.notify();
    }

    /// Add a container to the first session, with one card in it — an arrival, so nothing that is
    /// already placed moves.
    pub fn add_teamsim_task(&mut self, cx: &mut Context<Self>) {
        let session = self
            .sink
            .teamsim
            .sim
            .scenario
            .sessions
            .first()
            .map(|session| session.id.clone());
        if let Some(session) = session {
            self.sink.teamsim.sim.add_task(&session);
        }
        cx.notify();
    }

    /// Add a card to the first container, for the same reason.
    pub fn add_teamsim_agent(&mut self, cx: &mut Context<Self>) {
        let task = self
            .sink
            .teamsim
            .sim
            .scenario
            .tasks
            .first()
            .map(|task| task.id.clone());
        if let Some(task) = task {
            self.sink.teamsim.sim.add_agent(&task);
        }
        cx.notify();
    }

    pub fn add_teamsim_subagent(&mut self, agent: String, cx: &mut Context<Self>) {
        self.sink.teamsim.sim.add_subagent(&agent);
        cx.notify();
    }

    pub fn remove_teamsim_subagent(&mut self, agent: String, sub: String, cx: &mut Context<Self>) {
        self.sink.teamsim.sim.remove_subagent(&agent, &sub);
        cx.notify();
    }

    pub fn remove_teamsim_agent(&mut self, agent: String, cx: &mut Context<Self>) {
        if self.sink.teamsim.arming.as_deref() == Some(agent.as_str()) {
            self.sink.teamsim.arming = None;
        }
        self.sink.teamsim.sim.remove_agent(&agent);
        cx.notify();
    }

    /// Arm a link from one card, or — with one already armed — finish it against the card clicked.
    /// Clicking the armed card again disarms, because a link to itself is the one thing the gesture
    /// cannot mean.
    pub fn arm_teamsim_link(&mut self, agent: String, cx: &mut Context<Self>) {
        match self.sink.teamsim.arming.take() {
            Some(from) if from == agent => {}
            Some(from) => {
                self.sink
                    .teamsim
                    .sim
                    .link(&from, &agent, crate::state::teamsim::LinkKind::Handoff);
            }
            None => self.sink.teamsim.arming = Some(agent),
        }
        cx.notify();
    }

    /// The card clicked: what it is, and — when a link is armed — the target of that link instead.
    pub fn select_teamsim_card(&mut self, agent: String, cx: &mut Context<Self>) {
        if self.sink.teamsim.arming.is_some() {
            self.arm_teamsim_link(agent, cx);
            return;
        }
        self.sink.teamsim.selected = Some(agent);
        cx.notify();
    }

    pub fn unlink_teamsim(&mut self, index: usize, cx: &mut Context<Self>) {
        self.sink.teamsim.sim.unlink(index);
        cx.notify();
    }

    /// The scenario, positions and all, onto the clipboard. The clipboard is this page's save file:
    /// a bench with a document store behind it would be a screen, and the JSON is one paste away
    /// from the Python tool beside it.
    pub fn copy_teamsim_json(&mut self, cx: &mut Context<Self>) {
        let json = self.sink.teamsim.sim.to_json();
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(json));
        cx.notify();
    }

    /// And back off it. A paste that does not parse leaves what is on screen alone and says why.
    pub fn paste_teamsim_json(&mut self, cx: &mut Context<Self>) {
        let text = cx
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default();
        if text.trim().is_empty() {
            self.sink.teamsim.error = Some("nothing on the clipboard to read".to_string());
        } else {
            self.sink.teamsim.paste(&text);
        }
        cx.notify();
    }

    /// Pick a block up. The grab point is where inside it the pointer went down, which is what
    /// stops it jumping under the cursor on the first move.
    pub fn start_teamsim_carry(
        &mut self,
        held: crate::state::TeamsimHeld,
        grab: (f32, f32),
        cx: &mut Context<Self>,
    ) {
        self.sink.teamsim.carry = Some(crate::state::TeamsimCarry { held, grab });
        cx.notify();
    }

    /// Put whatever is held at `at`, in canvas coordinates at 100% zoom. The drop writes back
    /// through the same `Layout` calls the Teams screen makes, so a hand position here is a hand
    /// position there.
    pub fn move_teamsim_carry(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        let Some(carry) = self.sink.teamsim.carry.clone() else {
            return;
        };
        match &carry.held {
            crate::state::TeamsimHeld::Task(task) => self.sink.teamsim.sim.move_task(task, at),
            crate::state::TeamsimHeld::Agent(agent) => self.sink.teamsim.sim.move_agent(agent, at),
            crate::state::TeamsimHeld::Sub { agent, sub } => {
                self.sink.teamsim.sim.move_sub(agent, sub, at)
            }
        }
        cx.notify();
    }

    pub fn end_teamsim_carry(&mut self, cx: &mut Context<Self>) {
        self.sink.teamsim.carry = None;
        cx.notify();
    }
}
