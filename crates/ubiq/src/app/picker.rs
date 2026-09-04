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
    pub fn press_picker_key(&mut self, key: PickerKey, cx: &mut Context<Self>) -> bool {
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
                cx.notify();
                true
            }
            Pressed::Commit => {
                self.commit_file_picker(cx);
                true
            }
            Pressed::Dismiss => {
                self.cancel_file_picker(cx);
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

    pub fn toggle_picker_folder(&mut self, path: String, cx: &mut Context<Self>) {
        if let Some(picker) = self.file_picker.as_mut() {
            picker.toggle_folder(&path);
        }
        cx.notify();
    }

    /// What a click on a row does, which the picker itself decides: a folder that cannot be picked
    /// opens, and a pick that was asked to be final closes the dialog on the spot.
    pub fn click_picker_row(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.as_mut() else {
            return;
        };
        if picker.click(&path) {
            self.commit_file_picker(cx);
            return;
        }
        cx.notify();
    }

    /// Hand what was chosen to whoever asked for it, and take the dialog down.
    pub fn commit_file_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.take() else {
            return;
        };
        let picked = picker.picked().to_vec();
        match picker.request.owner {
            PickerOwner::Sink => {
                self.sink.picker.result = Some(picked);
                self.sink.picker.dismissed = false;
            }
        }
        cx.notify();
    }

    /// Take the dialog down with nothing chosen. Dismissed is not the same answer as an empty one,
    /// so whoever asked is told which it was.
    pub fn cancel_file_picker(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.file_picker.take() else {
            return;
        };
        match picker.request.owner {
            PickerOwner::Sink => {
                self.sink.picker.result = None;
                self.sink.picker.dismissed = true;
            }
        }
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
