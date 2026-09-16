//! A project's knowledge base: what its sources are, where each comes from, and how far a source
//! that has to be fetched has got.
//!
//! The knowledge base is the documents half of the IDE — markdown, diagrams and images rather
//! than code — and its explorer differs from the project explorer in one structural way: it has
//! **several sources**, and none of them need be inside the project. A source is a folder the
//! user pointed at, a repository the host clones on their behalf, or a wiki Ubiq keeps itself —
//! and the project's own folder is just the common first case rather than a special one.
//!
//! **Nothing here holds material.** A [`KbOrigin::Git`] names a URL; whatever credential reaches
//! it is the host's business, on the rule [`crate::connectors`] states.
//!
//! Paths in this family are relative *to their source*, never to the project — which is why every
//! message in the family carries a [`crate::ids::KbSourceId`] beside the path. The host resolves
//! the pair, through the same containment check the file family uses, and the interface learns
//! no absolute path from a listing.

use serde::{Deserialize, Serialize};

use crate::ids::KbSourceId;

/// Where a source's documents come from.
///
/// A folder is already on this machine and is read where it lies; a repository is fetched and
/// read from wherever its [`KbStore`] puts it; a wiki is Ubiq's own, with nothing to fetch and
/// nowhere to point. An Azure DevOps wiki, a Confluence space and the rest are further arms of
/// this enum when they come, not a second family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KbOrigin {
    /// A folder on the machine the host runs on, absolute. The one path the interface holds in
    /// this family, and it holds it for the reason it holds a project's own root: the user chose
    /// it in a folder picker and the settings row has to print it back.
    Folder { path: String },
    /// A repository the host clones and refreshes. `branch` absent is the repository's default.
    Git {
        url: String,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        store: KbStore,
    },
    /// A wiki Ubiq itself keeps, under the project's own directory in the config root. Nothing
    /// to fetch and nowhere to point: it starts empty and is written through Ubiq.
    Internal,
}

impl KbOrigin {
    /// What the row reads as before the user renames it.
    pub fn default_name(&self) -> String {
        match self {
            KbOrigin::Folder { path } => path
                .rsplit(['/', '\\'])
                .find(|part| !part.is_empty())
                .unwrap_or(path)
                .to_string(),
            KbOrigin::Git { url, .. } => url
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .rsplit(['/', ':'])
                .find(|part| !part.is_empty())
                .unwrap_or(url)
                .to_string(),
            KbOrigin::Internal => "Wiki".to_string(),
        }
    }

    /// Whether the host has to fetch this source before it can be listed.
    pub fn is_fetched(&self) -> bool {
        matches!(self, KbOrigin::Git { .. })
    }

    /// Whether this source is the wiki Ubiq keeps itself, rather than something it read or
    /// fetched from elsewhere.
    pub fn is_internal(&self) -> bool {
        matches!(self, KbOrigin::Internal)
    }
}

/// Where a fetched source's working copy is kept.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KbStore {
    /// The machine's temporary area — cheap, and gone when the OS sweeps it.
    Cache,
    /// Under the project's own directory in Ubiq's config root. The default, and the one
    /// place `D30` guarantees is never inside the user's folder.
    #[default]
    Internal,
    /// A `.ubiq/kb` folder inside the project itself, for a team that wants the checkout
    /// committed or shared. The one arm that writes inside a user's project.
    Project,
}

/// Whether a source may be written through Ubiq.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KbAccess {
    #[default]
    ReadOnly,
    ReadWrite,
}

impl KbAccess {
    pub fn is_writable(&self) -> bool {
        matches!(self, KbAccess::ReadWrite)
    }
}

/// One source of a knowledge base, as the user configured it.
///
/// The whole list rides [`crate::messages::Message::SetKbSources`] at once rather than one row
/// per message: it is a short list edited in a settings form, and a whole-list write is what
/// makes reordering and removal a single fact instead of three.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KbSource {
    pub id: KbSourceId,
    /// What the explorer's top-level row says. Seeded from [`KbOrigin::default_name`].
    pub name: String,
    pub origin: KbOrigin,
    /// Which files the source shows, as space-separated globs — `*.md *.excalidraw`. Empty shows
    /// everything. Matched against the file's name only, never the path, so a pattern means the
    /// same thing at every depth. Folders are never filtered out by it: one holding nothing that
    /// matches is simply empty when it is opened.
    #[serde(default)]
    pub filter: String,
    /// Whether Ubiq may write to this source. Ignored for [`KbOrigin::Internal`], which is
    /// always read-write — there is no fetched or user-picked material to protect from it.
    #[serde(default)]
    pub access: KbAccess,
}

impl KbSource {
    /// Whether a file of this name belongs in the source's listing.
    ///
    /// The one place the filter is interpreted, so the host's walk and the interface's tree agree
    /// by construction rather than by two implementations staying in step.
    pub fn admits(&self, name: &str) -> bool {
        let mut patterns = self.filter.split_whitespace().peekable();
        if patterns.peek().is_none() {
            return true;
        }
        patterns.any(|pattern| glob_match(pattern, name))
    }

