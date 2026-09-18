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
    /// Its "Save profile" name prompt, painted over it.
    NewAgentNaming,
    /// The profile form, beside the login modal.
    ProfileForm,
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
    /// The feedback modal.
    Feedback,
    /// The "All projects" modal.
    AllProjects,
    /// The file question — new, rename, save-as and the rest.
    FileDialog,
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
}
