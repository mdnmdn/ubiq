//! The one walk both content search and (later) the filename index run, so the two can never
//! disagree about what a project's files are.
//!
//! The project's own ignore rules (`.gitignore`, `.ignore`, hidden files — `ignore`'s defaults,
//! unchanged here) apply first; on top of them sits one [`ignore::overrides::Override`] carrying
//! the include globs and every exclude.

use std::cmp;
use std::path::Path;

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

    #[test]
    fn an_unparseable_glob_is_refused() {
        let dir = TempDir::new().unwrap();
        assert!(builder(dir.path(), dir.path(), &["[".to_string()], &[]).is_err());
    }
}
