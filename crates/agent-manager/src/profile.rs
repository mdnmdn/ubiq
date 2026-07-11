//! Profiles: a named, persistent base from which every run makes a throwaway
//! overlay.
//!
//! A **profile** ties together an [`account`](crate::account) (login), a set of
//! composition [`defaults`](ProfileDefaults) (mcps/skills/model/hooks/
//! instructions), an optional harness pin, and a default isolation policy. A run
//! resolves a profile, materializes an ephemeral overlay (seed the account's
//! captured login, layer composition on top), and throws the overlay away —
//! leaving the persistent base untouched. See `_docs/target/profiles.md` for the
//! full model.
//!
//! Profiles form an **inheritance chain** via the [`extends`](Profile::extends)
//! field: a leaf profile inherits every field a parent sets and overrides only
//! what it mentions itself ([`flatten`], replace-by-default — the same
//! "highest layer that mentions a key wins" rule used by [`crate::resolve`] and
//! [`crate::settings`]).
//!
//! This mirrors the shape of [`crate::account`]: a trait ([`ProfileStore`]) so
//! embedders can back it with whatever they like, and a filesystem-backed
//! implementation ([`FsProfileStore`]) for the CLI. Unlike accounts (flat
//! `<id>.toml` files), profiles are **directories** — `<root>/<name>/profile.toml`
//! plus a per-harness `base/` identity seed — because a profile owns persistent
//! state, not just a single reference record.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Result;

/// Maximum depth of an `extends` inheritance chain before giving up (guards
/// against pathological or accidentally-deep hierarchies as well as cycles).
const MAX_EXTENDS_DEPTH: usize = 16;

/// A named, persistent base for a family of runs.
///
/// Holds only *references and defaults*, never secret material: the account
/// field names an entry in the [`account`](crate::account) store (which itself
/// holds only references), and the composition defaults name catalog ids. The
/// captured login lives on disk under `<root>/<id>/base/<harness>/`, not in this
/// record.
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Profile {
    /// Stable profile identifier (the store key). Defaults to the directory
    /// name when `profile.toml` omits it (mirrors [`crate::account::Account::id`]).
    #[serde(default)]
    pub id: String,
    /// Optional parent profile name. When set, this profile inherits every field
    /// the parent (transitively) sets and overrides only what it mentions
    /// itself. Resolved by [`resolve_chain`] / [`flatten`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    /// Id of the [`account`](crate::account) this profile logs in as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// Optional harness pin (e.g. `"claude"`). `None` means the profile is
    /// harness-agnostic and the harness is chosen per run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Composition defaults carried by the profile (the `[defaults]` sub-table).
    #[serde(default, skip_serializing_if = "ProfileDefaults::is_empty")]
    pub defaults: ProfileDefaults,
    /// Default isolation policy for runs of this profile (`isolate = false` or
    /// `isolate = "<policy>"`). `None` means the profile doesn't mention the
    /// isolation axis; a lower-precedence layer or per-run `--isolate` decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolate: Option<ProfileIsolate>,
}

/// The `[defaults]` sub-table of a profile: the composition a run overlays.
///
/// Each field is `Option<_>` rather than a bare value: `None` means "this
/// profile didn't mention the key" (inherit from the parent / a lower layer),
/// which is the distinction [`flatten`] relies on for replace-by-default merge.
/// This mirrors [`crate::settings::HarnessDefaults`].
#[derive(Debug, Clone, Default, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProfileDefaults {
    /// Catalog MCP ids. `None` = "not mentioned by this profile".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcps: Option<Vec<String>>,
    /// Catalog skill ids. Same None-vs-empty distinction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// Default model id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Catalog hook ids to select. Same None-vs-empty distinction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<Vec<String>>,
    /// Path to an instructions file layered into the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<PathBuf>,
}

impl ProfileDefaults {
    /// True when no field is set — used to skip an empty `[defaults]` table on
    /// serialize.
    pub fn is_empty(&self) -> bool {
        self.mcps.is_none()
            && self.skills.is_none()
            && self.model.is_none()
            && self.hooks.is_none()
            && self.instructions.is_none()
    }

