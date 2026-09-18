//! What the window knows about Ubiq's own documentation.
//!
//! Help is content the host unpacked and named a directory for — the same standing
//! [`crate::state::web_panel::BundleState`] holds a vendor bundle on: the path is a value that
//! arrived on a message, never one composed here. [`HelpState`] is that standing, and [`Help`]
//! is everything the panel keeps beside it.
//!
//! **Help history is the panel's own.** Reading three pages must not disturb where the window
//! thinks the reader is, so back and forward here walk a list of page ids with a cursor rather
//! than `crate::state::nav::History`.
//!
//! **A page's markdown is read lazily and cached by id.** Nothing here reads it — `app::help`
//! does, because a path is the application layer's to touch — but the cache lives here, and an
//! id whose file is missing caches [`None`] so the miss is answered once rather than every frame.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ubiq_proto::help::{HelpCatalog, HelpPage};

/// The page the catalogue lands on when nothing more specific was asked for. Also the last rung of
/// the context ladder — see [`crate::app::AppState::context_key`].
pub const INDEX_PAGE: &str = "index";

/// Where the documentation stands, as far as this window knows.
///
/// Every state but [`HelpState::Ready`] still draws a panel: help that was never built is a
/// downgrade, not a fault, and the built-in stub is what it shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum HelpState {
    /// Never asked for. The first open asks.
    #[default]
    Unknown,
    /// Unpacked at `root`, which the host named rather than the window composing it, with the
    /// catalogue it shipped.
    Ready {
        root: PathBuf,
        catalog: Box<HelpCatalog>,
    },
    /// No bundle was built, or the one found could not be read. A sentence fit to show a reader.
    Unavailable { reason: String },
}

impl HelpState {
    /// The catalogue, once there is one.
    pub fn catalog(&self) -> Option<&HelpCatalog> {
        match self {
            HelpState::Ready { catalog, .. } => Some(catalog),
            _ => None,
        }
    }

    /// The directory pages are read from, once there is one.
    pub fn root(&self) -> Option<&PathBuf> {
        match self {
            HelpState::Ready { root, .. } => Some(root),
            _ => None,
        }
    }
}

/// Everything the help panel keeps: where the content stands, which page is open, how the reader
/// got there, and the markdown already read.
#[derive(Default)]
pub struct Help {
    /// Where the content stands.
    pub state: HelpState,
    /// Whether `EnsureHelp` has been sent. The panel asks once, on its first open; the host's
    /// answer is idempotent but the ask is not worth repeating every frame.
    pub asked: bool,
    /// The page on screen, by id. `None` before the first open.
    pub page: Option<String>,
    /// Every page this panel has been on, oldest first, and the cursor into it. **The panel's
    /// own**, not the window's.
    pub history: Vec<String>,
    pub at: usize,
    /// Whether the contents tree is drawn beside the page.
    pub contents_open: bool,
    /// Nav branches the reader folded away, by the id of the page that heads them. Collapsed
    /// rather than expanded so a tree nobody has touched is open, which is what a manual wants.
    pub folded: HashSet<String>,
    /// Markdown already read, by page id. A [`None`] is a page the nav lists and no file answers
    /// — cached so the miss costs one look at the filesystem, not one per frame.
    pub sources: HashMap<String, Option<String>>,
}

impl Help {
    /// The page the panel is on, as the catalogue has it.
    pub fn current(&self) -> Option<&HelpPage> {
        let catalog = self.state.catalog()?;
        catalog.page(self.page.as_deref()?)
    }

    /// Arrive at a page. The same page again is not an arrival, so a link followed twice leaves
    /// one entry.
    pub fn visit(&mut self, id: &str) {
        if self.page.as_deref() == Some(id) {
            return;
        }
        self.history.truncate(self.at + 1);
        if self.history.last().map(String::as_str) != Some(id) {
            self.history.push(id.to_string());
        }
        self.at = self.history.len().saturating_sub(1);
        self.page = Some(id.to_string());
    }

    pub fn can_go(&self, back: bool) -> bool {
        if back {
            self.at > 0
        } else {
            self.at + 1 < self.history.len()
        }
    }

    /// Step the cursor one entry and answer the page it landed on.
    pub fn step(&mut self, back: bool) -> Option<String> {
        if !self.can_go(back) {
            return None;
        }
        self.at = if back { self.at - 1 } else { self.at + 1 };
        let id = self.history.get(self.at)?.clone();
        self.page = Some(id.clone());
        Some(id)
    }

    /// Fold or unfold one branch of the contents tree.
    pub fn toggle_fold(&mut self, id: &str) {
        if !self.folded.remove(id) {
            self.folded.insert(id.to_string());
        }
    }

    /// Whether a nav row is drawn, given the folds above it.
    ///
    /// A row is hidden while any ancestor is folded. The tree is a flat list plus a depth, so the
    /// ancestor is whichever earlier row is the last one shallower than this — walked here rather
    /// than materialised, because the nav is tens of rows and never thousands.
    pub fn nav_rows(&self) -> Vec<(String, u8, bool)> {
        let Some(catalog) = self.state.catalog() else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        // The depth at and below which everything is hidden, while a fold is in force.
        let mut hidden_below: Option<u8> = None;
        for id in &catalog.nav {
            let depth = catalog.page(id).map(|page| page.depth).unwrap_or(0);
            if let Some(limit) = hidden_below {
                if depth > limit {
                    continue;
                }
                hidden_below = None;
            }
            let has_children = self.has_children(id, depth);
            rows.push((id.clone(), depth, has_children));
            if has_children && self.folded.contains(id) {
                hidden_below = Some(depth);
            }
        }
        rows
    }

    /// Whether the row after this one in the nav is deeper — which is what "this page heads a
    /// branch" means in a flattened tree.
    fn has_children(&self, id: &str, depth: u8) -> bool {
        let Some(catalog) = self.state.catalog() else {
            return false;
        };
        let Some(at) = catalog.nav.iter().position(|nav| nav == id) else {
            return false;
        };
        catalog
            .nav
            .get(at + 1)
            .and_then(|next| catalog.page(next))
            .is_some_and(|next| next.depth > depth)
    }
}
