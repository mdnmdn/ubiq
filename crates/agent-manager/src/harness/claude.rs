//! Claude Code provisioner.
//!
//! Transcribes `_docs/harness/claude-code.md` (esp. "Orchestration / headless
//! invocation", "MCP at launch", "Skills at launch") into a [`Harness`] impl.
//!
//! The "custom config folder" bridge: Claude Code's user config dir can be
//! relocated with the `CLAUDE_CONFIG_DIR` environment variable. Provisioning
//! points that variable at the ephemeral dir instead of the real `~/.claude`,
//! so skills/settings/memory are injected without ever touching the user's
//! real config.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::config::{McpServer, McpTransport};
use crate::spec::{McpRef, RunSpec};
use crate::Result;

use super::{Harness, IoSupport, Launch};

/// Environment variables stripped from the child so a nested `am`/Claude Code
/// invocation doesn't inherit the parent session's identity.
const ENV_HYGIENE: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SSE_PORT",
];

/// The marker that wraps `am`-managed content in `CLAUDE.md`, so user-authored
/// content (were any to coexist in the file) is distinguishable from ours.
const MANAGED_BEGIN: &str = "<!-- agent-manager:begin -->";
const MANAGED_END: &str = "<!-- agent-manager:end -->";

/// The Claude Code harness provisioner.
#[derive(Debug, Clone, Default)]
pub struct Claude;

impl Claude {
    /// Construct the Claude Code harness descriptor.
    pub fn new() -> Self {
        Claude
    }
}

impl Harness for Claude {
    fn id(&self) -> crate::spec::HarnessId {
        "claude-code".to_string()
    }

    fn display_name(&self) -> &str {
        "Claude Code"
    }

    fn command(&self) -> &str {
        "claude"
    }

    fn aliases(&self) -> &[&str] {
        &["claude"]
    }

    fn io_support(&self) -> IoSupport {
        IoSupport {
            passthrough: true,
            structured: true,
        }
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // 1. Skills: copy each skill folder into <dir>/skills/<id>/.
        let skills_dir = dir.join("skills");
        for skill in &spec.skills {
            if !skill.path.exists() {
                bail!(
                    "skill '{}' points at a path that does not exist: {}",
                    skill.id,
                    skill.path.display()
                );
            }
            let dest = skills_dir.join(&skill.id);
            super::copy_dir_recursive(&skill.path, &dest).with_context(|| {
                format!(
                    "copying skill '{}' from {} to {}",
                    skill.id,
                    skill.path.display(),
                    dest.display()
                )
            })?;
        }

        // 2. MCP: always write <dir>/mcp.json, even if empty, so
        // --strict-mcp-config yields a fully-controlled server set.
        let mcp_json = build_mcp_json(&spec.mcps)?;
        let mcp_path = dir.join("mcp.json");
        std::fs::write(&mcp_path, serde_json::to_string_pretty(&mcp_json)?)
            .with_context(|| format!("writing {}", mcp_path.display()))?;

        // 3. Policy + account helper: <dir>/settings.json, written when
        // either a policy or an account helper is present.
        let mut settings_obj = serde_json::Map::new();
        if let Some(policy) = &spec.policy {
            let mut permissions = serde_json::Map::new();
            if let Some(mode) = &policy.permission_mode {
                permissions.insert("defaultMode".to_string(), json!(mode));
            }
            permissions.insert("allow".to_string(), json!(policy.allow));
            permissions.insert("ask".to_string(), json!(policy.ask));
            permissions.insert("deny".to_string(), json!(policy.deny));
            settings_obj.insert("permissions".to_string(), Value::Object(permissions));
        }
        if let Some(account) = &spec.account
            && let Some(helper) = &account.helper
        {
            // `am` never runs the helper or sees its output; it only wires
            // the command string into Claude Code's native key-helper slot.
            settings_obj.insert("apiKeyHelper".to_string(), json!(helper));
        }
        if !settings_obj.is_empty() {
            let settings_path = dir.join("settings.json");
            std::fs::write(
                &settings_path,
                serde_json::to_string_pretty(&Value::Object(settings_obj))?,
            )
            .with_context(|| format!("writing {}", settings_path.display()))?;
        }

        // 4. Instructions: <dir>/CLAUDE.md, wrapped in a managed block.
        if let Some(instr_text) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
            let claude_md = format!("{MANAGED_BEGIN}\n{}\n{MANAGED_END}\n", instr_text);
            let claude_md_path = dir.join("CLAUDE.md");
            std::fs::write(&claude_md_path, claude_md)
                .with_context(|| format!("writing {}", claude_md_path.display()))?;
        }

