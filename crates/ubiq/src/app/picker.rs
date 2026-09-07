use super::*;

impl AppState {
    /// Raise a picker over the sink's fixture tree, in the shape the page's controls describe.
    ///
    /// The previous answer goes with it: a readout left standing over a dialog that is being asked
    /// again reads as this dialog's answer, which it is not.
    pub fn raise_sink_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sink.picker.result = None;
        self.sink.picker.dismissed = false;
        let request = self.sink.picker.request();
        let view = self.sink.picker.view;
        self.open_file_picker(request, crate::state::sink::picker_tree(), view, window, cx);
    }

    // ── The file picker ─────────────────────────────────────────
    //
    // One dialog, raised by whichever screen needs a path and answered back to whoever asked. The
    // window holds it because exactly one may be up, and because the field above its rows is one
    // of the window's fields like every other.

    /// Raise a picker over `forest`, in the arrangement it should open in.
    ///
    /// The field is emptied first: a filter left over from the last dialog would hide rows the new
    /// one was raised to show.
    pub fn open_file_picker(
        &mut self,
        request: crate::state::file_picker::PickerRequest,
        forest: Vec<crate::state::file_picker::PickerNode>,
        view: PickerView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A dialog takes the window's attention, so it closes whatever menu was down: two things
        // claiming an outside click is how a dismissal races itself.
        self.workbench.open_menu = None;
        self.picker_filter
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.file_picker = Some(FilePickerState::open(request, forest, view));
        // The field takes the keyboard, because the first thing a picker is for is typing a name
        // into it. Every other key the dialog answers to is bound against the field as well as
        // against the dialog, so the arrows still drive the rows — see `ui::file_picker`.
        let field = self.picker_filter.read(cx).focus_handle(cx);
        window.focus(&field, cx);
        cx.notify();
    }

    /// A key the dialog answered, or did not.
    ///
    /// Answering `false` is what hands the key back: `left` and `right` mean nothing in the flat
    /// list, and the caller propagates so the filter field gets its caret keys back.
    pub fn press_picker_key(
        &mut self,
        key: PickerKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(picker) = self.file_picker.as_mut() else {
            return false;
        };
        let pressed = picker.press(key);
        let at = picker.cursor_index();

        match pressed {
            Pressed::Ignored => false,
            Pressed::Moved => {
                // Follow the cursor: an arrow past the last drawn row has to bring it into view.
                if let Some(at) = at {
                    self.picker_scroll.scroll_to_item(at);
                }
                // A right-arrow or `enter` on a shut folder may just have expanded one a host
                // owns — the same folder a twisty click would have asked about.
                self.sync_host_browse(cx);
                cx.notify();
                true
            }
            Pressed::Commit => {
                self.commit_file_picker(window, cx);
                true
            }
            Pressed::Dismiss => {
                self.cancel_file_picker(window, cx);
                true
            }
        }
    }

    pub fn set_picker_view(&mut self, view: PickerView, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.set_view(view);
        }
        cx.notify();
    }

    /// Show or hide dotfiles. A toggle rather than a one-way filter — see
    /// `state::file_picker::FilePickerState::show_hidden`'s own doc on why one has to exist at
    /// all: a folder a user needs, like `.config`, must stay reachable.
    pub fn toggle_picker_hidden(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            let show = !picker.show_hidden();
            picker.set_show_hidden(show);
        }
        cx.notify();
    }

    pub fn toggle_picker_folder(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.toggle_folder(&path);
        }
        // The twisty may just have opened a folder a host still owes a listing to.
        self.sync_host_browse(cx);
        cx.notify();
    }

    /// What a click on a row does, which the picker itself decides: a folder that cannot be picked
    /// opens, and a pick that was asked to be final closes the dialog on the spot.
    pub fn click_picker_row(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.as_mut() else {
            return;
        };
        if picker.click(&path) {
            self.commit_file_picker(window, cx);
            return;
        }
        // An unpickable click on a folder in tree view opens it — the same event a twisty click
        // asks the host about.
        self.sync_host_browse(cx);
        cx.notify();
    }

    /// Hand what was chosen to whoever asked for it, and take the dialog down.
    pub fn commit_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.take() else {
            return;
        };
        let picked = picker.picked().to_vec();
        match picker.request.owner {
            PickerOwner::Sink => {
                self.sink.picker.result = Some(picked);
                self.sink.picker.dismissed = false;
            }
            // What a composer asked for arrives as tags under the token row, not as text in the
            // field: the prompt is still being written, and a path spelled into it cannot be
            // taken back out by clicking it. The sizes come off the picker's own nodes — the
            // interface reads no disk — which is why they are taken before it is dropped.
            PickerOwner::Composer { agent, slot } => {
                let sized = picker.picked_with_sizes();
                self.attach_files(agent, slot, &sized, window, cx);
            }
            // The host this folder came from lives in `host_browse`, not in the owner itself — see
            // `state::file_picker::PickerOwner::HostProject`'s own doc.
            PickerOwner::HostProject => {
                if let (Some(path), Some(browse)) =
                    (picked.into_iter().next(), self.host_browse.take())
                {
                    self.adding = true;
                    self.bus.send_to(
                        HostRef::Remote(browse.host),
                        Message::AddProject {
                            path,
                            name: None,
                            colour: None,
                            custom_colour: None,
                            temporary: false,
                        },
                    );
                }
            }
        }
        self.close_host_browse();
        cx.notify();
    }

    /// Raise the picker over the open project's explorer tree, to be answered into one composer.
    ///
    /// Nothing happens with no project open: there is no tree to choose from, and an empty dialog
    /// would say the project has no files rather than that none have been listed.
    pub fn raise_composer_picker(
        &mut self,
        agent: AgentId,
        slot: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forest = self
            .explorer(cx)
            .map(|explorer| crate::state::file_picker::forest_from_explorer(&explorer.root))
            .unwrap_or_default();
        if forest.is_empty() {
            return;
        }
        let request = crate::state::file_picker::PickerRequest::new(
            PickerOwner::Composer { agent, slot },
            "Attach files to the prompt",
        );
        self.open_file_picker(request, forest, PickerView::Tree, window, cx);
    }

    /// Hang what was picked on the conversation as attachments, one tag each.
    ///
    /// Nothing is written into the field: the paths become `@path` mentions only when the prompt
    /// is actually sent, which is `Conversation::compose_prompt`'s job. A path already attached
    /// is not attached twice. The keyboard goes back to the composer either way — the dialog took
    /// it, and the user was in the middle of typing.
    fn attach_files(
        &mut self,
        agent: AgentId,
        slot: usize,
        picked: &[(String, Option<u64>)],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(id) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent)
        {
            for (path, size) in picked {
                conversation.attach(path.clone(), *size);
            }
        }
        if let Some(input) = self.column_inputs.get(slot).cloned() {
            input.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    /// Take one attachment back off — a tag's own dismiss control.
    pub fn detach_file(&mut self, agent: AgentId, attachment: u64, cx: &mut Context<Self>) {
        if let Some(id) = self.project(cx)
            && let Some(open) = self.projects.get_mut(&id)
            && let Some(conversation) = open.conversations.get_mut(&agent)
        {
            conversation.detach(attachment);
        }
        cx.notify();
    }

    /// Take the dialog down with nothing chosen. Dismissed is not the same answer as an empty one,
    /// so whoever asked is told which it was.
    pub fn cancel_file_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.take() else {
            return;
        };
        match picker.request.owner {
            PickerOwner::Sink => {
                self.sink.picker.result = None;
                self.sink.picker.dismissed = true;
            }
            // Nothing to write back: a composer that was not answered keeps what it already had.
            // The keyboard goes back to it, which is where it was before the dialog took it.
            PickerOwner::Composer { slot, .. } => {
                if let Some(input) = self.column_inputs.get(slot).cloned() {
                    input.update(cx, |state, cx| state.focus(window, cx));
                }
            }
            // Nothing to write back here either — closing the dialog with nothing chosen leaves
            // no project to open.
            PickerOwner::HostProject => {}
        }
        self.close_host_browse();
        cx.notify();
    }

    pub fn start_picker_resize(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.start_drag(at);
        }
        cx.notify();
    }

    /// Follow a corner drag. The window is what the dialog has to fit inside, so its size comes in
    /// with the pointer.
    pub fn drag_picker_resize(&mut self, at: (f32, f32), window: &Window, cx: &mut Context<Self>) {
        let viewport = window.viewport_size();
        if let Some(picker) = self.file_picker.as_mut()
            && picker.drag_to(at, (f32::from(viewport.width), f32::from(viewport.height)))
        {
            cx.notify();
        }
    }

    pub fn end_picker_resize(&mut self, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut()
            && picker.is_resizing()
        {
            picker.end_drag();
            cx.notify();
        }
    }

    // ── The log console ────────────────────────────────────────

    pub fn pick_log_subsystem(&mut self, index: usize, cx: &mut Context<Self>) {
        self.logs.pick_subsystem(index);
        self.close_menu(cx);
    }

    pub fn pick_log_level(&mut self, index: usize, cx: &mut Context<Self>) {
        self.logs.pick_level(index);
        self.close_menu(cx);
    }

    pub fn toggle_log_follow(&mut self, cx: &mut Context<Self>) {
        self.logs.follow = !self.logs.follow;
        cx.notify();
    }

    /// Empty the sink. The ring is the whole process's, so this clears every window's console.
    pub fn clear_logs(&mut self, cx: &mut Context<Self>) {
        ubiq_proto::log::logs().clear();
        cx.notify();
    }

    // ── Projects ────────────────────────────────────────────────────
}
