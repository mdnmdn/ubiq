//! Turns CLI flags + settings + catalog into a [`RunSpec`] — the resolve
//! stage, the single boundary before provision/run.
//!
//! Resolution has two concerns:
//!
//! 1. **Merging** — CLI flags, per-harness settings, and defaults settings
//!    combine into one effective set of mcp ids, skill ids, and account.
//!    The merge is **replace by default**: the highest-precedence layer that
//!    mentions a key wins outright, it does not union with lower layers. See
//!    [`pick`].
//! 2. **Lookup** — effective ids are resolved against a [`Registry`] into
//!    [`McpRef`]/[`SkillRef`] values; a missing id is a hard error listing
//!    near matches.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context};
use serde::Deserialize;

use crate::account::AccountStore;
use crate::config::McpServer;
use crate::profile::{Profile, ProfileIsolate, ProfileStore};
use crate::registry::{McpExpose, Registry};
use crate::settings::Settings;
use crate::spec::{
    HarnessId, HookRef, Instructions, Isolation, McpAsSkill, McpRef, RunSpec, SkillRef,
};
use crate::Result;

/// Raw run flags gathered from the CLI (the `cli` layer feeds this in).
///
/// `Option<Vec<_>>`: `None` = flag not given (fall to settings); `Some` =
/// replace (see the module-level merge semantics).
#[derive(Debug, Clone, Default)]
pub struct RunFlags {
    /// Harness to wrap (e.g. `claude-code`).
    pub harness: HarnessId,
    /// `--mcps a,b,c`, if given.
    pub mcps: Option<Vec<String>>,
    /// `--skills a,b`, if given.
    pub skills: Option<Vec<String>>,
    /// `--mcp-json <path>`, if given: additive inline MCP definitions.
    pub mcp_json: Option<PathBuf>,
    /// `--account <id>`, if given.
    pub account: Option<String>,
    /// `--model <id>`, if given: harness-native model id to launch with.
    /// Passed straight through to `spec.model` (no catalog lookup).
    pub model: Option<String>,
    /// `--hooks a,b`, if given: catalog hook ids to enable for this run.
    pub hooks: Option<Vec<String>>,
    /// `--safe`: expand the `[presets.safe]` policy.
    pub safe: bool,
    /// `--instructions <path>`: file contents (already read).
    pub instructions: Option<String>,
    /// `--prompt <text>`: initial prompt text.
    pub prompt: Option<String>,
    /// Everything after `--`, forwarded verbatim to the harness binary.
    pub passthrough_args: Vec<String>,
    /// Working directory for the run.
    pub cwd: PathBuf,
    /// `--isolate[=profile]`, if given: `None` = flag absent (no isolation);
    /// `Some(None)` = bare `--isolate` (sandboxed, no named profile);
    /// `Some(Some(profile))` = `--isolate=<profile>`.
    pub isolate: Option<Option<String>>,
    /// `--resume <id>`, if given: a raw harness-native session id to resume
    /// (no catalog/store lookup — passed straight through to `spec.resume`).
    pub resume: Option<String>,
    /// `--mcp-as-skill a,b`, if given: additionally expose these mcp ids as
    /// a latent skill pointer for this run (see [`crate::spec::McpAsSkill`]).
    /// Merged (union, deduped) with any catalog entries already marked
    /// `expose = "skill"` — this flag is additive, not a replacement of the
    /// `pick()` merge semantics used elsewhere in this module.
    pub mcp_as_skill: Option<Vec<String>>,
    /// `--profile <name>`, if given: the profile whose (flattened) fields sit
    /// between CLI flags and the per-harness/defaults layers. When absent, the
    /// implicit `default` profile is used if one exists. See [`crate::profile`].
    pub profile: Option<String>,
}

/// Extract the injected id from an [`McpRef`], for merging/validating
/// `--mcp-as-skill` against the effective set of injected mcps.
fn mcp_ref_id(r: &McpRef) -> &str {
    match r {
        McpRef::Catalog(def) => &def.id,
        McpRef::Inline(def) => &def.id,
        McpRef::InProcess(h) => &h.name,
    }
}

/// Pick the effective value for a merge key across four precedence layers:
/// CLI flag > profile > per-harness settings > defaults settings.
///
/// Returns the first `Some`, else `T::default()`. This is the "replace by
/// default" rule: the highest layer that *mentions* the key (i.e. is
/// `Some`, even `Some(vec![])`) wins outright — it does not merge with lower
/// layers.
fn pick<T: Default>(
    cli: Option<T>,
    profile: Option<T>,
    per_harness: Option<T>,
    defaults: Option<T>,
) -> T {
    cli.or(profile)
        .or(per_harness)
        .or(defaults)
        .unwrap_or_default()
}