        // 5. Build the launch. Structured mode launches Claude Code headless
        // (`-p --output-format stream-json --input-format stream-json`),
        // with the prompt delivered as an NDJSON line on stdin by the
        // bridge rather than a trailing positional argument; passthrough
        // mode keeps the interactive argv shape from P1.
        let structured = spec.io == crate::spec::IoModes::Structured;

        let mut args = Vec::new();
        if structured {
            args.extend(
                [
                    "-p",
                    "--output-format",
                    "stream-json",
                    "--input-format",
                    "stream-json",
                    "--verbose",
                ]
                .map(str::to_string),
            );
        }
        args.push("--mcp-config".to_string());
        args.push(mcp_path.display().to_string());
        args.push("--strict-mcp-config".to_string());
        if structured {
            args.extend(
                [
                    "--permission-mode",
                    "bypassPermissions",
                    "--disallowedTools",
                    "AskUserQuestion",
                ]
                .map(str::to_string),
            );
        }
        args.extend(spec.passthrough_args.iter().cloned());

        // Append prompt as trailing positional argument, passthrough mode
        // only — structured mode's bridge sends it as NDJSON on stdin.
        if !structured
            && let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref())
        {
            args.push(prompt.clone());
        }

        // 6. Account: inject credential *references* into the child's env.
        // `am`'s account store never holds secret material — only env-var
        // NAMES, a base URL, a helper command, and/or a home dir path. The
        // only place a secret value is ever touched is the transient
        // `std::env::var` read below; it lands in `Launch.env` (in-memory,
        // passed to the child process) and is never written to disk.
        let mut env = vec![("CLAUDE_CONFIG_DIR".to_string(), dir.display().to_string())];
        if let Some(account) = &spec.account {
            if let Some(base_url) = &account.base_url {
                env.push(("ANTHROPIC_BASE_URL".to_string(), base_url.clone()));
            }
            if let Some(name) = &account.api_key_env {
                let value = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "account '{}' references env var '{}' which is not set",
                        account.id,
                        name
                    )
                })?;
                env.push(("ANTHROPIC_API_KEY".to_string(), value));
            }
            if let Some(name) = &account.auth_token_env {
                let value = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "account '{}' references env var '{}' which is not set",
                        account.id,
                        name
                    )
                })?;
                env.push(("ANTHROPIC_AUTH_TOKEN".to_string(), value));
            }
            if let Some(home) = &account.home {
                // CLAUDE_CONFIG_DIR stays pointed at the ephemeral dir
                // (skills/mcp injection); OAuth/keychain credentials resolve
                // relative to HOME, so a private-HOME account keeps its own
                // credential store while still getting injected skills/mcp.
                env.push(("HOME".to_string(), home.display().to_string()));
            }
        }

        Ok(Launch {
            program: "claude".to_string(),
            args,
            env,
            env_remove: ENV_HYGIENE.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        Ok(Box::new(crate::io::JsonlBridge::new(child)?))
    }
}

/// Render one [`McpServer`] into the JSON shape Claude Code's `--mcp-config`
/// file expects, keyed by transport.
fn server_json(server: &McpServer) -> Value {
    match server.transport {
        McpTransport::Stdio => {
            let mut obj = serde_json::Map::new();
            if let Some(command) = &server.command {
                obj.insert("command".to_string(), json!(command));
            }
            obj.insert("args".to_string(), json!(server.args));
            if !server.env.is_empty() {
                obj.insert("env".to_string(), json!(server.env));
            }
            Value::Object(obj)
        }
        McpTransport::Http => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), json!("http"));
            obj.insert("url".to_string(), json!(server.url));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
            Value::Object(obj)
        }
        McpTransport::Sse => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), json!("sse"));
            obj.insert("url".to_string(), json!(server.url));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
            Value::Object(obj)
        }
    }
}

