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
use crate::registry::Registry;
use crate::settings::Settings;
use crate::spec::{HarnessId, Instructions, McpRef, RunSpec, SkillRef};
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
}

/// Pick the effective value for a merge key across three precedence layers.
///
/// Returns the first `Some`, else `T::default()`. This is the "replace by
/// default" rule: the highest layer that *mentions* the key (i.e. is
/// `Some`, even `Some(vec![])`) wins outright — it does not merge with lower
/// layers.
fn pick<T: Default>(cli: Option<T>, per_harness: Option<T>, defaults: Option<T>) -> T {
    cli.or(per_harness).or(defaults).unwrap_or_default()
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
) -> Result<RunSpec> {
    let per_harness = settings.harness.get(&flags.harness);

    // --- merge (replace by default) ---
    let mcp_ids: Vec<String> = pick(
        flags.mcps.clone(),
        per_harness.and_then(|h| h.mcps.clone()),
        settings.defaults.mcps.clone(),
    );
    let skill_ids: Vec<String> = pick(
        flags.skills.clone(),
        per_harness.and_then(|h| h.skills.clone()),
        settings.defaults.skills.clone(),
    );
    let account_id: Option<String> = flags
        .account
        .clone()
        .or_else(|| per_harness.and_then(|h| h.account.clone()))
        .or_else(|| settings.defaults.account.clone());

    // --- lookup: mcp ids -> McpRef::Catalog ---
    let mut mcps = Vec::with_capacity(mcp_ids.len());
    for id in &mcp_ids {
        match registry
            .mcp(id)
            .with_context(|| format!("looking up mcp '{id}'"))?
        {
            Some(entry) => mcps.push(McpRef::Catalog(entry.def)),
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

    let mut spec = RunSpec::new(flags.harness.clone(), flags.cwd.clone());
    spec.skills = skills;
    spec.mcps = mcps;
    spec.account = account;
    spec.policy = policy;
    spec.passthrough_args = flags.passthrough_args.clone();

    // --- instructions & prompt ---
    let initial = Instructions {
        instructions: flags.instructions.clone(),
        prompt: flags.prompt.clone(),
    };
    spec.initial = if initial.is_empty() { None } else { Some(initial) };

    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Account, EmptyAccountStore};
    use crate::config::{McpServer, McpTransport};
    use crate::registry::{McpEntry, SkillEntry, SkillMeta};
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

    fn mcp_ref_id(r: &McpRef) -> &str {
        match r {
            McpRef::Catalog(def) => &def.id,
            McpRef::Inline(def) => &def.id,
            McpRef::InProcess(h) => &h.name,
        }
    }

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
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");
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
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");
        let ids: Vec<&str> = spec.mcps.iter().map(mcp_ref_id).collect();
        assert_eq!(ids, vec!["postgres"]);
    }

    #[test]
    fn defaults_used_when_no_cli_and_no_per_harness() {
        let f = flags("claude");

        let mut settings = Settings::default();
        settings.defaults.mcps = Some(vec!["github".to_string()]);

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");
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
            },
        );

        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");
        assert!(spec.mcps.is_empty());
    }

    #[test]
    fn missing_mcp_id_is_an_error_naming_the_id() {
        let mut f = flags("claude");
        f.mcps = Some(vec!["nonexistent".to_string()]);

        let settings = Settings::default();
        let reg = test_registry();
        let err = resolve(&f, &settings, &reg, &EmptyAccountStore).expect_err("should fail");
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
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");
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
        let err = resolve(&f, &settings, &reg, &EmptyAccountStore).expect_err("should fail");
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
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");

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
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");

        let initial = spec.initial.expect("should have initial");
        assert_eq!(initial.instructions.as_deref(), Some("REMEMBER: be helpful"));
        assert_eq!(initial.prompt.as_deref(), Some("do it"));
    }

    #[test]
    fn empty_instructions_and_prompt_yields_no_spec_initial() {
        let f = flags("claude");

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");

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
        let spec = resolve(&f, &settings, &reg, &accounts).expect("resolve");

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
        let err = resolve(&f, &settings, &reg, &accounts).expect_err("should fail");
        let msg = err.to_string();
        assert!(msg.contains("wrk"), "message was: {msg}");
        assert!(msg.contains("work"), "message was: {msg}");
    }

    #[test]
    fn no_account_flag_leaves_spec_account_none() {
        let f = flags("claude");

        let settings = Settings::default();
        let reg = test_registry();
        let spec = resolve(&f, &settings, &reg, &EmptyAccountStore).expect("resolve");

        assert!(spec.account.is_none());
    }
}
