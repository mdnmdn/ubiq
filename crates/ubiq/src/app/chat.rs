use super::*;

impl AppState {
    pub fn send_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.chat.draft.trim().is_empty() {
            return;
        }
        self.chat.send();

        let input = self.chat_input.clone();
        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.chat_scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Show one of the project's conversations in the chat panel.
    ///
    /// Selecting is a change of view and nothing else: the one leaving the panel keeps running,
    /// because the conversation is the host's and the panel is a perspective on it. Selecting also
    /// closes the list — a pick, whether the list was open to make it or already closed, leaves it
    /// closed.
    pub fn select_chat(&mut self, id: AgentId, cx: &mut Context<Self>) {
        self.chat.selected = Some(id);
        self.chat.collapsed = true;
        self.chat_scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Start a conversation from the chat panel, through the same menu the agents screen uses.
    ///
    /// One menu for one question. What the panel adds is that the conversation it starts becomes
    /// the one it is showing — which the agents screen does too, by revealing a column for it.
    pub fn new_chat(&mut self, at: (f32, f32), cx: &mut Context<Self>) {
        self.open_new_agent_menu(at, cx);
    }

    pub fn toggle_tool(&mut self, message: usize, block: usize, cx: &mut Context<Self>) {
        self.chat.toggle_tool(message, block);
        cx.notify();
    }
}

/// The diagram cache, and the queue that fills it.
///
/// Two tiers. The memory one is [`AppState::diagrams`], which every frame reads; the disk one is
/// [`crate::state::diagrams::Disk`], in the project's workarea, which survives a restart. Between
/// them and the renderer is the background executor: **a diagram is never drawn on the frame
/// thread**, because layout is superlinear and a large graph takes seconds.
impl AppState {
    /// What has been drawn for a source, queueing a render if nothing has been.
    ///
    /// Takes `&self` and answers a clone because it is called while the frame is being built: a
    /// viewer is a pure function of bytes and reaches no mutable window. The render is queued
    /// rather than started, and [`AppState::drain_diagram_asks`] starts it once the frame is built.
    pub fn diagram(&self, source: &str) -> DiagramEntry {
        let palette = self.diagram_palette();
        let key = diagrams::key(source, palette);

        let mut cache = self.diagrams.borrow_mut();
        if let Some(entry) = cache.get(&key) {
            return entry.clone();
        }
        cache.insert(key, DiagramEntry::Pending);
        self.diagram_asks
            .borrow_mut()
            .push((source.to_string(), palette));
        DiagramEntry::Pending
    }

    /// Which palette a diagram drawn now is drawn for. The renderer bakes its colours in, so this
    /// is part of what is asked for and part of what it is filed under.
    fn diagram_palette(&self) -> DiagramPalette {
        match self.workbench.theme_id {
            ThemeId::Dark => DiagramPalette::Dark,
            ThemeId::Light => DiagramPalette::Light,
        }
    }

    /// Draw every diagram the frame turned out to need, off the frame thread.
    ///
    /// In `render` for the reason `attach_arrived_files` is: the work belongs to the frame and
    /// cannot be done from inside it. Each render goes to the background executor and comes back
    /// as an entity update — **the window keeps drawing and keeps taking keystrokes while it
    /// runs**, and the viewer shows a pending state until it lands.
    pub(super) fn drain_diagram_asks(&mut self, cx: &mut Context<Self>) {
        let asks = std::mem::take(&mut *self.diagram_asks.borrow_mut());
        if asks.is_empty() {
            return;
        }

        // The disk tier belongs to the project on screen. A window with no project yet renders
        // with the memory tier alone rather than not rendering.
        let dir = self
            .project_snapshot(cx)
            .map(|project| diagrams::cache_dir(&project.workarea));

        for (source, palette) in asks {
            let dir = dir.clone();
            let drawing =
                cx.background_spawn(async move { diagrams::resolve(&source, palette, dir) });
            cx.spawn(async move |this: gpui::WeakEntity<Self>, cx| {
                let answer = drawing.await;
                // A window closed while its diagram was being drawn is not an error.
                let _ = this.update(cx, |this, cx| this.diagram_drawn(answer, cx));
            })
            .detach();
        }
    }

    /// One background render, landed.
    ///
    /// Keyed by the content key the render was filed under, so an answer finds its entry however
    /// long it took and whatever the window has done since — including a theme switch, which asks
    /// for a different key and leaves this one to be found again on the way back.
    fn diagram_drawn(&mut self, answer: DiagramAnswer, cx: &mut Context<Self>) {
        let entry = match answer.result {
            Ok(image) => DiagramEntry::Ready(diagram_picture(image)),
            Err(reason) => DiagramEntry::Failed(reason),
        };
        self.diagrams.borrow_mut().insert(answer.key, entry);
        cx.notify();
    }

    /// The camera on one picture, or the fitted default if the user has not touched it.
    pub fn viewport(&self, key: &str) -> Viewport {
        self.viewports
            .borrow()
            .get(key)
            .copied()
            .unwrap_or_default()
    }

    /// Remember the picture's own rectangle so a later wheel or drag can pin the fit.
    pub fn touch_viewport(&self, key: &str, content: Content) {
        self.viewports
            .borrow_mut()
            .entry(key.to_string())
            .or_default()
            .set_content(content);
    }

    /// Remember the panel a picture was just laid out in. Returns whether it went from unmeasured
    /// to measured, which is the one change that owes the window another frame — a resize already
    /// asked for one.
    pub fn note_viewport_panel(&self, key: &str, bounds: Bounds<Pixels>) -> bool {
        self.viewports
            .borrow_mut()
            .entry(key.to_string())
            .or_default()
            .set_panel(
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
            )
    }

    pub fn zoom_viewport(
        &mut self,
        key: &str,
        factor: f32,
        cursor: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if let Some(vp) = self.viewports.borrow_mut().get_mut(key) {
            vp.zoom_at(factor, f32::from(cursor.x), f32::from(cursor.y));
        }
        cx.notify();
    }

    pub fn start_viewport_drag(
        &mut self,
        key: &str,
        at: gpui::Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        *self.viewport_drag.borrow_mut() = Some((key.to_string(), at));
        cx.notify();
    }

    pub fn drag_viewport(&mut self, key: &str, at: gpui::Point<Pixels>, cx: &mut Context<Self>) {
        let last = {
            let drag = self.viewport_drag.borrow();
            match drag.as_ref() {
                Some((held, last)) if held == key => Some(*last),
                _ => None,
            }
        };
        let Some(last) = last else {
            return;
        };
        let dx = f32::from(at.x - last.x);
        let dy = f32::from(at.y - last.y);
        if let Some(vp) = self.viewports.borrow_mut().get_mut(key) {
            vp.pan_by(dx, dy);
        }
        *self.viewport_drag.borrow_mut() = Some((key.to_string(), at));
        cx.notify();
    }

    pub fn end_viewport_drag(&mut self, cx: &mut Context<Self>) {
        if self.viewport_drag.borrow_mut().take().is_some() {
            cx.notify();
        }
    }

    pub fn reset_viewport(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(vp) = self.viewports.borrow_mut().get_mut(key) {
            vp.reset();
        }
        cx.notify();
    }
}
