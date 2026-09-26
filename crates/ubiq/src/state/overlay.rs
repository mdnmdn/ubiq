//! The window's overlay stack, as an order.
//!
//! **An outside click belongs to the topmost layer.** `on_mouse_down_out` is a capture-phase
//! handler over one panel's own bounds, and a layer painted above that panel is outside it: a
//! click in the higher layer reads to every layer below as a click outside *them*, and no
//! `stop_propagation` from above can undo it, because capture runs back to front. Left alone,
//! one gesture peels the whole stack.
//!
//! So a dismissal asks what is above it first. [`Layer`] is the rung each overlay occupies,
//! declared bottom-up in `ui::shell`'s paint order, and `AppState::covered` answers with the
//! derived `Ord`: while anything sits above, the click is not this layer's and the layer stays
//! up. `AppState::cancel_dialog` reads the same order from the top for Escape — one gesture,
//! one layer, either way.
//!
//! **Adding an overlay is a rung here and an arm in `AppState::top_layer`, and nothing else.**
//! Every layer below it is protected from it without being touched, which is the whole point of
//! one order rather than a guard per pair.

/// One rung of the window's overlay stack, bottom first.
///
/// The order is `ui::shell`'s paint order — the `.children(...)` list at the window root, read
/// top to bottom — and the `Ord` derived from it is what every dismissal consults. A layer
/// painted inside another's element (the New agent modal's name prompt) gets its own rung
/// straight above the one it is drawn over.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// The project settings page.
    ProjectSettings,
    /// The knowledge base's "Add source" question, over that page.
    KbSource,
    /// The application settings page.
    Settings,
    /// The harness login modal, over the settings page.
    Login,
    /// The New agent modal.
    NewAgent,
    /// Its "Save agent" name prompt, painted over it.
    NewAgentNaming,
    /// The new-mission dialog, raised from the tasks board's toolbar.
    NewMission,
    /// The agent-definition form, beside the login modal.
    AgentDefinitionForm,
    /// The accounts section's rename, delete or sign-out question.
    AccountDialog,
    /// The connect flow.
    Connect,
    /// The application-registration form.
    AppForm,
    /// The connectors section's rename, disconnect or forget question.
    Connector,
    /// The certificate question, which interrupts a running connect flow.
    Cert,
    /// The API-provider form.
    AiForm,
    /// The provider test.
    AiTest,
    /// The provider removal question.
    AiRemove,
    /// The SSH-profile form.
    SshForm,
    /// The SSH-profile removal question.
    SshRemove,
    /// The Stop-drone question.
    DroneStop,
    /// The clone modal.
    Clone,
    /// The task-import dialog: what the binding's filter currently offers, to tick. Above the
    /// project settings page because the task-sync section raises it, and raisable from the board
    /// as well, which is why it is a rung of its own rather than a part of that page.
    TaskImport,
    /// The feedback modal.
    Feedback,
    /// An agent's question to the user. Above the feedback modal on the same terms — it is raised
    /// from the transcript, and from the host at any moment — and it is the one modal that is
    /// never raised over another: an ask arriving while anything is up leaves a notification and
    /// a transcript entry instead.
    Ask,
    /// The "All projects" modal.
    AllProjects,
    /// One mission's full view, framed as a modal — raised from the side panel's `⤢` outside IDE
    /// mode (§6.2). Below the plan, because the full view's *Plan & docs* tab is one of the
    /// places that raises the plan surface over it.
    Mission,
    /// A task's plan, read as rendered markdown — raised from the task panel.
    Plan,
    /// The image/diagram zoom modal (T-185) — raised from a corner button on a scaled-down
    /// picture in a markdown preview, wherever one is drawn, so it sits above both the standard
    /// viewer's tab and the plan surface's own rendered markdown.
    ImageZoom,
    /// What one ACP harness said it can do (`T-207`) — raised from the harnesses settings section
    /// and from a conversation's info modal. Above both, because it is a reading raised *over*
    /// whichever of them asked for it, and Escape means "put the reading away" while it is up.
    Capabilities,
    /// The file question — new, rename, save-as and the rest, and the plan modal's own Export
    /// prompt, painted over it on the same terms `FileDialog` already sits over everything else
    /// that can raise it.
    FileDialog,
    /// The size preset's name prompt. Above the settings page because the Size section raises it,
    /// and it is drawn at the window root because the status bar's popover raises the same one.
    SizeNaming,
    /// The theme editor. Above the settings page because the Appearance section raises it, and
    /// drawn at the window root for the same reason the size prompt is.
    ThemeEditor,
    /// The theme's name prompt — **New theme…**, and the editor's Rename. Above the editor,
    /// because the editor is one of the two places that raises it.
    ThemeNaming,
    /// The terminal tab's Close confirm.
    ClosePane,
    /// The chat tab's Close confirm.
    EndConversation,
    /// The file picker dialog.
    FilePicker,
    /// A context or chevron menu at the window root.
    Menu,
    /// The remote-hosts manager.
    RemoteManager,
    /// The remote-connect modal, over that manager.
    RemoteConnect,
    /// The bell's list, painted last of the window's overlays.
    Notifications,
    /// Any dropdown a form keeps open on its own state — a picker inside a modal is painted above
    /// every modal, so it is the top rung whatever raised it.
    Dropdown,
    /// In-place help's targeting mode, above every other rung in the window.
    ///
    /// **Deliberately the top, and the one layer that is not a dialog.** Everything below is
    /// something the user is *doing*; this one is a question about what they are looking at, and
    /// a dialog's own controls are exactly the things hardest to name — so the mode has to be
    /// able to cover one and point at it, the way a browser's inspector inspects its own page
    /// furniture. The consequences fall out of the order for free: Escape peels it before any
    /// modal under it, and no layer below can dismiss itself on a click that happened up here.
    HelpTarget,
}
