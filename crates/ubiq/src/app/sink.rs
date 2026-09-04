use super::*;

impl AppState {
    pub fn set_sink_section(&mut self, section: SinkSection, cx: &mut Context<Self>) {
        self.sink.section = section;
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
        if self.workbench.project_settings.is_some() {
            return;
        }
        self.sink.project.nav = nav;
        cx.notify();
    }

    pub fn set_sink_project_colour(
        &mut self,
        colour: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(settings) = self.workbench.project_settings.as_mut() {
            settings.colour = colour;
            settings.custom = None;
            settings.picker_open = false;
            self.sync_project_form_hex(window, cx);
        } else {
            self.sink.project.set_swatch(colour);
            self.sync_sink_project_hex(window, cx);
        }
        cx.notify();
    }

    pub fn toggle_sink_colour_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(settings) = self.workbench.project_settings.as_mut() {
            let open = !settings.picker_open;
            if open {
                let rgb = settings
                    .custom
                    .unwrap_or_else(|| project_swatch_rgb(settings.colour));
                let (hue, sat, val) = crate::state::sink::rgb_to_hsv(rgb);
                settings.hue = hue;
                settings.sat = sat;
                settings.val = val;
            }
            settings.picker_open = open;
            self.sync_project_form_hex(window, cx);
        } else {
            let open = !self.sink.project.picker_open;
            if open {
                let rgb = self.sink_project_rgb();
                let (hue, sat, val) = crate::state::sink::rgb_to_hsv(rgb);
                self.sink.project.hue = hue;
                self.sink.project.sat = sat;
                self.sink.project.val = val;
            }
            self.sink.project.picker_open = open;
            self.sync_sink_project_hex(window, cx);
        }
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
        if let Some(settings) = self.workbench.project_settings.as_mut() {
            settings.hue = hue.clamp(0.0, 1.0);
            settings.sat = sat.clamp(0.0, 1.0);
            settings.val = val.clamp(0.0, 1.0);
            settings.custom = Some(crate::state::sink::hsv_to_rgb(
                settings.hue,
                settings.sat,
                settings.val,
            ));
            self.sync_project_form_hex(window, cx);
        } else {
            self.sink.project.set_hsv(hue, sat, val);
            self.sync_sink_project_hex(window, cx);
        }
        cx.notify();
    }

    pub(super) fn apply_sink_project_hex(&mut self, cx: &mut Context<Self>) {
        let text = self.sink_project_hex.read(cx).value();
        let Some(rgb) = crate::state::sink::parse_hex(text.as_ref()) else {
            return;
        };
        if self.sink.project.custom == Some(rgb) {
            return;
        }
        if self.sink.project.custom.is_none() && rgb == project_swatch_rgb(self.sink.project.colour)
        {
            return;
        }
        self.sink.project.set_rgb(rgb);
        cx.notify();
    }

    fn sync_sink_project_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let hex = crate::state::sink::hex_string(self.sink_project_rgb());
        let input = self.sink_project_hex.clone();
        input.update(cx, |input, cx| input.set_value(&hex, window, cx));
    }

    fn sink_project_rgb(&self) -> u32 {
        self.sink
            .project
            .custom
            .unwrap_or_else(|| project_swatch_rgb(self.sink.project.colour))
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