    /// Overlay `other` onto `self` in place: for each field, a `Some` value in
    /// `other` (the higher-precedence layer) wins; a `None` leaves `self`'s
    /// value intact. This is the per-field replace-by-default rule.
    fn overlay(&mut self, other: &ProfileDefaults) {
        if other.mcps.is_some() {
            self.mcps = other.mcps.clone();
        }
        if other.skills.is_some() {
            self.skills = other.skills.clone();
        }
        if other.model.is_some() {
            self.model = other.model.clone();
        }
        if other.hooks.is_some() {
            self.hooks = other.hooks.clone();
        }
        if other.instructions.is_some() {
            self.instructions = other.instructions.clone();
        }
    }
}

/// A profile's default position on the isolation axis.
///
/// Deserializes from TOML `isolate = false` (→ [`ProfileIsolate::Off`]) or a
/// string naming an isol8 policy (→ [`ProfileIsolate::Sandboxed`]). Kept
/// self-contained — [`crate::resolve`] maps it onto the run's isolation model
/// later, so this type has no dependency on `crate::spec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileIsolate {
    /// No isolation (`isolate = false`).
    Off,
    /// Run inside an isol8 sandbox governed by the named policy
    /// (`isolate = "<policy>"`).
    Sandboxed(String),
}

impl Serialize for ProfileIsolate {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            ProfileIsolate::Off => serializer.serialize_bool(false),
            ProfileIsolate::Sandboxed(name) => serializer.serialize_str(name),
        }
    }
}

impl<'de> Deserialize<'de> for ProfileIsolate {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IsolateVisitor;

        impl<'de> Visitor<'de> for IsolateVisitor {
            type Value = ProfileIsolate;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("`false` or a policy-name string")
            }

            fn visit_bool<E>(self, value: bool) -> std::result::Result<ProfileIsolate, E>
            where
                E: de::Error,
            {
                if value {
                    Err(de::Error::custom(
                        "isolate = true is not supported; use `false` or a policy name string",
                    ))
                } else {
                    Ok(ProfileIsolate::Off)
                }
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<ProfileIsolate, E>
            where
                E: de::Error,
            {
                Ok(ProfileIsolate::Sandboxed(value.to_string()))
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<ProfileIsolate, E>
            where
                E: de::Error,
            {
                Ok(ProfileIsolate::Sandboxed(value))
            }
        }

        deserializer.deserialize_any(IsolateVisitor)
    }
}

/// A source of [`Profile`]s, resolved by id.
pub trait ProfileStore {
    /// All profiles, sorted by id.
    fn profiles(&self) -> Result<Vec<Profile>>;
    /// One profile by exact id.
    fn profile(&self, id: &str) -> Result<Option<Profile>> {
        Ok(self.profiles()?.into_iter().find(|p| p.id == id))
    }
    /// The config-overlay base dir for `id` + `harness`
    /// (`<root>/<id>/base/<harness>`) if this store is filesystem-backed;
    /// `None` for stores with no on-disk base. Feeds
    /// [`crate::spec::RunSpec::config_bases`] so provision can materialize the
    /// overlay across a profile's `extends` chain.
    fn overlay_base(&self, _id: &str, _harness: &str) -> Option<PathBuf> {
        None
    }
}

/// A [`ProfileStore`] with no profiles — the default for lib-mode embedders and
/// for the CLI when no profiles root is configured.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyProfileStore;

impl ProfileStore for EmptyProfileStore {
    fn profiles(&self) -> Result<Vec<Profile>> {
        Ok(Vec::new())
    }
}

/// A filesystem-backed profile store rooted at a profiles directory.
///
/// Each profile is a **subdirectory**: `<root>/<name>/profile.toml` (the `id`
/// field defaults to `<name>` if absent) plus a per-harness `base/<harness>/`
/// identity seed. Subdirectories without a `profile.toml` are skipped. Two
/// profiles resolving to the same id (e.g. a `profile.toml` whose explicit `id`
/// clashes with another dir) is a load-time error, mirroring
/// [`crate::account::FsAccountStore`]'s collision rule.
#[derive(Debug, Clone)]
pub struct FsProfileStore {
    root: PathBuf,
}

impl FsProfileStore {
    /// Create a store rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        FsProfileStore { root: root.into() }
    }

    /// Persist `profile` as `<root>/<id>/profile.toml` (creating the directory).
    /// Overwrites an existing entry. Holds only references/defaults — never a
    /// secret value.
    pub fn save(&self, profile: &Profile) -> Result<PathBuf> {
        if profile.id.is_empty() {
            bail!("cannot save a profile with an empty id");
        }
        let dir = self.root.join(&profile.id);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join("profile.toml");
        let body = toml::to_string_pretty(profile)
            .with_context(|| format!("serializing profile '{}'", profile.id))?;
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    /// Path of the config-overlay seed dir for a profile + harness:
    /// `<root>/<id>/base/<harness>`. This is where a captured login is
    /// persisted and seeded from; this method only computes the path (it does
    /// not create or populate it).
    pub fn base_dir(&self, id: &str, harness: &str) -> PathBuf {
        self.root.join(id).join("base").join(harness)
    }
}