/// Build the `{"mcpServers": {...}}` document from `spec.mcps`.
fn build_mcp_json(mcps: &[McpRef]) -> Result<Value> {
    let mut servers: BTreeMap<String, Value> = BTreeMap::new();
    for mcp in mcps {
        match mcp {
            McpRef::Catalog(server) | McpRef::Inline(server) => {
                servers.insert(server.id.clone(), server_json(server));
            }
            McpRef::InProcess(_) => {
                bail!("in-process MCP not supported in CLI/passthrough mode");
            }
        }
    }
    Ok(json!({ "mcpServers": servers }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpServer, McpTransport};
    use crate::spec::{ConfigStrategy, McpRef, Policy, SkillRef};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn write_skill(dir: &Path, id: &str) -> PathBuf {
        let skill_dir = dir.join(id);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {id}\ndescription: test skill\n---\nBody."),
        )
        .unwrap();
        skill_dir
    }

    #[test]
    fn provision_writes_mcp_json_skills_and_launch_without_touching_home() {
        // A stand-in for the user's real `$HOME`. `provision()` never reads
        // or writes an env-derived home directory (it only touches the `dir`
        // it is explicitly given), so this must stay untouched — the core
        // invariant this test protects. (We don't actually mutate the
        // process's `HOME` var here: `std::env::set_var` requires `unsafe`
        // as of edition 2024, and this crate forbids unsafe code; asserting
        // the fake dir stays empty is sufficient since nothing in the
        // provisioner ever consults `HOME`.)
        let fake_home = tempfile::TempDir::new().unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");

        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            path: skill_path,
        });
        spec.mcps.push(McpRef::Catalog(McpServer {
            id: "postgres".to_string(),
            transport: McpTransport::Stdio,
            command: Some("postgres-mcp".to_string()),
            args: vec!["--flag".to_string()],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
        }));
        spec.mcps.push(McpRef::Inline(McpServer {
            id: "docs".to_string(),
            transport: McpTransport::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some("https://example.com/mcp/".to_string()),
            headers: BTreeMap::new(),
        }));

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        // mcp.json exists and has the right shape.
        let mcp_json_path = config_dir.path().join("mcp.json");
        assert!(mcp_json_path.exists());
        let mcp_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        let servers = mcp_json.get("mcpServers").unwrap().as_object().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(
            servers["postgres"]["command"].as_str(),
            Some("postgres-mcp")
        );
        assert_eq!(
            servers["postgres"]["args"].as_array().unwrap().len(),
            1
        );
        assert_eq!(servers["docs"]["type"].as_str(), Some("http"));
        assert_eq!(
            servers["docs"]["url"].as_str(),
            Some("https://example.com/mcp/")
        );

        // skill copied.
        let skill_md = config_dir.path().join("skills/my-skill/SKILL.md");
        assert!(skill_md.exists());

        // launch shape.
        assert!(launch.args.contains(&"--strict-mcp-config".to_string()));
        assert!(launch.args.contains(&"--mcp-config".to_string()));
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &config_dir.path().display().to_string()));
        assert!(launch.env_remove.contains(&"CLAUDECODE".to_string()));

        // Invariant: nothing written under the stand-in home dir.
        let home_entries: Vec<_> = std::fs::read_dir(fake_home.path())
            .unwrap()
            .collect();
        assert!(
            home_entries.is_empty(),
            "expected no writes under the fake home dir, found: {home_entries:?}"
        );
    }

    #[test]
    fn provision_missing_skill_path_is_an_error() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "missing".to_string(),
            path: PathBuf::from("/definitely/does/not/exist/anywhere"),
        });

        let claude = Claude::new();
        let err = claude.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn provision_policy_writes_valid_settings_json() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.policy = Some(Policy {
            permission_mode: Some("restricted".to_string()),
            allow: vec!["Read".to_string()],
            ask: vec![],
            deny: vec!["Bash(rm *)".to_string()],
        });

        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        let settings_path = config_dir.path().join("settings.json");
        assert!(settings_path.exists());
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        let permissions = settings.get("permissions").unwrap();
        assert_eq!(
            permissions.get("defaultMode").unwrap().as_str(),
            Some("restricted")
        );
        assert_eq!(
            permissions.get("deny").unwrap().as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn provision_empty_mcps_still_writes_mcp_json() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        let mcp_json_path = config_dir.path().join("mcp.json");
        assert!(mcp_json_path.exists());
        let mcp_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        assert_eq!(
            mcp_json.get("mcpServers").unwrap().as_object().unwrap().len(),
            0
        );
    }

    #[test]
    fn provision_instructions_writes_claude_md() {
        use crate::spec::Instructions;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER: be helpful\nAlways ask questions".to_string()),
            prompt: None,
        });

        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        let claude_md_path = config_dir.path().join("CLAUDE.md");
        assert!(claude_md_path.exists());
        let content = std::fs::read_to_string(&claude_md_path).unwrap();
        assert!(content.contains("REMEMBER: be helpful"));
        assert!(content.contains("Always ask questions"));
        assert!(content.contains(MANAGED_BEGIN));
        assert!(content.contains(MANAGED_END));
    }

    #[test]
    fn provision_prompt_appends_to_launch_args() {
        use crate::spec::Instructions;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("say hello world".to_string()),
        });

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        assert_eq!(launch.args.last(), Some(&"say hello world".to_string()));
    }

    #[test]
    fn provision_instructions_and_prompt_both_set() {
        use crate::spec::Instructions;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER ME".to_string()),
            prompt: Some("do something".to_string()),
        });

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        let claude_md_path = config_dir.path().join("CLAUDE.md");
        assert!(claude_md_path.exists());
        let content = std::fs::read_to_string(&claude_md_path).unwrap();
        assert!(content.contains("REMEMBER ME"));

        assert_eq!(launch.args.last(), Some(&"do something".to_string()));
    }

    #[test]
    fn provision_account_base_url_helper_and_home_are_injected() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "gateway".to_string(),
            base_url: Some("https://gw/".to_string()),
            helper: Some("get-key".to_string()),
            home: Some(PathBuf::from("/tmp/acct")),
            ..Default::default()
        });

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "ANTHROPIC_BASE_URL" && v == "https://gw/"));
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "HOME" && v == "/tmp/acct"));

        let settings_path = config_dir.path().join("settings.json");
        assert!(settings_path.exists());
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(settings.get("apiKeyHelper").unwrap().as_str(), Some("get-key"));
    }

    #[test]
    fn provision_account_api_key_env_is_passed_through_without_touching_disk() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "path-account".to_string(),
            api_key_env: Some("PATH".to_string()),
            ..Default::default()
        });

        let expected = std::env::var("PATH").expect("PATH should be set in the test environment");

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "ANTHROPIC_API_KEY" && v == &expected));

        // No-secret-on-disk invariant: walk the whole ephemeral dir and
        // confirm the secret value never landed in any file `am` wrote.
        for entry in walkdir::WalkDir::new(config_dir.path())
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let content = std::fs::read_to_string(entry.path()).unwrap_or_default();
            assert!(
                !content.contains(&expected),
                "secret value leaked into {}",
                entry.path().display()
            );
        }
    }

    #[test]
    fn provision_structured_io_builds_headless_argv_without_positional_prompt() {
        use crate::spec::Instructions;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("say hello world".to_string()),
        });

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        assert!(launch.args.contains(&"-p".to_string()));
        assert!(launch.args.contains(&"--input-format".to_string()));
        assert!(launch.args.contains(&"stream-json".to_string()));
        assert!(launch.args.contains(&"--output-format".to_string()));
        assert!(launch.args.contains(&"--verbose".to_string()));
        assert!(launch.args.contains(&"--permission-mode".to_string()));
        assert!(launch.args.contains(&"bypassPermissions".to_string()));
        assert!(launch.args.contains(&"--disallowedTools".to_string()));
        assert!(launch.args.contains(&"AskUserQuestion".to_string()));
        // The prompt is delivered as NDJSON on stdin by the bridge, not
        // appended as a positional argument.
        assert!(!launch.args.contains(&"say hello world".to_string()));
    }

    #[test]
    fn provision_passthrough_io_does_not_build_headless_argv() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        // spec.io defaults to Passthrough.

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        assert!(!launch.args.contains(&"-p".to_string()));
        assert!(!launch.args.contains(&"--input-format".to_string()));
        assert!(!launch.args.contains(&"--permission-mode".to_string()));
        assert!(!launch.args.contains(&"--disallowedTools".to_string()));
        // The mcp-config plumbing stays present in both modes.
        assert!(launch.args.contains(&"--mcp-config".to_string()));
        assert!(launch.args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn provision_account_unset_api_key_env_is_an_error_naming_the_var() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "broken".to_string(),
            api_key_env: Some("__AM_DEFINITELY_UNSET_VAR__".to_string()),
            ..Default::default()
        });

        let claude = Claude::new();
        let err = claude.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("__AM_DEFINITELY_UNSET_VAR__"));
    }
}
