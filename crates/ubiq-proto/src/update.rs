//! Updating Ubiq itself: which channel a build follows, what the host found, and how it can apply.
//!
//! Like feedback, this is a host-level family with no pane: the destination (a signed feed) is
//! compiled into the build, and the host does the network, the verification and the swap. The
//! interface only draws [`UpdateStatus`] and sends the user's choices back.
//!
//! The build's own channel is stated here ([`UpdateChannel::build`]) rather than in the host so
//! [`UpdateSettings::default`] and the host's feed selection can never disagree about it.

use serde::{Deserialize, Serialize};

/// A release stream. Ordered by how much churn the user accepts: nightly includes beta includes
/// stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    Stable,
    Beta,
    Nightly,
}

impl UpdateChannel {
    /// Every channel, in the order a picker offers them.
    pub const ALL: &'static [Self] = &[Self::Stable, Self::Beta, Self::Nightly];

    /// Whether following `self` also accepts releases published on `other`.
    pub fn includes(self, other: Self) -> bool {
        self.rank() >= other.rank()
    }

    fn rank(self) -> u8 {
        match self {
            Self::Stable => 0,
            Self::Beta => 1,
            Self::Nightly => 2,
        }
    }

    /// The feed spelling: the manifest file is `<slug>.json`.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Nightly => "nightly",
        }
    }

    /// What the control says.
    pub fn label(self) -> &'static str {
        match self {
            Self::Stable => "Stable",
            Self::Beta => "Beta",
            Self::Nightly => "Nightly",
        }
    }

    /// The channel a version string belongs to: `-nightly` is nightly, any other prerelease is
    /// beta, a plain version is stable.
    pub fn of_version(version: &str) -> Self {
        let version = version.trim_start_matches('v');
        match version.split_once('-') {
            None => Self::Stable,
            Some((_, pre)) if pre.starts_with("nightly") => Self::Nightly,
            Some(_) => Self::Beta,
        }
    }

    /// The channel this build was compiled for: `UBIQ_CHANNEL` if set and valid, else derived from
    /// `UBIQ_VERSION`, else stable.
    pub fn build() -> Self {
        let named = match option_env!("UBIQ_CHANNEL") {
            Some("stable") => Some(Self::Stable),
            Some("beta") => Some(Self::Beta),
            Some("nightly") => Some(Self::Nightly),
            _ => None,
        };
        named.unwrap_or_else(|| option_env!("UBIQ_VERSION").map_or(Self::Stable, Self::of_version))
    }
}

/// What the user chose about updates. Persisted by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateSettings {
    pub channel: UpdateChannel,
    /// Look for an update on a schedule (30 s after start, then every 6 hours).
    pub auto_check: bool,
    /// Download an available update without being asked. Applying it is always the user's call.
    pub auto_download: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            channel: UpdateChannel::build(),
            auto_check: true,
            auto_download: false,
        }
    }
}

/// A release the host found, as the interface shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub channel: UpdateChannel,
    /// When it was published, as the feed wrote it.
    pub published: String,
    /// The release notes page.
    pub notes_url: String,
    /// The download's size in bytes.
    pub size: u64,
}

/// What one channel publishes for this platform, as [`Message::Releases`] lists it.
///
/// [`Message::Releases`]: crate::messages::Message::Releases
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRelease {
    pub channel: UpdateChannel,
    /// The channel's newest release. `None` when it publishes nothing for this platform, or could
    /// not be read — `error` says which.
    pub release: Option<UpdateInfo>,
    /// A sentence for the user when the channel could not be read (or the build cannot read any).
    pub error: Option<String>,
}

/// How this install can apply an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyMode {
    /// Replace the app bundle in place (macOS, a writable bundle).
    Swap,
    /// Run the installer silently (Windows, an installed copy).
    Installer,
    /// Nothing the host can do safely: the interface links the download or the notes page.
    Manual,
}

/// Where the updater is, drawn by the interface as one line plus at most one button.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateStatus {
    /// This build cannot update itself, and `reason` is a sentence saying why.
    Disabled {
        reason: String,
    },
    Idle,
    Checking,
    /// The newest release is this one or older. `checked` is when the host last looked.
    UpToDate {
        checked: String,
    },
    Available {
        info: UpdateInfo,
        apply: ApplyMode,
    },
    Downloading {
        info: UpdateInfo,
        received: u64,
        total: u64,
    },
    /// Downloaded and verified, waiting to be applied.
    Ready {
        info: UpdateInfo,
    },
    /// The helper has been started; the app is expected to quit.
    Applying {
        info: UpdateInfo,
    },
    /// `message` is a sentence for the user.
    Failed {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_nest() {
        use UpdateChannel::*;
        assert!(Nightly.includes(Beta) && Nightly.includes(Stable) && Nightly.includes(Nightly));
        assert!(Beta.includes(Stable) && Beta.includes(Beta) && !Beta.includes(Nightly));
        assert!(Stable.includes(Stable) && !Stable.includes(Beta) && !Stable.includes(Nightly));
    }

    #[test]
    fn a_version_names_its_channel() {
        assert_eq!(UpdateChannel::of_version("v1.4.0"), UpdateChannel::Stable);
        assert_eq!(
            UpdateChannel::of_version("1.5.0-beta.1"),
            UpdateChannel::Beta
        );
        assert_eq!(
            UpdateChannel::of_version("1.5.0-nightly.20260101"),
            UpdateChannel::Nightly
        );
        assert_eq!(UpdateChannel::of_version("1.5.0-rc.1"), UpdateChannel::Beta);
    }

    #[test]
    fn channels_serialise_lowercase() {
        assert_eq!(
            serde_json::to_string(&UpdateChannel::Nightly).unwrap(),
            "\"nightly\""
        );
    }
}