impl ProfileStore for FsProfileStore {
    fn profiles(&self) -> Result<Vec<Profile>> {
        let mut entries = Vec::new();
        let mut seen_ids: BTreeSet<String> = BTreeSet::new();

        if !self.root.is_dir() {
            return Ok(entries);
        }

        for entry in std::fs::read_dir(&self.root)
            .with_context(|| format!("reading directory {}", self.root.display()))?
        {
            let entry = entry?;
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }

            let toml_path = dir.join("profile.toml");
            if !toml_path.is_file() {
                // Not a profile directory (e.g. stray dir); skip.
                continue;
            }

            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("invalid profile directory name: {}", dir.display()))?;

            let content = std::fs::read_to_string(&toml_path)
                .with_context(|| format!("reading {}", toml_path.display()))?;
            let mut profile: Profile = toml::from_str(&content)
                .with_context(|| format!("parsing {}", toml_path.display()))?;
            if profile.id.is_empty() {
                profile.id = name;
            }

            if !seen_ids.insert(profile.id.clone()) {
                bail!(
                    "profile id collision: '{}' resolves from more than one directory under {}",
                    profile.id,
                    self.root.display()
                );
            }
            entries.push(profile);
        }

        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(entries)
    }

    fn overlay_base(&self, id: &str, harness: &str) -> Option<PathBuf> {
        Some(self.base_dir(id, harness))
    }
}

/// The default profiles root: `~/.config/agent-manager/profiles` — the same
/// base dir as `config.toml` and `accounts/`
/// ([`crate::settings::default_config_dir`]). Overridable by `AM_PROFILES` (see
/// [`resolve_profiles_root`]).
pub fn default_profiles_root() -> Option<PathBuf> {
    crate::settings::default_config_dir().map(|d| d.join("profiles"))
}

/// Resolve the profiles root from (highest first): an explicit path, the
/// `AM_PROFILES` env var, then the default. Returns `None` if none apply.
pub fn resolve_profiles_root(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit
        .or_else(|| std::env::var("AM_PROFILES").ok().map(PathBuf::from))
        .or_else(default_profiles_root)
}

/// Walk the `extends` chain from `name` up to its root, returning the profiles
/// ordered **root → leaf** (so a later fold applies the most-specific override
/// last).
///
/// Errors if `name` or any named parent is missing, if a cycle is detected (the
/// message names the cycle), or if the chain exceeds [`MAX_EXTENDS_DEPTH`].
pub fn resolve_chain(store: &dyn ProfileStore, name: &str) -> Result<Vec<Profile>> {
    let mut chain: Vec<Profile> = Vec::new();
    let mut visited: Vec<String> = Vec::new();
    let mut current = name.to_string();

    loop {
        if visited.len() >= MAX_EXTENDS_DEPTH {
            bail!(
                "profile inheritance chain for '{}' exceeds the maximum depth of {} \
                 (chain so far: {})",
                name,
                MAX_EXTENDS_DEPTH,
                visited.join(" -> ")
            );
        }

        if let Some(pos) = visited.iter().position(|v| v == &current) {
            let mut cycle = visited[pos..].to_vec();
            cycle.push(current.clone());
            bail!(
                "profile inheritance cycle detected: {}",
                cycle.join(" -> ")
            );
        }

        let profile = store
            .profile(&current)?
            .ok_or_else(|| anyhow!("profile '{}' not found", current))?;

        visited.push(current.clone());

        let parent = profile.extends.clone();
        chain.push(profile);

        match parent {
            Some(next) => current = next,
            None => break,
        }
    }

    chain.reverse();
    Ok(chain)
}

/// Fold an inheritance chain (root → leaf, as produced by [`resolve_chain`])
/// into a single effective profile with **replace-by-default** semantics: for
/// each scalar field and each [`ProfileDefaults`] field, the leaf (later) value
/// wins when it is `Some`, otherwise the parent's value is inherited. The
/// result's `id` and `extends` are the leaf's.
pub fn flatten(chain: &[Profile]) -> Profile {
    let mut acc = Profile::default();

    for profile in chain {
        if profile.account.is_some() {
            acc.account = profile.account.clone();
        }
        if profile.harness.is_some() {
            acc.harness = profile.harness.clone();
        }
        if profile.isolate.is_some() {
            acc.isolate = profile.isolate.clone();
        }
        acc.defaults.overlay(&profile.defaults);
    }

    if let Some(leaf) = chain.last() {
        acc.id = leaf.id.clone();
        acc.extends = leaf.extends.clone();
    }

    acc
}

