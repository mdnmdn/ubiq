//! The About modal: which tab is up, what each update channel offers, and the open-source
//! packages this build is made of.
//!
//! Data and small readers only. The releases are the host's answer to
//! [`Message::QueryReleases`]; the package list is `assets/third-party.json`, written by
//! `just licenses` from `cargo metadata` and baked into the binary, so a build always lists exactly
//! what it was built from.
//!
//! [`Message::QueryReleases`]: ubiq_proto::messages::Message::QueryReleases

use std::sync::OnceLock;

use serde::Deserialize;
use ubiq_proto::update::ChannelRelease;

/// The modal's three tabs, in strip order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AboutTab {
    #[default]
    About,
    Releases,
    Licences,
}

impl AboutTab {
    pub const ALL: &'static [Self] = &[Self::About, Self::Releases, Self::Licences];

    pub fn label(self) -> &'static str {
        match self {
            Self::About => "About",
            Self::Releases => "Releases",
            Self::Licences => "Open source",
        }
    }
}

/// The modal, while it is up.
#[derive(Clone, Debug, Default)]
pub struct AboutState {
    pub tab: AboutTab,
    /// What every channel offers, one entry per `UpdateChannel::ALL`. `None` while the question is
    /// out — asked each time the modal opens, because a feed changes while the app runs.
    pub releases: Option<Vec<ChannelRelease>>,
}

/// One third-party package, as `_tools/licenses.py` wrote it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct ThirdParty {
    pub name: String,
    pub version: String,
    /// The SPDX expression the package declares, where it declares one.
    pub license: Option<String>,
    /// One SPDX page per licence the expression names.
    pub license_urls: Vec<String>,
    /// The package's own licence file, where one is published.
    pub license_file_url: Option<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    pub authors: Vec<String>,
    /// The copyright line from the package's licence file.
    pub copyright: Option<String>,
}

impl ThirdParty {
    /// The licence as the list prints it.
    pub fn license_label(&self) -> &str {
        self.license.as_deref().unwrap_or("unstated")
    }

    /// Who the package is credited to: its copyright line, else its authors.
    pub fn attribution(&self) -> String {
        match &self.copyright {
            Some(line) if !line.is_empty() => line.clone(),
            _ => self.authors.join(", "),
        }
    }

    /// Where the licence is read: the package's own file, else the first SPDX page.
    pub fn license_link(&self) -> Option<&str> {
        self.license_file_url
            .as_deref()
            .or_else(|| self.license_urls.first().map(String::as_str))
    }

    /// Where the package lives: its repository, else its homepage.
    pub fn project_link(&self) -> Option<&str> {
        self.repository.as_deref().or(self.homepage.as_deref())
    }
}

#[derive(Deserialize)]
struct Manifest {
    packages: Vec<ThirdParty>,
}

const MANIFEST: &[u8] = include_bytes!("../../../../assets/third-party.json");

/// Every third-party package in this build, sorted by name. Parsed once; a manifest that does not
/// parse lists nothing rather than taking the window down.
pub fn third_party() -> &'static [ThirdParty] {
    static PARSED: OnceLock<Vec<ThirdParty>> = OnceLock::new();
    PARSED.get_or_init(|| {
        serde_json::from_slice::<Manifest>(MANIFEST)
            .map(|manifest| manifest.packages)
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baked_manifest_parses_and_lists_packages() {
        let packages = third_party();
        assert!(!packages.is_empty());
        assert!(packages.iter().all(|p| !p.name.is_empty()));
    }
}
