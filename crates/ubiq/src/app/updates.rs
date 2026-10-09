//! Ubiq updating itself: what the Updates section and the status bar ask of the host, and what the
//! host says back.
//!
//! **The host does the work; this half only asks and draws.** Checking, downloading, verifying and
//! swapping are the host's (`ubiq_host::updates`). The one thing it cannot do is quit the
//! application, so "Restart and install" sends [`Message::ApplyUpdate`] and then quits through the
//! same `cx.quit()` the quit confirm uses, with the same question put first when the window holds
//! unsaved files or running agents.

use ubiq_proto::update::{UpdateChannel, UpdateSettings};

use crate::state::FileDialog;
use crate::state::updates::UpdatesState;

use super::*;

impl AppState {
    /// Ask where the updater is. Sent when the window attaches; every later change is broadcast.
    pub fn ask_updates(&mut self) {
        self.bus.send_to(HostRef::Local, Message::QueryUpdates);
    }

    pub fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        self.bus.send_to(HostRef::Local, Message::CheckForUpdates);
        cx.notify();
    }

    pub fn download_update(&mut self, cx: &mut Context<Self>) {
        self.bus.send_to(HostRef::Local, Message::DownloadUpdate);
        cx.notify();
    }

    pub fn set_update_channel(&mut self, channel: UpdateChannel, cx: &mut Context<Self>) {
        self.save_update_settings(
            UpdateSettings {
                channel,
                ..self.workbench.updates.settings
            },
            cx,
        );
    }

    pub fn toggle_update_auto_check(&mut self, cx: &mut Context<Self>) {
        let settings = self.workbench.updates.settings;
        self.save_update_settings(
            UpdateSettings {
                auto_check: !settings.auto_check,
                ..settings
            },
            cx,
        );
    }

    pub fn toggle_update_auto_download(&mut self, cx: &mut Context<Self>) {
        let settings = self.workbench.updates.settings;
        self.save_update_settings(
            UpdateSettings {
                auto_download: !settings.auto_download,
                ..settings
            },
            cx,
        );
    }

    /// Optimistic: the pill moves at once, and the host's rebroadcast confirms it.
    fn save_update_settings(&mut self, settings: UpdateSettings, cx: &mut Context<Self>) {
        self.workbench.updates.settings = settings;
        self.bus
            .send_to(HostRef::Local, Message::SaveUpdateSettings { settings });
        cx.notify();
    }

    /// "Install on quit": the host installs after this process exits, and nothing quits now.
    pub fn install_update_on_quit(&mut self, cx: &mut Context<Self>) {
        self.workbench.updates.on_quit = true;
        self.bus
            .send_to(HostRef::Local, Message::ApplyUpdate { relaunch: false });
        cx.notify();
    }

    /// "Restart and install". With anything a restart would take with it, the question comes
    /// first; the message is sent only on the yes.
    pub fn request_restart_to_update(&mut self, cx: &mut Context<Self>) {
        if self.unsaved_summary(cx).is_empty() {
            self.restart_to_update(cx);
        } else {
            self.workbench.file_dialog = Some(FileDialog::RestartForUpdate);
            cx.notify();
        }
    }

    pub(super) fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        self.bus
            .send_to(HostRef::Local, Message::ApplyUpdate { relaunch: true });
        cx.quit();
    }

    /// Open the application settings on the Updates section — the status bar item's click.
    pub fn open_updates_settings(&mut self, cx: &mut Context<Self>) {
        if !self.workbench.settings.open {
            self.toggle_settings(cx);
        }
        self.set_settings_nav(
            crate::state::settings::SettingsSection(crate::ext::ids::UPDATES),
            cx,
        );
    }

    /// The update path of the host's answer. Answers with the message when it belongs to another
    /// family, on the convention every `receive_*` in `wire.rs` follows.
    pub(super) fn receive_updates(
        &mut self,
        host: HostRef,
        message: Message,
        cx: &mut Context<Self>,
    ) -> Option<Message> {
        match message {
            // Only the local host's: updating is about this binary, not a remote host's.
            Message::UpdateState {
                status,
                settings,
                current_version,
            } => {
                if host == HostRef::Local {
                    use ubiq_proto::update::UpdateStatus as S;
                    let keep = matches!(status, S::Ready { .. } | S::Applying { .. });
                    let updates = &mut self.workbench.updates;
                    updates.on_quit &= keep;
                    *updates = UpdatesState {
                        status,
                        settings,
                        current_version,
                        on_quit: updates.on_quit,
                    };
                    cx.notify();
                }
            }
            Message::Releases { channels } => {
                if host == HostRef::Local {
                    self.receive_releases(channels, cx);
                }
            }
            other => return Some(other),
        }
        None
    }
}
