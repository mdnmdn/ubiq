//! The "Remote hosts" manager panel's actions: open and close, rename, and the per-row
//! connect, disconnect, reconnect and test.
//!
//! The panel itself is `ui::remote_hosts`; the dials it triggers are `remote_connect`'s. What
//! lives here is only the chrome around them: which saved entry a row acts on, and where the
//! rename prompt's text goes.

use super::*;

impl AppState {
    /// Raise the manager panel. The connect modal stays where it is when it is up — the panel
    /// paints under it, so "New connection" from here lands on top.
    pub fn open_remote_manager(&mut self, cx: &mut Context<Self>) {
        self.workbench.remote_manager.open = true;
        cx.notify();
    }

    /// Close the panel, and with it a rename prompt or a test result left on it. Live
    /// connections, saved entries and reconnect loops are untouched — closing the panel stops
    /// nothing, it only stops showing it.
    pub fn close_remote_manager(&mut self, cx: &mut Context<Self>) {
        self.workbench.remote_manager.open = false;
        self.workbench.remote_manager.renaming = None;
        cx.notify();
    }

    /// Start renaming a saved entry: the prompt opens with the current name in it.
    pub fn begin_rename_remote_host(
        &mut self,
        save_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self
            .workbench
            .settings
            .host
            .remote_hosts
            .iter()
            .find(|host| host.id == save_id)
            .map(|host| host.name.clone())
            .unwrap_or_default();
        self.remote_rename_input.update(cx, |state, cx| {
            state.set_value(&current, window, cx);
        });
        self.workbench.remote_manager.renaming = Some(save_id);
        cx.notify();
    }

    /// Confirm the rename prompt. An empty name confirms nothing — the button dims rather than
    /// refusing, the same way every other prompt in this window does.
    pub fn confirm_rename_remote_host(&mut self, cx: &mut Context<Self>) {
        let Some(save_id) = self.workbench.remote_manager.renaming.clone() else {
            return;
        };
        let name = self.remote_rename_input.read(cx).value().to_string();
        self.workbench.remote_manager.renaming = None;
        self.rename_remote_host(save_id, name, cx);
    }

    /// Dismiss the rename prompt, keeping the old name.
    pub fn cancel_rename_remote_host(&mut self, cx: &mut Context<Self>) {
        self.workbench.remote_manager.renaming = None;
        cx.notify();
    }

    /// Connect a saved entry from its manager row: raise the connect modal with everything but
    /// a missing token filled in. A reconnect loop left behind for it stops — this dial
    /// replaces the loop rather than racing it.
    pub fn connect_saved_host(
        &mut self,
        save_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved = self
            .workbench
            .settings
            .host
            .remote_hosts
            .iter()
            .find(|host| host.id == save_id)
            .cloned();
        let Some(saved) = saved else {
            return;
        };
        let key = host_secrets::key_for(&saved.id, &saved.address);
        self.workbench.settings.reconnects.remove(&key);
        self.reconnect_saved_host(
            saved.id,
            saved.name,
            saved.address,
            saved.scheme,
            saved.trust_insecure,
            window,
            cx,
        );
    }
}