/// Shape of an `--mcp-json` file: `{"mcpServers": {"<id>": {...}}}`, the same
/// shape most harnesses already use for inline MCP config.
#[derive(Debug, Deserialize)]
struct McpJsonFile {
    #[serde(default, rename = "mcpServers")]
    mcp_servers: BTreeMap<String, McpServer>,
}

/// Suggest near matches for an unknown id: a case-insensitive substring /
/// prefix match against the available ids. Falls back to listing everything
/// available if nothing matches.
fn suggest(query: &str, available: &[String]) -> Vec<String> {
    let q = query.to_lowercase();
    let mut matches: Vec<String> = available
        .iter()
        .filter(|id| {
            let idl = id.to_lowercase();
            idl.contains(&q) || q.contains(idl.as_str()) || idl.starts_with(&q)
        })
        .cloned()
        .collect();
    if matches.is_empty() {
        matches = available.to_vec();
    }
    matches
}

/// Resolve `flags` + `settings` + `registry` + `accounts` into a fully-resolved [`RunSpec`].
pub fn resolve(
    flags: &RunFlags,
    settings: &Settings,
    registry: &dyn Registry,
    accounts: &dyn AccountStore,
    profiles: &dyn ProfileStore,
) -> Result<RunSpec> {
    let per_harness = settings.harness.get(&flags.harness);

    // --- select + flatten the effective profile ---
    // Which profile: --profile > [harness.<id>].profile > [defaults].profile >
    // the implicit "default" (used only if it actually exists, so a machine
    // with no profiles still resolves). `resolve_flattened` folds the
    // `extends` inheritance chain root->leaf.
    let profile_name: Option<String> = match flags
        .profile
        .clone()
        .or_else(|| per_harness.and_then(|h| h.profile.clone()))
        .or_else(|| settings.defaults.profile.clone())
    {
        Some(name) => Some(name),
        // Implicit "default": used only if it actually exists.
        None => profiles.profile("default")?.map(|_| "default".to_string()),
    };
    let profile: Option<Profile> = match &profile_name {
        Some(name) => Some(
            crate::profile::resolve_flattened(profiles, name)
                .with_context(|| format!("resolving profile '{name}'"))?,
        ),
        None => None,
    };
    let profile_defaults = profile.as_ref().map(|p| &p.defaults);

    // Config-overlay base dirs across the `extends` chain (root -> leaf), for
    // provision to materialize on top of the harness-generated config.
    let config_bases: Vec<PathBuf> = match &profile_name {
        Some(name) => crate::profile::resolve_chain(profiles, name)?
            .iter()
            .filter_map(|p| profiles.overlay_base(&p.id, &flags.harness))
            .filter(|dir| dir.is_dir())
            .collect(),
        None => Vec::new(),
    };

    // --- merge (replace by default): flags > profile > per-harness > defaults ---
    let mcp_ids: Vec<String> = pick(
        flags.mcps.clone(),
        profile_defaults.and_then(|d| d.mcps.clone()),
        per_harness.and_then(|h| h.mcps.clone()),
        settings.defaults.mcps.clone(),
    );
    let skill_ids: Vec<String> = pick(
        flags.skills.clone(),
        profile_defaults.and_then(|d| d.skills.clone()),
        per_harness.and_then(|h| h.skills.clone()),
        settings.defaults.skills.clone(),
    );
    let account_id: Option<String> = flags
        .account
        .clone()
        .or_else(|| profile.as_ref().and_then(|p| p.account.clone()))
        .or_else(|| per_harness.and_then(|h| h.account.clone()))
        .or_else(|| settings.defaults.account.clone());
    let hook_ids: Vec<String> = pick(
        flags.hooks.clone(),
        profile_defaults.and_then(|d| d.hooks.clone()),
        per_harness.and_then(|h| h.hooks.clone()),
        settings.defaults.hooks.clone(),
    );

    // --- lookup: mcp ids -> McpRef::Catalog ---
    // Also collects catalog entries marked `expose = "skill"` (additive: the
    // MCP still lands in `mcps` below as normal — see `McpAsSkill`'s doc).
    let mut mcps = Vec::with_capacity(mcp_ids.len());
    let mut mcp_as_skill: Vec<McpAsSkill> = Vec::new();
    for id in &mcp_ids {
        match registry
            .mcp(id)
            .with_context(|| format!("looking up mcp '{id}'"))?
        {
            Some(entry) => {
                if entry.expose == McpExpose::Skill {
                    mcp_as_skill.push(McpAsSkill {
                        id: entry.id.clone(),
                        summary: entry.summary.clone(),
                    });
                }
                mcps.push(McpRef::Catalog(entry.def));
            }
            None => {
                let available: Vec<String> = registry
                    .mcps()
                    .with_context(|| "listing available mcps")?
                    .into_iter()
                    .map(|e| e.id)
                    .collect();
                let near = suggest(id, &available);
                bail!(
                    "unknown mcp id '{id}'; near matches: {}",
                    near.join(", ")
                );
            }
        }
    }

    // --- lookup: skill ids -> SkillRef ---
    let mut skills = Vec::with_capacity(skill_ids.len());
    for id in &skill_ids {
        match registry
            .skill(id)
            .with_context(|| format!("looking up skill '{id}'"))?
        {
            Some(entry) => skills.push(SkillRef {
                id: entry.id,
                path: entry.path,
            }),
            None => {
                let available: Vec<String> = registry
                    .skills()
                    .with_context(|| "listing available skills")?
                    .into_iter()
                    .map(|e| e.id)
                    .collect();
                let near = suggest(id, &available);
                bail!(
                    "unknown skill id '{id}'; near matches: {}",
                    near.join(", ")
                );
            }
        }
    }

    // --- --mcp-json: additive inline servers (bypass the catalog) ---
    if let Some(path) = &flags.mcp_json {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading mcp-json file {}", path.display()))?;
        let parsed: McpJsonFile = serde_json::from_str(&raw)
            .with_context(|| format!("parsing mcp-json file {}", path.display()))?;
        for (id, mut server) in parsed.mcp_servers {
            server.id = id;
            mcps.push(McpRef::Inline(server));
        }
    }

    // --- --mcp-as-skill: additive, merged (union, deduped) with the
    // catalog-derived `expose = "skill"` ids collected above. Each id named
    // must be in the effective set of injected mcps (catalog or inline) —
    // this flag never injects a new mcp, only marks an already-injected one.
    if let Some(ids) = &flags.mcp_as_skill {
        let effective_ids: Vec<String> =
            mcps.iter().map(|r| mcp_ref_id(r).to_string()).collect();
        for id in ids {
            if !effective_ids.iter().any(|e| e == id) {
                let near = suggest(id, &effective_ids);
                bail!(
                    "unknown mcp id '{id}' for --mcp-as-skill; near matches: {}",
                    near.join(", ")
                );
            }
            if mcp_as_skill.iter().any(|m| &m.id == id) {
                continue; // already added via catalog `expose = "skill"`
            }
            // Reuse the catalog's summary when this id happens to have one
            // (an inline-only id, or a catalog entry with no summary, gets
            // `None`).
            let summary = registry.mcp(id).ok().flatten().and_then(|e| e.summary);
            mcp_as_skill.push(McpAsSkill {
                id: id.clone(),
                summary,
            });
        }
    }

    // --- --safe: expand [presets.safe] ---
    let policy = if flags.safe {
        match settings.presets.get("safe") {
            Some(p) => Some(p.clone()),
            None => bail!("--safe given but no [presets.safe] defined"),
        }
    } else {
        None
    };

    // --- lookup: account id -> Account ---
    let account = match account_id {
        Some(id) => {
            match accounts
                .account(&id)
                .with_context(|| format!("looking up account '{id}'"))?
            {
                Some(acct) => Some(acct),
                None => {
                    let available: Vec<String> = accounts
                        .accounts()
                        .with_context(|| "listing available accounts")?
                        .into_iter()
                        .map(|a| a.id)
                        .collect();
                    let near = suggest(&id, &available);
                    bail!(
                        "account '{id}' not found; near matches: {}",
                        near.join(", ")
                    );
                }
            }
        }
        None => None,
    };

    // --- lookup: hook ids -> HookRef ---
    let mut hooks = Vec::with_capacity(hook_ids.len());
    for id in &hook_ids {
        match settings.hooks.get(id) {
            Some(def) => hooks.push(HookRef {
                id: id.clone(),
                event: def.event.clone(),
                command: def.command.clone(),
                matcher: def.matcher.clone(),
            }),
            None => {
                let available: Vec<String> = settings.hooks.keys().cloned().collect();
                let near = suggest(id, &available);
                bail!(
                    "unknown hook id '{id}'; near matches: {}",
                    near.join(", ")
                );
            }
        }
    }

    let mut spec = RunSpec::new(flags.harness.clone(), flags.cwd.clone());
    spec.skills = skills;
    spec.mcps = mcps;
    spec.mcp_as_skill = mcp_as_skill;
    spec.hooks = hooks;
    spec.account = account;
    spec.policy = policy;
    spec.model = flags
        .model
        .clone()
        .or_else(|| profile_defaults.and_then(|d| d.model.clone()));
    spec.passthrough_args = flags.passthrough_args.clone();

    // --- instructions & prompt ---
    // Instructions: the CLI passes already-read file *contents*; a profile
    // carries a *path* (read here, relative to CWD or absolute) used only when
    // the flag is absent.
    let instructions = match &flags.instructions {
        Some(text) => Some(text.clone()),
        None => match profile_defaults.and_then(|d| d.instructions.clone()) {
            Some(path) => Some(std::fs::read_to_string(&path).with_context(|| {
                format!("reading profile instructions file {}", path.display())
            })?),
            None => None,
        },
    };
    let initial = Instructions {
        instructions,
        prompt: flags.prompt.clone(),
    };
    spec.initial = if initial.is_empty() { None } else { Some(initial) };

    // --- isolation: --isolate[=profile] > the profile's `isolate` default ---
    spec.isolation = match &flags.isolate {
        Some(Some(name)) => Isolation::Sandboxed(name.clone()),
        Some(None) => Isolation::Sandboxed(String::new()),
        None => match profile.as_ref().and_then(|p| p.isolate.clone()) {
            Some(ProfileIsolate::Sandboxed(name)) => Isolation::Sandboxed(name),
            Some(ProfileIsolate::Off) | None => Isolation::None,
        },
    };

    // --- --resume <id>: a raw harness-native id, no lookup needed ---
    spec.resume = flags.resume.clone();

    // --- profile config-overlay bases (materialized by provision) ---
    spec.config_bases = config_bases;

    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Account, EmptyAccountStore};
    use crate::profile::EmptyProfileStore;
    use crate::config::{McpServer, McpTransport};
    use crate::registry::{McpEntry, McpExpose, SkillEntry, SkillMeta};
    use crate::spec::Policy;
    use std::path::PathBuf;

    /// An in-memory registry test-double so merge tests don't touch the
    /// filesystem.
    struct TestRegistry {
        mcps: Vec<McpEntry>,
        skills: Vec<SkillEntry>,
    }

    impl Registry for TestRegistry {
        fn skills(&self) -> Result<Vec<SkillEntry>> {
            Ok(self.skills.clone())
        }
        fn mcps(&self) -> Result<Vec<McpEntry>> {
            Ok(self.mcps.clone())
        }
    }

    /// An in-memory account store test-double so account-lookup tests don't
    /// touch the filesystem.
    struct TestAccountStore {
        accounts: Vec<Account>,
    }

    impl AccountStore for TestAccountStore {
        fn accounts(&self) -> Result<Vec<Account>> {
            Ok(self.accounts.clone())
        }
    }

    /// In-memory profile store test-double so profile-layer tests don't touch
    /// the filesystem.
    struct TestProfileStore {
        profiles: Vec<Profile>,
    }

    impl ProfileStore for TestProfileStore {
        fn profiles(&self) -> Result<Vec<Profile>> {
            Ok(self.profiles.clone())
        }
    }

    /// A profile with just an id (all other fields default), for the caller to
    /// fill in.
    fn prof(id: &str) -> Profile {
        Profile {
            id: id.to_string(),
            ..Default::default()
        }
    }

    fn mcp(id: &str) -> McpEntry {
        McpEntry {
            id: id.to_string(),
            def: McpServer {
                id: id.to_string(),
                transport: McpTransport::Stdio,
                command: Some(format!("{id}-cmd")),
                args: vec![],
                env: Default::default(),
                url: None,
                headers: Default::default(),
            },
            expose: McpExpose::Tools,
            summary: None,
        }
    }

    /// Like [`mcp`] but marked `expose = "skill"` with the given summary.
    fn mcp_as_skill_entry(id: &str, summary: &str) -> McpEntry {
        McpEntry {
            expose: McpExpose::Skill,
            summary: Some(summary.to_string()),
            ..mcp(id)
        }
    }

    fn skill(id: &str) -> SkillEntry {
        SkillEntry {
            id: id.to_string(),
            path: PathBuf::from(format!("/catalog/skills/{id}")),
            meta: SkillMeta::default(),
        }
    }

    fn test_registry() -> TestRegistry {
        TestRegistry {
            mcps: vec![mcp("github"), mcp("postgres"), mcp("figma")],
            skills: vec![skill("web-designer"), skill("reviewer")],
        }
    }

    fn flags(harness: &str) -> RunFlags {
        RunFlags {
            harness: harness.to_string(),
            cwd: PathBuf::from("/tmp/project"),
            ..Default::default()
        }
    }

    // `mcp_ref_id` is defined once at module scope (above `resolve`) and
    // pulled in here via `use super::*`.

    #[test]
    fn cli_mcps_replaces_settings_layers() {
        let mut f = flags("claude");
        f.mcps = Some(vec!["figma".to_string()]);

        let mut settings = Settings::default();
        settings.defaults.mcps = Some(vec!["github".to_string()]);
        settings.harness.insert(
            "claude".to_string(),
            crate::settings::HarnessDefaults {
                mcps: Some(vec!["postgres".to_string()]),
                skills: None,
                account: None,
                hooks: None,
                profile: None,
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");
        let ids: Vec<&str> = spec.mcps.iter().map(mcp_ref_id).collect();
        assert_eq!(ids, vec!["figma"]);
    }

    #[test]
    fn per_harness_replaces_defaults_when_no_cli_flag() {
        let f = flags("claude");

        let mut settings = Settings::default();
        settings.defaults.mcps = Some(vec!["github".to_string()]);
        settings.harness.insert(
            "claude".to_string(),
            crate::settings::HarnessDefaults {
                mcps: Some(vec!["postgres".to_string()]),
                skills: None,
                account: None,
                hooks: None,
                profile: None,
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");
        let ids: Vec<&str> = spec.mcps.iter().map(mcp_ref_id).collect();
        assert_eq!(ids, vec!["postgres"]);
    }

    #[test]
    fn defaults_used_when_no_cli_and_no_per_harness() {
        let f = flags("claude");

        let mut settings = Settings::default();
        settings.defaults.mcps = Some(vec!["github".to_string()]);

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");
        let ids: Vec<&str> = spec.mcps.iter().map(mcp_ref_id).collect();
        assert_eq!(ids, vec!["github"]);
    }

    #[test]
    fn explicit_empty_replaces_lower_layers() {
        let f = flags("claude");

        let mut settings = Settings::default();
        settings.defaults.mcps = Some(vec!["github".to_string()]);
        settings.harness.insert(
            "claude".to_string(),
            crate::settings::HarnessDefaults {
                mcps: Some(vec![]), // explicitly empty
                skills: None,
                account: None,
                hooks: None,
                profile: None,
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");
        assert!(spec.mcps.is_empty());
    }

    #[test]
    fn missing_mcp_id_is_an_error_naming_the_id() {
        let mut f = flags("claude");
        f.mcps = Some(vec!["nonexistent".to_string()]);

        let settings = Settings::default();
        let reg = test_registry();
        let err = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect_err("should fail");
        let msg = err.to_string();
        assert!(msg.contains("nonexistent"), "message was: {msg}");
    }

    #[test]
    fn safe_with_preset_sets_policy() {
        let mut f = flags("claude");
        f.safe = true;

        let mut settings = Settings::default();
        settings.presets.insert(
            "safe".to_string(),
            Policy {
                permission_mode: Some("restricted".to_string()),
                allow: vec![],
                ask: vec![],
                deny: vec!["Bash(rm *)".to_string()],
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");
        let policy = spec.policy.expect("policy should be set");
        assert_eq!(policy.permission_mode.as_deref(), Some("restricted"));
        assert_eq!(policy.deny, vec!["Bash(rm *)".to_string()]);
    }

    #[test]
    fn safe_without_preset_is_an_error() {
        let mut f = flags("claude");
        f.safe = true;

        let settings = Settings::default();
        let reg = test_registry();
        let err = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect_err("should fail");
        assert!(err.to_string().contains("presets.safe"));
    }

    #[test]
    fn mcp_json_is_additive_to_catalog_mcps() {
        let mut f = flags("claude");
        f.mcps = Some(vec!["github".to_string()]);

        let temp = tempfile::TempDir::new().expect("tempdir");
        let path = temp.path().join("inline.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {"custom": {"command": "custom-cmd", "args": []}}}"#,
        )
        .expect("write inline json");
        f.mcp_json = Some(path);

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        assert_eq!(spec.mcps.len(), 2);
        let catalog_ids: Vec<&str> = spec
            .mcps
            .iter()
            .filter_map(|r| match r {
                McpRef::Catalog(def) => Some(def.id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(catalog_ids, vec!["github"]);

        let inline_ids: Vec<&str> = spec
            .mcps
            .iter()
            .filter_map(|r| match r {
                McpRef::Inline(def) => Some(def.id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(inline_ids, vec!["custom"]);
    }

    #[test]
    fn instructions_and_prompt_populate_spec_initial() {
        let mut f = flags("claude");
        f.instructions = Some("REMEMBER: be helpful".to_string());
        f.prompt = Some("do it".to_string());

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        let initial = spec.initial.expect("should have initial");
        assert_eq!(initial.instructions.as_deref(), Some("REMEMBER: be helpful"));
        assert_eq!(initial.prompt.as_deref(), Some("do it"));
    }

    #[test]
    fn empty_instructions_and_prompt_yields_no_spec_initial() {
        let f = flags("claude");

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        assert!(spec.initial.is_none());
    }

    #[test]
    fn account_id_resolves_to_account_from_store() {
        let mut f = flags("claude");
        f.account = Some("work".to_string());

        let settings = Settings::default();
        let reg = test_registry();
        let accounts = TestAccountStore {
            accounts: vec![Account {
                id: "work".to_string(),
                api_key_env: Some("WORK_KEY".to_string()),
                ..Default::default()
            }],
        };
        let spec = resolve(&f, &settings, &reg, &accounts, &EmptyProfileStore).expect("resolve");

        let account = spec.account.expect("account should be set");
        assert_eq!(account.id, "work");
        assert_eq!(account.api_key_env.as_deref(), Some("WORK_KEY"));
    }

    #[test]
    fn unknown_account_id_errors_with_near_matches() {
        let mut f = flags("claude");
        f.account = Some("wrk".to_string());

        let settings = Settings::default();
        let reg = test_registry();
        let accounts = TestAccountStore {
            accounts: vec![Account {
                id: "work".to_string(),
                ..Default::default()
            }],
        };
        let err = resolve(&f, &settings, &reg, &accounts, &EmptyProfileStore).expect_err("should fail");
        let msg = err.to_string();
        assert!(msg.contains("wrk"), "message was: {msg}");
        assert!(msg.contains("work"), "message was: {msg}");
    }

    #[test]
    fn no_account_flag_leaves_spec_account_none() {
        let f = flags("claude");

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        assert!(spec.account.is_none());
    }

    #[test]
    fn isolate_flag_absent_yields_isolation_none() {
        let f = flags("claude");

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        assert!(matches!(spec.isolation, crate::spec::Isolation::None));
    }

    #[test]
    fn bare_isolate_flag_yields_sandboxed_with_empty_profile() {
        let mut f = flags("claude");
        f.isolate = Some(None);

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        match spec.isolation {
            crate::spec::Isolation::Sandboxed(profile) => assert_eq!(profile, ""),
            other => panic!("expected Sandboxed, got {other:?}"),
        }
    }

    #[test]
    fn resume_flag_sets_spec_resume() {
        let mut f = flags("claude");
        f.resume = Some("abc".to_string());

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        assert_eq!(spec.resume.as_deref(), Some("abc"));
    }

    #[test]
    fn no_resume_flag_leaves_spec_resume_none() {
        let f = flags("claude");

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        assert!(spec.resume.is_none());
    }

    #[test]
    fn isolate_flag_with_profile_yields_sandboxed_with_profile() {
        let mut f = flags("claude");
        f.isolate = Some(Some("dev".to_string()));

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        match spec.isolation {
            crate::spec::Isolation::Sandboxed(profile) => assert_eq!(profile, "dev"),
            other => panic!("expected Sandboxed, got {other:?}"),
        }
    }

    fn hook_def(event: &str, command: &str, matcher: Option<&str>) -> crate::settings::HookDef {
        crate::settings::HookDef {
            event: event.to_string(),
            command: command.to_string(),
            matcher: matcher.map(str::to_string),
        }
    }

    #[test]
    fn cli_hooks_selects_def_and_populates_spec_hooks() {
        let mut f = flags("claude");
        f.hooks = Some(vec!["notify".to_string()]);

        let mut settings = Settings::default();
        settings.hooks.insert(
            "notify".to_string(),
            hook_def("PreToolUse", "notify-send hi", Some("Bash")),
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");
        assert_eq!(spec.hooks.len(), 1);
        let hook = &spec.hooks[0];
        assert_eq!(hook.id, "notify");
        assert_eq!(hook.event, "PreToolUse");
        assert_eq!(hook.command, "notify-send hi");
        assert_eq!(hook.matcher.as_deref(), Some("Bash"));
    }

    #[test]
    fn per_harness_hooks_replaces_defaults_when_no_cli_flag() {
        let f = flags("claude");

        let mut settings = Settings::default();
        settings
            .hooks
            .insert("a".to_string(), hook_def("Stop", "echo a", None));
        settings
            .hooks
            .insert("b".to_string(), hook_def("Stop", "echo b", None));
        settings.defaults.hooks = Some(vec!["a".to_string()]);
        settings.harness.insert(
            "claude".to_string(),
            crate::settings::HarnessDefaults {
                mcps: None,
                skills: None,
                account: None,
                hooks: Some(vec!["b".to_string()]),
                profile: None,
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");
        let ids: Vec<&str> = spec.hooks.iter().map(|h| h.id.as_str()).collect();
        assert_eq!(ids, vec!["b"]);
    }

    #[test]
    fn unknown_hook_id_is_an_error_naming_the_id_and_near_match() {
        let mut f = flags("claude");
        f.hooks = Some(vec!["notfy".to_string()]);

        let mut settings = Settings::default();
        settings
            .hooks
            .insert("notify".to_string(), hook_def("Stop", "echo hi", None));

        let reg = test_registry();
        let err = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect_err("should fail");
        let msg = err.to_string();
        assert!(msg.contains("notfy"), "message was: {msg}");
        assert!(msg.contains("notify"), "message was: {msg}");
    }

    #[test]
    fn catalog_expose_skill_populates_mcp_as_skill_and_stays_in_mcps() {
        let mut f = flags("claude");
        f.mcps = Some(vec!["postgres".to_string()]);

        let reg = TestRegistry {
            mcps: vec![mcp_as_skill_entry("postgres", "Query a DB.")],
            skills: vec![],
        };

        let settings = Settings::default();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        // Additive: the mcp is still injected as normal...
        let ids: Vec<&str> = spec.mcps.iter().map(mcp_ref_id).collect();
        assert_eq!(ids, vec!["postgres"]);

        // ...and also marked for a skill pointer, carrying the summary.
        assert_eq!(spec.mcp_as_skill.len(), 1);
        assert_eq!(spec.mcp_as_skill[0].id, "postgres");
        assert_eq!(spec.mcp_as_skill[0].summary.as_deref(), Some("Query a DB."));
    }

    #[test]
    fn mcp_as_skill_flag_adds_a_tools_default_entry() {
        let mut f = flags("claude");
        f.mcps = Some(vec!["github".to_string()]);
        f.mcp_as_skill = Some(vec!["github".to_string()]);

        let reg = test_registry(); // "github" defaults to expose = tools
        let settings = Settings::default();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        assert_eq!(spec.mcp_as_skill.len(), 1);
        assert_eq!(spec.mcp_as_skill[0].id, "github");
    }

    #[test]
    fn unknown_mcp_as_skill_id_is_an_error_naming_it() {
        let mut f = flags("claude");
        f.mcps = Some(vec!["github".to_string()]);
        f.mcp_as_skill = Some(vec!["nonexistent".to_string()]);

        let reg = test_registry();
        let settings = Settings::default();
        let err = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect_err("should fail");
        assert!(err.to_string().contains("nonexistent"));
    }

    #[test]
    fn mcp_as_skill_flag_and_catalog_expose_dedupe() {
        let mut f = flags("claude");
        f.mcps = Some(vec!["postgres".to_string()]);
        f.mcp_as_skill = Some(vec!["postgres".to_string()]);

        let reg = TestRegistry {
            mcps: vec![mcp_as_skill_entry("postgres", "Query a DB.")],
            skills: vec![],
        };

        let settings = Settings::default();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &EmptyProfileStore).expect("resolve");

        // Named by both the catalog entry and the flag: appears exactly once.
        assert_eq!(spec.mcp_as_skill.len(), 1);
        assert_eq!(spec.mcp_as_skill[0].id, "postgres");
    }

    // ---- profile layer (B1) ----

    #[test]
    fn profile_fields_apply_when_no_flag_or_settings() {
        let mut f = flags("claude");
        f.profile = Some("work".to_string());

        let mut work = prof("work");
        work.defaults.mcps = Some(vec!["postgres".to_string()]);
        work.defaults.model = Some("sonnet".to_string());
        let profiles = TestProfileStore {
            profiles: vec![work],
        };

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &profiles).expect("resolve");

        let ids: Vec<&str> = spec.mcps.iter().map(mcp_ref_id).collect();
        assert_eq!(ids, vec!["postgres"]);
        assert_eq!(spec.model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn flags_override_profile_fields() {
        let mut f = flags("claude");
        f.profile = Some("work".to_string());
        f.mcps = Some(vec!["figma".to_string()]);
        f.model = Some("haiku".to_string());

        let mut work = prof("work");
        work.defaults.mcps = Some(vec!["postgres".to_string()]);
        work.defaults.model = Some("sonnet".to_string());
        let profiles = TestProfileStore {
            profiles: vec![work],
        };

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &profiles).expect("resolve");

        let ids: Vec<&str> = spec.mcps.iter().map(mcp_ref_id).collect();
        assert_eq!(ids, vec!["figma"], "explicit --mcps must override the profile");
        assert_eq!(spec.model.as_deref(), Some("haiku"));
    }

    #[test]
    fn profile_sits_above_per_harness_and_defaults() {
        // flags absent; profile mcps must win over per-harness AND defaults.
        let mut f = flags("claude");
        f.profile = Some("work".to_string());

        let mut work = prof("work");
        work.defaults.mcps = Some(vec!["figma".to_string()]);
        let profiles = TestProfileStore {
            profiles: vec![work],
        };

        let mut settings = Settings::default();
        settings.defaults.mcps = Some(vec!["github".to_string()]);
        settings.harness.insert(
            "claude".to_string(),
            crate::settings::HarnessDefaults {
                mcps: Some(vec!["postgres".to_string()]),
                skills: None,
                account: None,
                hooks: None,
                profile: None,
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &profiles).expect("resolve");
        let ids: Vec<&str> = spec.mcps.iter().map(mcp_ref_id).collect();
        assert_eq!(ids, vec!["figma"]);
    }

    #[test]
    fn flag_account_overrides_profile_account() {
        let mut f = flags("claude");
        f.profile = Some("work".to_string());
        f.account = Some("personal".to_string());

        let mut work = prof("work");
        work.account = Some("workacct".to_string());
        let profiles = TestProfileStore {
            profiles: vec![work],
        };

        let accounts = TestAccountStore {
            accounts: vec![
                Account {
                    id: "personal".to_string(),
                    ..Default::default()
                },
                Account {
                    id: "workacct".to_string(),
                    ..Default::default()
                },
            ],
        };

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &accounts, &profiles).expect("resolve");
        assert_eq!(spec.account.expect("account").id, "personal");
    }

    #[test]
    fn profile_account_used_when_no_account_flag() {
        let mut f = flags("claude");
        f.profile = Some("work".to_string());

        let mut work = prof("work");
        work.account = Some("workacct".to_string());
        let profiles = TestProfileStore {
            profiles: vec![work],
        };
        let accounts = TestAccountStore {
            accounts: vec![Account {
                id: "workacct".to_string(),
                ..Default::default()
            }],
        };

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &accounts, &profiles).expect("resolve");
        assert_eq!(spec.account.expect("account").id, "workacct");
    }

    #[test]
    fn profile_isolate_sets_spec_isolation_when_no_isolate_flag() {
        let mut f = flags("claude");
        f.profile = Some("locked".to_string());

        let mut locked = prof("locked");
        locked.isolate = Some(crate::profile::ProfileIsolate::Sandboxed("dev".to_string()));
        let profiles = TestProfileStore {
            profiles: vec![locked],
        };

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &profiles).expect("resolve");
        match spec.isolation {
            Isolation::Sandboxed(name) => assert_eq!(name, "dev"),
            other => panic!("expected Sandboxed, got {other:?}"),
        }
    }

    #[test]
    fn explicit_isolate_flag_overrides_profile_isolate() {
        let mut f = flags("claude");
        f.profile = Some("locked".to_string());
        f.isolate = Some(Some("prod".to_string()));

        let mut locked = prof("locked");
        locked.isolate = Some(crate::profile::ProfileIsolate::Sandboxed("dev".to_string()));
        let profiles = TestProfileStore {
            profiles: vec![locked],
        };

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &profiles).expect("resolve");
        match spec.isolation {
            Isolation::Sandboxed(name) => assert_eq!(name, "prod"),
            other => panic!("expected Sandboxed, got {other:?}"),
        }
    }

    #[test]
    fn implicit_default_profile_is_used_when_present_and_no_flag() {
        let f = flags("claude"); // no --profile

        let mut default = prof("default");
        default.defaults.model = Some("opus".to_string());
        let profiles = TestProfileStore {
            profiles: vec![default],
        };

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &profiles).expect("resolve");
        assert_eq!(spec.model.as_deref(), Some("opus"));
    }

    #[test]
    fn no_profiles_and_no_flag_resolves_without_a_profile_layer() {
        let f = flags("claude");
        let profiles = TestProfileStore { profiles: vec![] };

        let settings = Settings::default();
        let reg = test_registry();
        // Must NOT error on a missing implicit "default".
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore, &profiles).expect("resolve");
        assert!(spec.model.is_none());
    }

    #[test]
    fn explicit_unknown_profile_errors() {
        let mut f = flags("claude");
        f.profile = Some("nope".to_string());
        let profiles = TestProfileStore { profiles: vec![] };

        let settings = Settings::default();
        let reg = test_registry();
        let err = resolve(&f, &settings, &reg, &EmptyAccountStore, &profiles).expect_err("should fail");
        assert!(err.to_string().contains("nope"), "message was: {err}");
    }
}
