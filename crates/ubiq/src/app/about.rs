//! The About modal: opening it, switching its tabs, and the host's list of releases.
//!
//! Every control in it is one the updater already answers — checking, downloading, restarting,
//! following a channel are `app::updates`'. The modal adds one question of its own,
//! [`Message::QueryReleases`], asked each time it opens.

use crate::state::about::{AboutState, AboutTab};

use super::*;

impl AppState {
    /// Press the mark: raise the modal on its first tab and ask what every channel offers.
    pub fn open_about(&mut self, cx: &mut Context<Self>) {
        if self.workbench.about.is_some() {
            return;
        }
        self.workbench.open_menu = None;
        self.workbench.about = Some(AboutState::default());
        self.refresh_releases(cx);
    }

    pub fn close_about(&mut self, cx: &mut Context<Self>) {
        self.workbench.about = None;
        cx.notify();
    }

    pub fn set_about_tab(&mut self, tab: AboutTab, cx: &mut Context<Self>) {
        if let Some(about) = self.workbench.about.as_mut() {
            about.tab = tab;
            cx.notify();
        }
    }

    /// Ask again: the list goes back to "asking" until the host answers.
    pub fn refresh_releases(&mut self, cx: &mut Context<Self>) {
        if let Some(about) = self.workbench.about.as_mut() {
            about.releases = None;
        }
        self.bus.send_to(HostRef::Local, Message::QueryReleases);
        cx.notify();
    }

    /// The host's answer. Dropped when the modal has closed meanwhile: it is asked again on open.
    pub(super) fn receive_releases(
        &mut self,
        channels: Vec<ubiq_proto::update::ChannelRelease>,
        cx: &mut Context<Self>,
    ) {
        if let Some(about) = self.workbench.about.as_mut() {
            about.releases = Some(channels);
            cx.notify();
        }
    }

    /// The About tab's one action: take the update one step further — download it when one is
    /// available, restart into it once it is ready, and open the page when this install can only
    /// be updated by hand.
    pub fn take_update(&mut self, cx: &mut Context<Self>) {
        use ubiq_proto::update::{ApplyMode, UpdateStatus};
        match &self.workbench.updates.status {
            UpdateStatus::Available {
                apply: ApplyMode::Manual,
                info,
            } => cx.open_url(&info.notes_url),
            UpdateStatus::Available { .. } => self.download_update(cx),
            UpdateStatus::Ready { .. } => self.request_restart_to_update(cx),
            _ => {}
        }
    }
}
