//! What the interface knows about Ubiq updating itself: the host's last [`Message::UpdateState`].
//!
//! Data and small readers only. The host owns the check, the download and the swap; this holds
//! the answer so the Updates section and the status bar draw the same fact.
//!
//! [`Message::UpdateState`]: ubiq_proto::messages::Message::UpdateState

use ubiq_proto::update::{ApplyMode, UpdateInfo, UpdateSettings, UpdateStatus};

/// The updater as the window last heard of it.
#[derive(Clone, Debug)]
pub struct UpdatesState {
    pub status: UpdateStatus,
    pub settings: UpdateSettings,
    pub current_version: String,
    /// `ApplyUpdate { relaunch: false }` was sent: the install waits for the next quit.
    pub on_quit: bool,
}

impl Default for UpdatesState {
    /// Before the host has answered: nothing is known, so nothing is offered.
    fn default() -> Self {
        Self {
            status: UpdateStatus::Idle,
            settings: UpdateSettings::default(),
            current_version: String::new(),
            on_quit: false,
        }
    }
}

impl UpdatesState {
    /// The release a status is about, where it is about one.
    pub fn info(&self) -> Option<&UpdateInfo> {
        match &self.status {
            UpdateStatus::Available { info, .. }
            | UpdateStatus::Downloading { info, .. }
            | UpdateStatus::Ready { info }
            | UpdateStatus::Applying { info } => Some(info),
            _ => None,
        }
    }

    /// Whether the status bar has something to say: an update to take, or one waiting to be
    /// applied. Not while disabled, current, checking or failed.
    pub fn pending(&self) -> bool {
        matches!(
            self.status,
            UpdateStatus::Available { .. } | UpdateStatus::Ready { .. }
        )
    }

    /// Whether the user can act on the controls at all.
    pub fn enabled(&self) -> bool {
        !matches!(self.status, UpdateStatus::Disabled { .. })
    }

    /// Whether this install can only link the download, never apply it.
    pub fn manual(&self) -> bool {
        matches!(
            self.status,
            UpdateStatus::Available {
                apply: ApplyMode::Manual,
                ..
            }
        )
    }

    /// The download's progress as a whole percentage.
    pub fn percent(received: u64, total: u64) -> u64 {
        received
            .saturating_mul(100)
            .checked_div(total)
            .map_or(0, |percent| percent.min(100))
    }

    /// The status as one line.
    pub fn line(&self) -> String {
        match &self.status {
            UpdateStatus::Disabled { reason } => format!("Disabled: {reason}"),
            UpdateStatus::Idle => "Not checked yet".to_string(),
            UpdateStatus::Checking => "Checking\u{2026}".to_string(),
            UpdateStatus::UpToDate { checked } => format!("Up to date \u{2014} checked {checked}"),
            UpdateStatus::Available { info, .. } => format!("v{} available", bare(&info.version)),
            UpdateStatus::Downloading {
                received, total, ..
            } => format!("Downloading {}%", Self::percent(*received, *total)),
            UpdateStatus::Ready { info } | UpdateStatus::Applying { info } if self.on_quit => {
                format!("v{} will install on quit", bare(&info.version))
            }
            UpdateStatus::Ready { info } => {
                format!("v{} is ready to install", bare(&info.version))
            }
            UpdateStatus::Applying { info } => {
                format!("Installing v{}\u{2026}", bare(&info.version))
            }
            UpdateStatus::Failed { message } => format!("Failed: {message}"),
        }
    }
}

fn bare(version: &str) -> &str {
    version.trim_start_matches('v')
}

#[cfg(test)]
mod tests {
    use super::*;
    use ubiq_proto::update::UpdateChannel;

    fn info() -> UpdateInfo {
        UpdateInfo {
            version: "v1.2.0".into(),
            channel: UpdateChannel::Stable,
            published: String::new(),
            notes_url: String::new(),
            size: 10,
        }
    }

    #[test]
    fn the_bar_speaks_only_for_available_and_ready() {
        let mut state = UpdatesState::default();
        assert!(!state.pending());
        state.status = UpdateStatus::Available {
            info: info(),
            apply: ApplyMode::Swap,
        };
        assert!(state.pending());
        state.status = UpdateStatus::Ready { info: info() };
        assert!(state.pending());
        state.status = UpdateStatus::Disabled {
            reason: "dev build".into(),
        };
        assert!(!state.pending() && !state.enabled());
    }

    #[test]
    fn lines_read_plainly() {
        let mut state = UpdatesState {
            status: UpdateStatus::Downloading {
                info: info(),
                received: 50,
                total: 200,
            },
            ..UpdatesState::default()
        };
        assert_eq!(state.line(), "Downloading 25%");
        state.status = UpdateStatus::Ready { info: info() };
        state.on_quit = true;
        assert_eq!(state.line(), "v1.2.0 will install on quit");
    }
}
