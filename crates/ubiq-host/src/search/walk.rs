//! The one walk both content search and (later) the filename index run, so the two can never
//! disagree about what a project's files are.
//!
//! The project's own ignore rules (`.gitignore`, `.ignore`, hidden files — `ignore`'s defaults,
//! unchanged here) apply first; on top of them sits one [`ignore::overrides::Override`] carrying
//! the include globs and every exclude.

use std::cmp;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Build the walk over `start` (the project root, or a validated subdirectory of it).
///
/// `patterns` are include globs against the project-relative path; an empty list means everything
/// the ignore rules allow. `excludes` are added as `Override` ignore lines (`!pattern`) on top.
///
/// Errors are the contract's ([`ubiq_proto::search::SearchError::BadFilter`]), because a glob that
/// will not compile is a refusal the user has to see rather than a silently wider search.
///
/// `Override`'s own rule — any whitelist glob means everything that does not match one is ignored
/// — is exactly the include semantics wanted here, and is exercised in the tests below rather than
/// assumed.
pub fn builder(
    root: &Path,
    start: &Path,
    patterns: &[String],
    excludes: &[String],
) -> Result<ignore::WalkBuilder, String> {
    let mut over = ignore::overrides::OverrideBuilder::new(root);
    for pattern in patterns {
        over.add(pattern).map_err(|error| error.to_string())?;
    }
    for exclude in excludes {
        over.add(&format!("!{exclude}"))
            .map_err(|error| error.to_string())?;
    }
    let over = over.build().map_err(|error| error.to_string())?;

    let mut builder = ignore::WalkBuilder::new(start);
    builder
        .threads(cmp::min(num_cpus::get(), 8))
        .overrides(over);
    Ok(builder)
}

/// How far below `start` the nested-repository discovery looks.
const NESTED_DEPTH: usize = 6;

/// Directories discovery never enters: they are huge and never hold a repository worth searching.
const DISCOVERY_SKIP: [&str; 3] = [".git", "node_modules", "target"];

/// Every walk a project's files need: the project's own, then one per repository nested inside it.
///
/// A nested repository — a folder with its own `.git` — is very often listed in the *outer*
/// project's `.gitignore` (that is how a checkout lives inside another), and `ignore` never enters a
/// directory its parent's rules exclude, so a single walk silently skips the whole repository. Each
/// nested repository is therefore walked on its own, with the outer repository's rules switched off
/// (`parents(false)`) so only its own `.gitignore` applies; the outer walk is told to leave those
/// directories alone so no file is visited twice.
pub fn walks(
    root: &Path,
    start: &Path,
    patterns: &[String],
    excludes: &[String],
) -> Result<Vec<ignore::WalkBuilder>, String> {
    let nested: Vec<PathBuf> = nested_repos(root, excludes)?
        .into_iter()
        .filter(|repo| repo != start && repo.starts_with(start))
        .collect();
    let mut main = builder(root, start, patterns, excludes)?;
    if nested.is_empty() {
        return Ok(vec![main]);
    }
    let all = Arc::new(nested.iter().cloned().collect::<HashSet<PathBuf>>());
    prune(&mut main, all.clone(), None);
    let mut walks = vec![main];
    for repo in nested {
        let mut one = builder(root, &repo, patterns, excludes)?;
        one.parents(false);
        prune(&mut one, all.clone(), Some(repo));
        walks.push(one);
    }
    Ok(walks)
}

/// Leave every nested repository root but `own` to its own walk.
fn prune(builder: &mut ignore::WalkBuilder, all: Arc<HashSet<PathBuf>>, own: Option<PathBuf>) {
    builder.filter_entry(move |entry| {
        entry.depth() == 0
            || !entry.file_type().is_some_and(|t| t.is_dir())
            || own.as_deref() == Some(entry.path())
            || !all.contains(entry.path())
    });
}

/// Discovery results, per project root and exclude set. A walk of the whole tree is too much to pay
/// on every search and every index build, so it is kept until [`invalidate`] — the watcher calls it
/// when repository plumbing moves, and a project's open does.
static CACHE: Mutex<Vec<((PathBuf, Vec<String>), Vec<PathBuf>)>> = Mutex::new(Vec::new());

/// Forget what is known about `root`'s nested repositories.
pub fn invalidate(root: &Path) {
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|((cached, _), _)| cached != root);
    }
}

/// The repositories nested in `root`, cached.
pub fn nested_repos(root: &Path, excludes: &[String]) -> Result<Vec<PathBuf>, String> {
    let key = (root.to_path_buf(), excludes.to_vec());
    if let Ok(cache) = CACHE.lock()
        && let Some((_, found)) = cache.iter().find(|(k, _)| *k == key)
    {
        return Ok(found.clone());
    }
    let found = discover(root, excludes)?;
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|(k, _)| *k != key);
        cache.push((key, found.clone()));
    }
    Ok(found)
}

