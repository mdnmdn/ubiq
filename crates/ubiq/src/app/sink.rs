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

    /// The A2UI page's example picker. Choosing one replaces the editor's text, which is the only
    /// place the drawn surface comes from — so the preview follows without anything caching it.
    ///
    /// The navigation inside the old surface goes with it: a tab index and a disclosed modal are
    /// both keyed by a component id, and an id from the payload being replaced means nothing in
    /// the one arriving.
    pub fn pick_a2ui_example(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(example) = crate::state::a2ui::EXAMPLES.get(index) else {
            return;
        };
        self.sink.a2ui.example = index;
        self.sink.a2ui.tabs.clear();
        self.sink.a2ui.modal = None;
        self.workbench.open_menu = None;
        self.a2ui_buffer
            .update(cx, |state, cx| state.set_value(example.source, window, cx));
        cx.notify();
    }

    /// Bring one tab of a drawn `Tabs` forward. Navigation inside the preview, not a value: it
    /// writes nothing back into the payload.
    pub fn select_a2ui_tab(&mut self, id: String, index: usize, cx: &mut Context<Self>) {
        self.sink.a2ui.tabs.insert(id, index);
        cx.notify();
    }

    /// Disclose a drawn `Modal`'s content, or fold the open one away. Only one is open at a time,
    /// the same rule the window's own overlays follow.
    pub fn toggle_a2ui_modal(&mut self, id: String, cx: &mut Context<Self>) {
        self.sink.a2ui.modal = if self.sink.a2ui.modal.as_deref() == Some(id.as_str()) {
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
        // reopen wherever the dialog was left. Only an existing project can carry tools, so
        // Tools is the one other nav a live dialog answers to — a folder not yet in the
        // catalogue stays on General.
        if let Some(settings) = self.workbench.project_settings.as_mut() {
            let editing = matches!(settings.mode, ProjectSettingsMode::Edit { .. });
            if nav == ProjectNav::General || (nav == ProjectNav::Tools && editing) {
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
}
