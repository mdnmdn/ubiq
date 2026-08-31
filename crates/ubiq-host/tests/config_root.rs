//! Where the config root comes from, and in what order.
//!
//! Every case goes through `resolve(flag, env, cwd)`, which takes the environment as an argument.
//! Nothing here mutates the process's environment, so the cases are independent and the file can
//! run in parallel with everything else.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use ubiq_host::config::{ConfigRoot, RootSource, resolve};

/// A directory that looks like a repository checkout: a `.git` marker, and a nested working
/// directory to resolve from.
fn checkout(bootstrap: Option<&str>) -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::create_dir_all(dir.path().join("crates/ubiq/src")).unwrap();
    if let Some(body) = bootstrap {
        fs::write(dir.path().join("ubiq.toml"), body).unwrap();
    }
    dir
}

fn at(dir: &TempDir, rel: &str) -> PathBuf {
    dir.path().join(rel)
}

fn root(result: ConfigRoot) -> PathBuf {
    result.path
}

#[test]
fn the_flag_beats_everything() {
    let dir = checkout(Some("config_root = \"_data/config\"\n"));
    let flag = at(&dir, "from-the-flag");
    let env = at(&dir, "from-the-env");

    let resolved = resolve(Some(&flag), Some(&env), &at(&dir, "crates/ubiq/src")).unwrap();

    assert_eq!(resolved.source, RootSource::Flag);
    assert_eq!(root(resolved), flag);
}

#[test]
fn the_environment_beats_a_bootstrap() {
    let dir = checkout(Some("config_root = \"_data/config\"\n"));
    let env = at(&dir, "from-the-env");

    let resolved = resolve(None, Some(&env), &at(&dir, "crates/ubiq/src")).unwrap();

    assert_eq!(resolved.source, RootSource::Env);
    assert_eq!(root(resolved), env);
}

#[test]
fn an_empty_environment_variable_is_not_an_answer() {
    let dir = checkout(Some("config_root = \"_data/config\"\n"));

    let resolved = resolve(None, Some(Path::new("")), &at(&dir, "crates/ubiq/src")).unwrap();

    assert!(matches!(resolved.source, RootSource::Bootstrap(_)));
}

#[test]
fn a_bootstrap_is_found_by_walking_up_and_resolves_against_its_own_directory() {
    let dir = checkout(Some("config_root = \"_data/config\"\n"));

    // Resolved from deep inside the tree, not from the root that holds the file.
    let resolved = resolve(None, None, &at(&dir, "crates/ubiq/src")).unwrap();

    assert_eq!(
        resolved.source,
        RootSource::Bootstrap(at(&dir, "ubiq.toml"))
    );
    // Relative to the bootstrap, so a checked-in path means the same in every clone.
    assert_eq!(root(resolved), at(&dir, "_data/config"));
}

#[test]
fn a_bootstrap_may_name_an_absolute_path() {
    let elsewhere = TempDir::new().unwrap();
    let body = format!("config_root = {:?}\n", elsewhere.path().to_str().unwrap());
    let dir = checkout(Some(&body));

    let resolved = resolve(None, None, dir.path()).unwrap();

    assert_eq!(root(resolved), elsewhere.path());
}

#[test]
fn the_walk_up_stops_at_the_repository_it_started_in() {
    // An outer checkout with a bootstrap, and an inner one without. The inner `.git` stops the
    // ascent, so the outer file must not be picked up.
    let outer = checkout(Some("config_root = \"outer\"\n"));
    let inner = outer.path().join("vendor/inner");
    fs::create_dir_all(inner.join(".git")).unwrap();
    fs::create_dir_all(inner.join("src")).unwrap();

    let resolved = resolve(None, None, &inner.join("src")).unwrap();

    assert_eq!(
        resolved.source,
        RootSource::Default,
        "a bootstrap outside the nearest repository is not this run's"
    );
}

#[test]
fn with_nothing_to_go_on_it_is_the_default() {
    let dir = checkout(None);

    let resolved = resolve(None, None, dir.path()).unwrap();

    assert_eq!(resolved.source, RootSource::Default);
    assert!(resolved.is_default());
    assert!(root(resolved).ends_with(".config/ubiq"));
}

#[test]
fn a_malformed_bootstrap_is_refused_rather_than_ignored() {
    let dir = checkout(Some("config_root = this is not toml\n"));

    let error = resolve(None, None, dir.path()).unwrap_err();

    // Falling back to the user's real config directory from a broken bootstrap is the trap this
    // whole mechanism exists to avoid, so it must not be a warning.
    assert!(
        error.to_string().contains("ubiq.toml"),
        "the error should name the file: {error}"
    );
}

#[test]
fn a_bootstrap_naming_no_config_root_is_refused() {
    let dir = checkout(Some("# says nothing\n"));

    let error = resolve(None, None, dir.path()).unwrap_err();

    assert!(
        error.to_string().contains("config_root"),
        "the error should say what is missing: {error}"
    );
}

#[test]
fn a_relative_flag_resolves_against_the_working_directory() {
    let dir = checkout(None);

    let resolved = resolve(Some(Path::new("scratch")), None, dir.path()).unwrap();

    assert_eq!(root(resolved), dir.path().join("scratch"));
}

#[test]
fn this_repository_resolves_to_its_own_data_directory() {
    // The real `ubiq.toml` at the root of this checkout, read the way a development run reads it.
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let resolved = resolve(None, None, here).unwrap();

    assert!(
        matches!(resolved.source, RootSource::Bootstrap(_)),
        "a run from this checkout must find the committed bootstrap, got {:?}",
        resolved.source
    );
    assert!(
        resolved.path.ends_with("_data/config"),
        "and land in the ignored directory, not the user's own: {}",
        resolved.path.display()
    );
    assert!(!resolved.is_default());
}
