//! Which folders version control ignores, for the window's background cache walk (T-333).
//!
//! The cache walk is a convenience, so it obeys the ignore set ([`WALK_SKIP`]) and, through this,
//! the project's own `.gitignore`s: a `bin/`, `obj/` or `Library/` a build left behind is never
//! crawled just so a filter could match inside it. An explicit expand is answered in full whatever
//! the folder is — this is consulted only for a prefetch.
//!
//! The answer is libgit2's (`is_path_ignored`), so nested `.gitignore`s, `info/exclude` and the
//! global excludes all count. A folder holding a `.git` of its own is a repository root and is
//! never ignored — the base checkout inside a workspace that ignores it is still walked — and below
//! it that repository's own rules apply. With no repository, or no `git` feature, nothing is.
//!
//! [`WALK_SKIP`]: ubiq_proto::files::WALK_SKIP

#[cfg(not(feature = "git"))]
use std::path::Path;

#[cfg(feature = "git")]
pub(super) use with_git::Ignored;

#[cfg(not(feature = "git"))]
pub(super) struct Ignored;

#[cfg(not(feature = "git"))]
impl Ignored {
    pub(super) fn new(_root: &Path) -> Self {
        Ignored
    }

    pub(super) fn is_ignored(&mut self, _rel_path: &str) -> bool {
        false
    }
}

#[cfg(feature = "git")]
mod with_git {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};

    use git2::Repository;

    use crate::git::observe;

    /// One request's view of the ignore rules. Repositories are opened once per request, on first
    /// need, and which folders hold a `.git` is remembered for as long.
    pub(in crate::files) struct Ignored {
        root: PathBuf,
        /// The repository holding the project's root, and the project's prefix inside its work
        /// tree (`""` when they are the same folder, else ending in `/`).
        outer: Option<(Repository, String)>,
        /// Repositories nested in the project, by project-relative root.
        nested: HashMap<String, Option<Repository>>,
        /// Whether a project-relative folder holds a `.git`.
        roots: HashMap<String, bool>,
    }

    impl Ignored {
        pub(in crate::files) fn new(root: &Path) -> Self {
            let outer = observe::open(root).ok().flatten().and_then(|repo| {
                let prefix = observe::scope(root, &repo).ok()?;
                let prefix = if prefix.is_empty() {
                    prefix
                } else {
                    format!("{prefix}/")
                };
                Some((repo, prefix))
            });
            // An enclosing repository (a dotfiles repo in `~` ignoring `*`) that ignores the
            // project's own root says nothing useful about what is inside it: treat it as none.
            let outer = outer.filter(|(repo, prefix)| {
                prefix.is_empty()
                    || !repo
                        .is_path_ignored(prefix.trim_end_matches('/'))
                        .unwrap_or(false)
            });
            Self {
                root: root.to_path_buf(),
                outer,
                nested: HashMap::new(),
                roots: HashMap::new(),
            }
        }

        fn holds_git(&mut self, rel_path: &str) -> bool {
            if let Some(&held) = self.roots.get(rel_path) {
                return held;
            }
            let held = self.root.join(rel_path).join(".git").exists();
            self.roots.insert(rel_path.to_string(), held);
            held
        }

        /// Whether the project-relative folder `rel_path` is ignored by the repository that owns
        /// it. The project's root, and any repository root, never is.
        pub(in crate::files) fn is_ignored(&mut self, rel_path: &str) -> bool {
            if rel_path.is_empty() || self.holds_git(rel_path) {
                return false;
            }
            // The deepest proper ancestor that is a repository root owns the path.
            let mut owner: Option<String> = None;
            let mut at = 0;
            while let Some(slash) = rel_path[at..].find('/') {
                let prefix = &rel_path[..at + slash];
                if self.holds_git(prefix) {
                    owner = Some(prefix.to_string());
                }
                at += slash + 1;
            }
            match owner {
                Some(owner) => {
                    let inner = &rel_path[owner.len() + 1..];
                    let root = &self.root;
                    let repo = self
                        .nested
                        .entry(owner.clone())
                        .or_insert_with(|| Repository::open(root.join(&owner)).ok());
                    repo.as_ref()
                        .is_some_and(|repo| repo.is_path_ignored(inner).unwrap_or(false))
                }
                None => self.outer.as_ref().is_some_and(|(repo, prefix)| {
                    repo.is_path_ignored(format!("{prefix}{rel_path}"))
                        .unwrap_or(false)
                }),
            }
        }
    }
}
