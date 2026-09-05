use super::*;

impl AppState {
    /// Mint a fresh chat tab, attached to nothing, and give it a composer slot of its own.
    /// `None` only when there is no project on screen or every chat slot is already taken.
    ///
    /// This never puts the tab in the dock — the two callers that mint one (a `+` in the panel's
    /// own header, and [`Self::toggle_region`] filling an empty right region) each decide how it
    /// reaches the tree.
    pub(super) fn open_chat_tab(&mut self, cx: &mut Context<Self>) -> Option<ChatId> {
        let project = self.project(cx)?;
        let open = self.projects.get_mut(&project)?;
        let slot = free_chat_slot(&open.chats)?;
        let id = ChatId::generate();
        open.chats.push(ChatTab {
            id,
            slot,
            attached: None,
            picker_open: false,
        });
        Some(id)
    }

    /// The `+` beside *New chat*: a new **view**, attached to nothing, beside whatever tabs are
    /// already open. Distinct from [`Self::new_chat`], which starts a new **harness** — one adds
    /// a perspective, the other adds a conversation to have one on.
    pub fn new_chat_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.open_chat_tab(cx) {
            self.pending_panels
                .push(PanelEdit::Open(PanelKind::Chat(id)));
        }
        cx.notify();
    }

    /// Attach one chat tab to a conversation, or to nothing. The one place this is done, so the
    /// picker's pick and a freshly started conversation's own attach both go through it.
    pub fn attach_chat(&mut self, id: ChatId, agent: Option<AgentId>, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(tab) = open.chats.iter_mut().find(|tab| tab.id == id)
        {
            tab.attached = agent;
            tab.picker_open = false;
        }
        cx.notify();
    }

    /// Open or shut one chat tab's own attach picker. Per tab, like a conversation's own
    /// pre-launch config picker: several may be down at once.
    pub fn toggle_chat_picker(&mut self, id: ChatId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(tab) = open.chats.iter_mut().find(|tab| tab.id == id)
        {
            tab.picker_open = !tab.picker_open;
        }
        // A fresh search on every open, the way every searchable picker in the window starts one.
        let picker_search = self.picker_search.clone();
        picker_search.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }

    /// Dismiss one chat tab's attach picker without picking — an outside click.
    pub fn dismiss_chat_picker(&mut self, id: ChatId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            return;
        };
        if let Some(open) = self.projects.get_mut(&project)
            && let Some(tab) = open.chats.iter_mut().find(|tab| tab.id == id)
        {
            tab.picker_open = false;
        }
        cx.notify();
    }

    /// Start a conversation from one chat tab's own *New chat*, through the same menu the agents
    /// screen uses.
    ///
    /// One menu for one question. What the tab adds is that the conversation it starts becomes
    /// the one it shows — `AppState::pending_chat_attach` carries which tab asked across the round
    /// trip to [`Self::pick_new_agent_menu`], which is where the attach actually happens.
    pub fn new_chat(&mut self, id: ChatId, at: (f32, f32), cx: &mut Context<Self>) {
        self.pending_chat_attach = Some(id);
        self.open_new_agent_menu(at, cx);
    }

    /// Close a chat tab, panel and all — the gesture, as opposed to
    /// [`Self::closed_chat_tab`], which is what the dock calls back once a tab has already gone.
    ///
    /// The tab goes here and its panel leaves through a `Window` this does not have — so the
    /// panel edit queues, exactly as [`Self::close_editor_tab`] queues a file's. **The state has
    /// to be dropped here rather than left to the callback**: `PanelEdit::Close` takes the panel
    /// entity out before the dock removes the leaf, and `on_removed`'s deferred answer reads that
    /// entity to tell a close from a displacement — a dead one reads as displaced, so
    /// `closed_chat_tab` never runs for a close this method started. `closed_chat_tab` stays the
    /// path for a close the dock itself began, and is idempotent, so the two never collide.
    ///
    /// Closing the last one is allowed: there is no last-tab guard anywhere in this tree, and a
    /// chat tab is a view — the conversation it was looking at is the host's and outlives it.
    pub fn close_chat_tab(&mut self, id: ChatId, cx: &mut Context<Self>) {
        let slot = self.project(cx).and_then(|project| {
            let open = self.projects.get_mut(&project)?;
            let at = open.chats.iter().position(|tab| tab.id == id)?;
            Some(open.chats.remove(at).slot)
        });
        if let Some(slot) = slot
            && let Some(agents) = self.agents_mut(cx)
        {
            agents.clear_draft(slot);
        }
        self.pending_panels
            .push(PanelEdit::Close(PanelKind::Chat(id)));
        cx.notify();
    }

    /// A chat panel left the dock for good. The conversation it was attached to, if any, is the
    /// host's and keeps running — only the tab and its composer slot go.
    pub fn closed_chat_tab(&mut self, id: ChatId, cx: &mut Context<Self>) {
        let Some(project) = self.project(cx) else {
            self.panels.remove(&PanelKind::Chat(id));
            return;
        };
        let Some(open) = self.projects.get_mut(&project) else {
            self.panels.remove(&PanelKind::Chat(id));
            return;
        };
        let Some(at) = open.chats.iter().position(|tab| tab.id == id) else {
            // The tab belonged to a project this window has since switched away from — the panel
            // simply follows it out, the way an unmatched key lets a file panel's.
            self.panels.remove(&PanelKind::Chat(id));
            return;
        };
        let slot = open.chats.remove(at).slot;
        self.panels.remove(&PanelKind::Chat(id));
        if let Some(agents) = self.agents_mut(cx) {
            agents.clear_draft(slot);
        }
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
