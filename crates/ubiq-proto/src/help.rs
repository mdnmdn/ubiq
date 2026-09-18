//! Ubiq's own documentation: the catalogue of pages the help panel draws, and the keys that bind a
//! page to a place in the interface.
//!
//! Help content ships as one prebuilt bundle beside the binary, unpacked once by the host. Nothing
//! in this family carries a page's markdown — the catalogue is metadata, and a page is read from
//! the unpacked root the host names in [`crate::messages::Message::HelpReady`].
//!
//! **A page is addressed by its `id`, never by its path.** The id is written into the page's
//! frontmatter by whoever authored it and is stable for the life of the page; the path is where it
//! happens to live today. Links, the context map and the MCP tools all speak ids, so moving a page
//! between folders breaks nothing.
//!
//! **The context map is derived, not authored.** Each page declares the places it explains, and the
//! packer inverts those declarations into [`HelpCatalog::context`]. The interface computes a key
//! from values it already holds — a panel's name, a view, a rail mode — and looks it up.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Everything the interface knows about the help content, read from the bundle's `catalog.json`.
///
/// One of these per bundle root. A second root — Studio's, when there is one — is a second
/// catalogue with its own namespace, never a merge into this one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpCatalog {
    /// The content version, as the manifest states it. Also the directory the host unpacked into,
    /// which is how a new bundle lands beside an old one rather than over it.
    pub version: String,
    /// Which bundle these pages came from — `ubiq` today, `studio` later. Page ids are unique
    /// within a namespace and qualified across them.
    pub namespace: String,
    /// Every page, in no particular order. [`HelpCatalog::nav`] is what gives them one.
    pub pages: Vec<HelpPage>,
    /// Page ids in the order the tree draws them, folders already flattened by the packer.
    pub nav: Vec<String>,
    /// Context key → page id. Keys are the strings the interface already computes for a panel, a
    /// view or a rail mode; the packer refuses a key two pages both claim.
    pub context: BTreeMap<String, String>,
    /// Paths and ids a page used to answer to → the page id now. What keeps a link someone pasted
    /// last month working after the page moved.
    #[serde(default)]
    pub redirects: BTreeMap<String, String>,
}

impl HelpCatalog {
    /// The page with this id, or the page a redirect sends this id or path to.
    pub fn page(&self, id: &str) -> Option<&HelpPage> {
        if let Some(page) = self.pages.iter().find(|page| page.id == id) {
            return Some(page);
        }
        let target = self.redirects.get(id)?;
        self.pages.iter().find(|page| &page.id == target)
    }

    /// The page bound to a context key, if one claimed it.
    pub fn for_context(&self, key: &str) -> Option<&HelpPage> {
        self.context.get(key).and_then(|id| self.page(id))
    }
}

/// One page's metadata. Its markdown is a file under the unpacked root, at [`HelpPage::path`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelpPage {
    /// Stable for the life of the page. The handle every link, tool and context binding uses.
    pub id: String,
    /// Relative to the bundle root, with its extension — `git/changes.md`. The one field that may
    /// change without breaking anything that points at the page.
    pub path: String,
    /// Shown in the tree, the panel header and the history. The page's body does not repeat it as
    /// an `# h1`.
    pub title: String,
    /// One sentence. The tree's tooltip, a search result's line, and the only thing an agent reads
    /// before deciding to open the page.
    #[serde(default)]
    pub summary: String,
    /// Search terms weighted above body text — the words a reader types that the prose never uses.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// How finished the page is. A [`HelpStatus::Planned`] page is listed and opens on a stub.
    #[serde(default)]
    pub status: HelpStatus,
    /// Ids drawn as a "see also" strip at the foot of the page.
    #[serde(default)]
    pub related: Vec<String>,
    /// Depth in the nav tree, so the panel can draw the tree from a flat list.
    #[serde(default)]
    pub depth: u8,
}

/// How finished a page is, as its frontmatter declares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelpStatus {
    /// Written and true of the running build.
    #[default]
    Current,
    /// Written, but known to be behind or incomplete. Drawn with a note above the body.
    Draft,
    /// Listed in the tree so the shape of the manual is honest, with nothing written yet.
    Planned,
}