/// Directories below `start` that hold their own `.git` (a directory, or the file a worktree or
/// submodule keeps). Found by a walk with every ignore rule off — the point is to see the folders
/// the project's rules hide — bounded in depth and by [`DISCOVERY_SKIP`]; the caller's excludes still
/// apply.
fn discover(root: &Path, excludes: &[String]) -> Result<Vec<PathBuf>, String> {
    let start = root;
    let mut over = ignore::overrides::OverrideBuilder::new(root);
    for exclude in excludes {
        over.add(&format!("!{exclude}"))
            .map_err(|error| error.to_string())?;
    }
    let over = over.build().map_err(|error| error.to_string())?;

    let found = Arc::new(Mutex::new(Vec::new()));
    let sink = found.clone();
    let origin = start.to_path_buf();
    let walk = ignore::WalkBuilder::new(start)
        .standard_filters(false)
        // Hidden directories are not entered, as in the main walk: `.claude/worktrees/*`,
        // `.venv/src/*` and `.terraform/modules/*` hold `.git` files that are not projects' repos.
        .hidden(true)
        .overrides(over)
        .max_depth(Some(NESTED_DEPTH))
        .filter_entry(move |entry| {
            if !entry.file_type().is_some_and(|t| t.is_dir()) {
                return false;
            }
            let name = entry.file_name().to_string_lossy();
            if DISCOVERY_SKIP.contains(&name.as_ref()) {
                return false;
            }
            if entry.path() != origin && entry.path().join(".git").exists() {
                sink.lock().unwrap().push(entry.path().to_path_buf());
            }
            true
        })
        .build();
    for _ in walk {}
    let mut found = std::mem::take(&mut *found.lock().unwrap());
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Every relative file path the walk visits, sorted for a stable comparison.
    fn walked(builder: ignore::WalkBuilder) -> Vec<String> {
        let mut seen: Vec<String> = builder
            .build()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
            .map(|entry| entry.path().to_string_lossy().into_owned())
            .collect();
        seen.sort();
        seen
    }

    #[test]
    fn an_include_pattern_is_a_whitelist_everything_else_is_ignored() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("keep.rs"), "").unwrap();
        fs::write(dir.path().join("skip.txt"), "").unwrap();

        let builder = builder(dir.path(), dir.path(), &["*.rs".to_string()], &[]).unwrap();
        let seen = walked(builder);

        assert_eq!(seen.len(), 1);
        assert!(seen[0].ends_with("keep.rs"));
    }

    #[test]
    fn no_patterns_means_everything_the_ignore_rules_allow() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "").unwrap();
        fs::write(dir.path().join("b.txt"), "").unwrap();

        let builder = builder(dir.path(), dir.path(), &[], &[]).unwrap();
        assert_eq!(walked(builder).len(), 2);
    }

    #[test]
    fn an_exclude_with_no_include_pattern_only_removes_what_it_names() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.rs"), "").unwrap();
        fs::write(dir.path().join("b.txt"), "").unwrap();

        let builder = builder(dir.path(), dir.path(), &[], &["*.txt".to_string()]).unwrap();
        let seen = walked(builder);

        assert_eq!(seen.len(), 1);
        assert!(seen[0].ends_with("a.rs"));
    }

    fn walked_all(walks: Vec<ignore::WalkBuilder>) -> Vec<String> {
        let mut seen: Vec<String> = walks.into_iter().flat_map(|b| walked(b)).collect();
        seen.sort();
        seen
    }

    #[test]
    fn a_nested_repo_the_outer_gitignore_hides_is_walked_by_its_own_rules() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".gitignore"), "nested/\n").unwrap();
        fs::write(root.join("outer.rs"), "").unwrap();
        // A nested repo the outer rules hide, with its own ignore file.
        fs::create_dir_all(root.join("nested/.git")).unwrap();
        fs::write(root.join("nested/.gitignore"), "skipped.txt\n").unwrap();
        fs::write(root.join("nested/inner.rs"), "").unwrap();
        fs::write(root.join("nested/skipped.txt"), "").unwrap();
        // A nested repo the outer rules do not hide must not be visited twice.
        fs::create_dir_all(root.join("open/.git")).unwrap();
        fs::write(root.join("open/seen.rs"), "").unwrap();

        let seen = walked_all(walks(root, root, &[], &[]).unwrap());
        let names: Vec<&str> = seen
            .iter()
            .map(|p| p.strip_prefix(root.to_str().unwrap()).unwrap())
            .collect();

        assert!(names.iter().any(|n| n.ends_with("outer.rs")));
        assert_eq!(names.iter().filter(|n| n.ends_with("inner.rs")).count(), 1);
        assert_eq!(names.iter().filter(|n| n.ends_with("seen.rs")).count(), 1);
        assert!(!names.iter().any(|n| n.ends_with("skipped.txt")));
    }

    #[test]
    fn hidden_directories_are_not_searched_for_repositories() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        // A git worktree keeps a `.git` *file*, and lives under a dot-dir.
        fs::create_dir_all(root.join(".claude/worktrees/x")).unwrap();
        fs::write(root.join(".claude/worktrees/x/.git"), "gitdir: elsewhere").unwrap();
        fs::create_dir_all(root.join("real/.git")).unwrap();

        let found = nested_repos(&root, &[]).unwrap();
        assert_eq!(found, vec![root.join("real")]);
    }

    #[test]
    fn an_unparseable_glob_is_refused() {
        let dir = TempDir::new().unwrap();
        assert!(builder(dir.path(), dir.path(), &["[".to_string()], &[]).is_err());
    }
}