    /// Whether Ubiq may write to this source. An [`KbOrigin::Internal`] wiki is always
    /// writable, whatever `access` was last saved as — there is nothing fetched or user-picked
    /// for read-only to protect.
    pub fn is_writable(&self) -> bool {
        self.origin.is_internal() || self.access.is_writable()
    }
}

/// How far a source is from being readable.
///
/// A folder is [`KbSourceState::Ready`] the moment it is configured; only a fetched source has
/// anything in between. Typed rather than a percentage for the reason [`crate::repos::CloneStage`]
/// is: "cloning" and "failed" need different sentences, and one of them names something the user
/// can fix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KbSourceState {
    /// Configured and not fetched yet. What a git source reads as until the first sync finishes.
    Pending,
    /// Being cloned or refreshed. `detail` is a phase the user can read, not a percentage.
    Syncing {
        detail: String,
    },
    Ready,
    /// The last attempt to fetch or read it failed, with the sentence to print.
    Failed {
        error: String,
    },
}

impl KbSourceState {
    pub fn is_ready(&self) -> bool {
        matches!(self, KbSourceState::Ready)
    }
}

/// One source plus what the host knows about it now. What a listing of the configuration answers
/// with, so a settings row and an explorer row are drawn from the same record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KbSourceStatus {
    pub source: KbSource,
    pub state: KbSourceState,
}

/// A very small glob: `*` matches any run, `?` matches one character, everything else is literal,
/// and the comparison ignores case because a filter is typed by a person and `*.MD` is the same
/// wish as `*.md`.
///
/// Spelled out rather than taken from a crate because this is the whole of what a filter field
/// needs, and the host and the interface both have to agree on it — a dependency that only one
/// half could take would be two answers.
fn glob_match(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.to_lowercase().chars().collect();
    let name: Vec<char> = name.to_lowercase().chars().collect();
    // Iterative backtracking rather than recursion: a pathological pattern must not be a stack
    // overflow in the host's file worker.
    let (mut p, mut n) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some(p);
                mark = n;
                p += 1;
            }
            Some('?') => {
                p += 1;
                n += 1;
            }
            Some(ch) if *ch == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some(at) => {
                    p = at + 1;
                    mark += 1;
                    n = mark;
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|ch| *ch == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(filter: &str) -> KbSource {
        KbSource {
            id: KbSourceId::generate(),
            name: "docs".into(),
            origin: KbOrigin::Folder {
                path: "/docs".into(),
            },
            filter: filter.into(),
            access: KbAccess::ReadOnly,
        }
    }

    #[test]
    fn an_empty_filter_admits_everything() {
        assert!(source("").admits("notes.md"));
        assert!(source("   ").admits("photo.png"));
    }

    #[test]
    fn a_filter_admits_only_what_it_names() {
        let source = source("*.md *.excalidraw");
        assert!(source.admits("notes.md"));
        assert!(source.admits("NOTES.MD"));
        assert!(source.admits("board.excalidraw"));
        assert!(!source.admits("main.rs"));
        assert!(!source.admits("md"));
    }

    #[test]
    fn the_glob_handles_runs_and_single_characters() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("a*b*c", "azzbzzc"));
        assert!(!glob_match("a*b*c", "azzbzz"));
        assert!(glob_match("d?.md", "d1.md"));
        assert!(!glob_match("d?.md", "d12.md"));
    }

    #[test]
    fn a_name_is_taken_from_the_tail_of_a_source() {
        let folder = KbOrigin::Folder {
            path: "/home/u/docs/".into(),
        };
        assert_eq!(folder.default_name(), "docs");
        let git = KbOrigin::Git {
            url: "https://example.com/org/wiki.git".into(),
            branch: None,
            store: KbStore::default(),
        };
        assert_eq!(git.default_name(), "wiki");
    }

    #[test]
    fn an_internal_source_s_default_name_is_wiki() {
        assert_eq!(KbOrigin::Internal.default_name(), "Wiki");
        assert!(!KbOrigin::Internal.is_fetched());
        assert!(KbOrigin::Internal.is_internal());
    }

    #[test]
    fn writability_follows_access_except_for_an_internal_source() {
        let mut folder = source("");
        folder.access = KbAccess::ReadOnly;
        assert!(!folder.is_writable());
        folder.access = KbAccess::ReadWrite;
        assert!(folder.is_writable());

        // An internal wiki is always writable, whatever `access` says — the field is only
        // meaningful for a source with fetched or user-picked material to protect.
        let mut wiki = source("");
        wiki.origin = KbOrigin::Internal;
        wiki.access = KbAccess::ReadOnly;
        assert!(wiki.is_writable());
    }

    #[test]
    fn a_kb_store_defaults_to_internal_when_absent() {
        let expected = KbOrigin::Git {
            url: "https://example.com/org/wiki.git".into(),
            branch: None,
            store: KbStore::Internal,
        };

        let toml: KbOrigin =
            toml::from_str("[git]\nurl = \"https://example.com/org/wiki.git\"\n").unwrap();
        assert_eq!(toml, expected);

        let json: KbOrigin =
            serde_json::from_str(r#"{"git":{"url":"https://example.com/org/wiki.git"}}"#).unwrap();
        assert_eq!(json, expected);
    }
}