/// Convenience: resolve the chain for `name` and fold it into a single
/// effective profile.
pub fn resolve_flattened(store: &dyn ProfileStore, name: &str) -> Result<Profile> {
    let chain = resolve_chain(store, name)?;
    Ok(flatten(&chain))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Write `<root>/<name>/profile.toml` with `body`.
    fn write_profile(root: &std::path::Path, name: &str, body: &str) {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("profile.toml"), body).unwrap();
    }

    #[test]
    fn fs_profile_store_parses_subdirs_and_defaults_id_to_dir_name() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        write_profile(
            root,
            "work",
            r#"
account = "work-acct"
harness = "claude"
isolate = false

[defaults]
mcps = ["github", "postgres"]
skills = []
model = "haiku"
instructions = "/etc/work-instructions.md"
"#,
        );
        // id omitted -> defaults to dir name "personal".
        write_profile(root, "personal", "account = \"me\"\n");
        // A stray dir with no profile.toml is skipped.
        fs::create_dir_all(root.join("not-a-profile"))?;

        let store = FsProfileStore::new(root);
        let profiles = store.profiles()?;

        let ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["personal", "work"]);

        let work = store.profile("work")?.expect("work profile");
        assert_eq!(work.id, "work");
        assert_eq!(work.account.as_deref(), Some("work-acct"));
        assert_eq!(work.harness.as_deref(), Some("claude"));
        assert_eq!(work.isolate, Some(ProfileIsolate::Off));
        assert_eq!(
            work.defaults.mcps,
            Some(vec!["github".to_string(), "postgres".to_string()])
        );
        assert_eq!(work.defaults.skills, Some(vec![]));
        assert_eq!(work.defaults.model.as_deref(), Some("haiku"));
        assert_eq!(
            work.defaults.instructions,
            Some(PathBuf::from("/etc/work-instructions.md"))
        );

        let personal = store.profile("personal")?.expect("personal profile");
        assert_eq!(personal.id, "personal");
        assert!(personal.harness.is_none());
        assert!(personal.isolate.is_none());
        assert!(personal.defaults.is_empty());

        temp.close()?;
        Ok(())
    }

    #[test]
    fn fs_profile_store_isolate_string_is_sandboxed() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        write_profile(root, "ci", "isolate = \"locked.toml\"\n");

        let store = FsProfileStore::new(root);
        let ci = store.profile("ci")?.expect("ci profile");
        assert_eq!(
            ci.isolate,
            Some(ProfileIsolate::Sandboxed("locked.toml".to_string()))
        );
        temp.close()?;
        Ok(())
    }

    #[test]
    fn fs_profile_store_id_collision_is_an_error() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        // Two directories that both resolve to id "shared": one via explicit id.
        write_profile(root, "dir-a", "id = \"shared\"\n");
        write_profile(root, "shared", "account = \"x\"\n");

        let store = FsProfileStore::new(root);
        let err = store.profiles().expect_err("should error on collision");
        assert!(err.to_string().contains("collision"), "message was: {err}");
        temp.close()?;
        Ok(())
    }

    #[test]
    fn empty_profile_store_has_no_profiles() {
        let store = EmptyProfileStore;
        assert!(store.profiles().unwrap().is_empty());
        assert!(store.profile("anything").unwrap().is_none());
    }

    #[test]
    fn fs_profile_store_save_round_trips_and_base_dir() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path().join("profiles");

        let profile = Profile {
            id: "cap".to_string(),
            account: Some("cap-acct".to_string()),
            isolate: Some(ProfileIsolate::Sandboxed("dev.toml".to_string())),
            defaults: ProfileDefaults {
                mcps: Some(vec!["github".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };

        let store = FsProfileStore::new(&root);
        let path = store.save(&profile)?;
        assert!(path.exists());
        assert_eq!(path, root.join("cap").join("profile.toml"));

        let loaded = FsProfileStore::new(&root)
            .profile("cap")?
            .expect("saved profile should be found");
        assert_eq!(loaded, profile);

        assert_eq!(
            store.base_dir("cap", "claude"),
            root.join("cap").join("base").join("claude")
        );

        temp.close()?;
        Ok(())
    }

    #[test]
    fn resolve_profiles_root_honors_explicit_over_env_and_default() {
        // An explicit path always wins over the `AM_PROFILES` env var and the
        // default, regardless of the ambient environment. (This crate forbids
        // `unsafe`, so tests can't mutate `AM_PROFILES` to exercise the env
        // branch — `std::env::set_var` is `unsafe` as of edition 2024; the env
        // precedence itself is a straight `or_else` chain over `env::var`.)
        let explicit = PathBuf::from("/explicit/profiles");
        assert_eq!(
            resolve_profiles_root(Some(explicit.clone())),
            Some(explicit)
        );

        // With no explicit path, the result matches the env-or-default fallback
        // computed the same way `resolve_profiles_root` does internally.
        let expected_fallback = std::env::var("AM_PROFILES")
            .ok()
            .map(PathBuf::from)
            .or_else(default_profiles_root);
        assert_eq!(resolve_profiles_root(None), expected_fallback);

        // The default root sits alongside the config dir when one is resolvable.
        if let Some(root) = default_profiles_root() {
            assert!(root.ends_with("profiles"));
        }
    }

    #[test]
    fn flatten_leaf_overrides_parent_per_field_and_inherits_unset() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        write_profile(
            root,
            "base",
            r#"
account = "base-acct"
harness = "claude"
isolate = "base-policy"

[defaults]
mcps = ["github"]
skills = ["review"]
model = "sonnet"
"#,
        );
        // Leaf overrides account + defaults.mcps + isolate, inherits the rest.
        write_profile(
            root,
            "child",
            r#"
extends = "base"
account = "child-acct"
isolate = false

[defaults]
mcps = ["postgres"]
model = "haiku"
"#,
        );

        let store = FsProfileStore::new(root);
        let chain = resolve_chain(&store, "child")?;
        let ids: Vec<&str> = chain.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, vec!["base", "child"], "chain must be root -> leaf");

        let flat = flatten(&chain);
        assert_eq!(flat.id, "child");
        assert_eq!(flat.extends.as_deref(), Some("base"));
        // Overridden by leaf.
        assert_eq!(flat.account.as_deref(), Some("child-acct"));
        assert_eq!(flat.isolate, Some(ProfileIsolate::Off));
        assert_eq!(flat.defaults.mcps, Some(vec!["postgres".to_string()]));
        assert_eq!(flat.defaults.model.as_deref(), Some("haiku"));
        // Inherited from parent (leaf did not mention).
        assert_eq!(flat.harness.as_deref(), Some("claude"));
        assert_eq!(flat.defaults.skills, Some(vec!["review".to_string()]));

        // Convenience wrapper agrees.
        assert_eq!(resolve_flattened(&store, "child")?, flat);

        temp.close()?;
        Ok(())
    }

    #[test]
    fn resolve_chain_detects_cycles() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        write_profile(root, "a", "extends = \"b\"\n");
        write_profile(root, "b", "extends = \"a\"\n");

        let store = FsProfileStore::new(root);
        let err = resolve_chain(&store, "a").expect_err("should detect cycle");
        assert!(err.to_string().contains("cycle"), "message was: {err}");
        temp.close()?;
        Ok(())
    }

    #[test]
    fn resolve_chain_errors_on_missing_parent() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        write_profile(root, "leaf", "extends = \"ghost\"\n");

        let store = FsProfileStore::new(root);
        let err = resolve_chain(&store, "leaf").expect_err("should error on missing parent");
        assert!(err.to_string().contains("ghost"), "message was: {err}");
        assert!(err.to_string().contains("not found"), "message was: {err}");
        temp.close()?;
        Ok(())
    }

    #[test]
    fn resolve_chain_caps_depth() -> Result<()> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();

        // Build a linear chain deeper than MAX_EXTENDS_DEPTH: p0 -> p1 -> ... .
        let depth = MAX_EXTENDS_DEPTH + 5;
        for i in 0..depth {
            let body = format!("extends = \"p{}\"\n", i + 1);
            write_profile(root, &format!("p{i}"), &body);
        }

        let store = FsProfileStore::new(root);
        let err = resolve_chain(&store, "p0").expect_err("should hit depth cap");
        assert!(
            err.to_string().contains("maximum depth"),
            "message was: {err}"
        );
        temp.close()?;
        Ok(())
    }
}
