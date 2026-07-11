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
use crate::spec::{HookRef, McpRef, RunSpec};
use crate::Result;

use super::{ConfigAnchor, Harness, IoSupport, Launch, Relocate, SeedFile};

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

    /// Class A: `CLAUDE_CONFIG_DIR` relocates the entire config — credentials
    /// and `.claude.json` included (verified against Claude Code 2.1.206) — so
    /// a captured login is the two files below, seeded into the ephemeral dir
    /// while the real `HOME` stays intact. See `_docs/target/profiles.md` §5.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: vec![("CLAUDE_CONFIG_DIR".to_string(), Relocate::All)],
            login_seed: vec![
                SeedFile::new(".claude/.credentials.json", ".credentials.json"),
                SeedFile::new(".claude.json", ".claude.json"),
            ],
            requires_home_relocation: false,
        }
    }

    /// Claude Code exposes no machine-readable model-list command (only the
    /// interactive `/model` TUI picker and the alias hints in `claude --help`),
    /// so this is a **curated static list** of the aliases `--model` accepts —
    /// verified against `claude --help`. A full id (e.g. `claude-opus-4-8`,
    /// `claude-sonnet-5`) also works. The default when `--model` is omitted is
    /// the `model` key in `~/.claude/settings.json`, so no entry is marked
    /// default here. See `_docs/harness/claude-code.md`
    /// §"Model discovery & selection".
    fn discover_models(&self) -> Result<Vec<super::ModelInfo>> {
        Ok(vec![
            super::ModelInfo::new("opus").with_description("most capable"),
            super::ModelInfo::new("sonnet").with_description("balanced"),
            super::ModelInfo::new("haiku").with_description("fastest"),
            super::ModelInfo::new("fable").with_description("Fable family"),
        ])
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
        // 1b. MCP-as-skill: latent SKILL.md pointers (stepping stone; see
        // harness::write_mcp_as_skill_pointers's doc). No-op when
        // spec.mcp_as_skill is empty.
        super::write_mcp_as_skill_pointers(spec, &skills_dir)?;

        // 2. MCP: always write <dir>/mcp.json, even if empty, so
        // --strict-mcp-config yields a fully-controlled server set.
        let mcp_json = build_mcp_json(&spec.mcps)?;
        let mcp_path = dir.join("mcp.json");
        std::fs::write(&mcp_path, serde_json::to_string_pretty(&mcp_json)?)
            .with_context(|| format!("writing {}", mcp_path.display()))?;

        // 3. Policy + account helper + hooks: <dir>/settings.json, written
        // when any of a policy, an account helper, or hooks is present.
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
        if !spec.hooks.is_empty() {
            settings_obj.insert("hooks".to_string(), build_hooks_json(&spec.hooks));
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
        // Model selection: `--model <id>` works in both passthrough and
        // structured invocation. Only added when a model is set, so runs
        // without `--model` keep byte-identical argv.
        if let Some(model) = &spec.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        // Resume: `--resume <id>` works in both passthrough and headless
        // (structured) invocation, so it's appended here rather than
        // branching on `structured`. Only added when a resume id is set —
        // resumeless runs keep byte-identical argv.
        if let Some(id) = &spec.resume {
            args.push("--resume".to_string());
            args.push(id.clone());
        }
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
                // Reuse a prior `am account login` by *seeding* the ephemeral
                // config dir with that account's credentials + identity —
                // deliberately WITHOUT overriding the child's `HOME`.
                //
                // Overriding `HOME` (the previous behavior) had two fatal
                // problems: (1) Claude Code ≥2.x relocates its *entire* config
                // — `.claude.json` included, not just `.claude/.credentials.json`
                // — into `CLAUDE_CONFIG_DIR`, which points at the *empty*
                // ephemeral dir, so the HOME-resident creds were never read and
                // every run re-triggered onboarding; and (2) a per-account HOME
                // strips the user's real environment — `nvm`/`mise`/`pyenv`,
                // shell rc, PATH shims — none of which exist under a bare
                // account home. Seeding into `CLAUDE_CONFIG_DIR` fixes the auth
                // half while leaving the real HOME (and toolchain) intact.
                // The seed list is declared once in `config_anchor()`.
                super::seed_login(dir, home, &self.config_anchor().login_seed)?;
            }
        }

        Ok(Launch {
            program: "claude".to_string(),
            args,
            env,
            env_remove: ENV_HYGIENE.iter().map(|s| s.to_string()).collect(),
        })
    }

    /// Log Claude Code into `home`, capturing the resulting credential file.
    ///
    /// Verified against `claude auth --help`: `claude auth login` is a real
    /// subcommand ("Sign in to your Anthropic account"), so this launches
    /// that rather than the bare interactive `/login` fallback.
    ///
    /// HOME relocation moves the whole `~/.claude` tree (creds +
    /// `~/.claude.json`) into the capture home; running login with the OS
    /// keychain unreachable (no real `HOME`) forces the plaintext
    /// `.credentials.json` (no documented file-storage knob). Deliberately
    /// does NOT set `CLAUDE_CONFIG_DIR` here — we want the default
    /// HOME-relative layout (`<home>/.claude/.credentials.json`,
    /// `<home>/.claude.json`) so the reuse path can find and seed those files:
    /// `provision()` above copies them into the ephemeral `CLAUDE_CONFIG_DIR`
    /// (via [`super::seed_login`] driven by [`Claude::config_anchor`]) rather
    /// than relocating the child's `HOME`.
    fn login(&self, home: &Path) -> Result<super::LoginPlan> {
        let env = vec![("HOME".to_string(), home.display().to_string())];
        let args = vec!["auth".to_string(), "login".to_string()];
        Ok(super::LoginPlan {
            launch: Launch {
                program: "claude".to_string(),
                args,
                env,
                env_remove: ENV_HYGIENE.iter().map(|s| s.to_string()).collect(),
            },
            credential_files: vec![
                std::path::PathBuf::from(".claude/.credentials.json"), // required
                std::path::PathBuf::from(".claude.json"),              // optional metadata
            ],
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

/// Build the `settings.json` `"hooks"` object: grouped by native event name,
/// each event's array holding one `{"matcher": …, "hooks": [{"type": "command",
/// "command": …}]}` entry per [`HookRef`] in that event. The `"matcher"` key
/// is included only when the hook carries one — events like `UserPromptSubmit`
/// / `Stop` take no matcher.
fn build_hooks_json(hooks: &[HookRef]) -> Value {
    let mut by_event: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for hook in hooks {
        let mut entry = serde_json::Map::new();
        if let Some(matcher) = &hook.matcher {
            entry.insert("matcher".to_string(), json!(matcher));
        }
        entry.insert(
            "hooks".to_string(),
            json!([{ "type": "command", "command": hook.command }]),
        );
        by_event
            .entry(hook.event.clone())
            .or_default()
            .push(Value::Object(entry));
    }
    json!(by_event)
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
    fn provision_hooks_writes_settings_json_hooks_object() {
        use crate::spec::HookRef;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.policy = Some(Policy {
            permission_mode: Some("restricted".to_string()),
            allow: vec![],
            ask: vec![],
            deny: vec![],
        });
        spec.hooks.push(HookRef {
            id: "notify".to_string(),
            event: "PreToolUse".to_string(),
            command: "notify-send hi".to_string(),
            matcher: Some("Bash".to_string()),
        });
        spec.hooks.push(HookRef {
            id: "on-stop".to_string(),
            event: "Stop".to_string(),
            command: "echo done".to_string(),
            matcher: None,
        });

        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        let settings_path = config_dir.path().join("settings.json");
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();

        // existing keys survive alongside hooks.
        assert_eq!(
            settings["permissions"]["defaultMode"].as_str(),
            Some("restricted")
        );

        let pre_tool_use = &settings["hooks"]["PreToolUse"][0];
        assert_eq!(pre_tool_use["matcher"].as_str(), Some("Bash"));
        assert_eq!(
            pre_tool_use["hooks"][0]["command"].as_str(),
            Some("notify-send hi")
        );
        assert_eq!(pre_tool_use["hooks"][0]["type"].as_str(), Some("command"));

        let stop = &settings["hooks"]["Stop"][0];
        assert!(stop.get("matcher").is_none());
        assert_eq!(stop["hooks"][0]["command"].as_str(), Some("echo done"));
    }

    #[test]
    fn provision_no_hooks_omits_hooks_key_and_matches_prior_output() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.policy = Some(Policy {
            permission_mode: Some("restricted".to_string()),
            allow: vec![],
            ask: vec![],
            deny: vec![],
        });

        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        let settings_path = config_dir.path().join("settings.json");
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert!(settings.get("hooks").is_none());
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
    fn provision_account_seeds_login_into_config_dir_without_touching_home() {
        use crate::account::Account;

        // A persistent per-account "home" holding a captured login, laid out
        // exactly as `login()` writes it: `<home>/.claude/.credentials.json`
        // and `<home>/.claude.json`.
        let account_home = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(account_home.path().join(".claude")).unwrap();
        std::fs::write(
            account_home.path().join(".claude").join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"tok"}}"#,
        )
        .unwrap();
        std::fs::write(
            account_home.path().join(".claude.json"),
            r#"{"hasCompletedOnboarding":true}"#,
        )
        .unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "gateway".to_string(),
            base_url: Some("https://gw/".to_string()),
            helper: Some("get-key".to_string()),
            home: Some(account_home.path().to_path_buf()),
            ..Default::default()
        });

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        // base_url + apiKeyHelper still wired as before.
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "ANTHROPIC_BASE_URL" && v == "https://gw/"));
        let settings_path = config_dir.path().join("settings.json");
        assert!(settings_path.exists());
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(settings.get("apiKeyHelper").unwrap().as_str(), Some("get-key"));

        // The captured login is seeded INTO the ephemeral config dir...
        let seeded_creds = config_dir.path().join(".credentials.json");
        let seeded_json = config_dir.path().join(".claude.json");
        assert!(seeded_creds.exists(), "credentials should be seeded into CLAUDE_CONFIG_DIR");
        assert!(seeded_json.exists(), ".claude.json should be seeded into CLAUDE_CONFIG_DIR");
        assert!(std::fs::read_to_string(&seeded_creds).unwrap().contains("claudeAiOauth"));

        // ...and the child's HOME is left untouched, so the user's real
        // toolchain (nvm/mise/pyenv, shell rc, PATH shims) still resolves.
        assert!(
            !launch.env.iter().any(|(k, _)| k == "HOME"),
            "HOME must not be overridden by a `home` account: {:?}",
            launch.env
        );
    }

    #[test]
    fn provision_account_with_missing_home_files_still_launches() {
        use crate::account::Account;

        // A `home` that exists but has no captured login yet: seeding is a
        // no-op, provisioning still succeeds (reference-only / partial account).
        let account_home = tempfile::TempDir::new().unwrap();
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "empty-home".to_string(),
            home: Some(account_home.path().to_path_buf()),
            ..Default::default()
        });

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();
        assert!(!config_dir.path().join(".credentials.json").exists());
        assert!(!launch.env.iter().any(|(k, _)| k == "HOME"));
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
    fn provision_resume_appends_resume_flag() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.resume = Some("abc".to_string());

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        let resume_idx = launch
            .args
            .iter()
            .position(|a| a == "--resume")
            .expect("--resume present");
        assert_eq!(launch.args.get(resume_idx + 1), Some(&"abc".to_string()));
    }

    #[test]
    fn provision_no_resume_omits_resume_flag() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));

        let claude = Claude::new();
        let launch = claude.provision(&spec, config_dir.path()).unwrap();

        assert!(!launch.args.contains(&"--resume".to_string()));
    }

    #[test]
    fn provision_mcp_as_skill_writes_skill_md_and_keeps_mcp_injected() {
        use crate::spec::McpAsSkill;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.mcps.push(McpRef::Catalog(McpServer {
            id: "postgres".to_string(),
            transport: McpTransport::Stdio,
            command: Some("postgres-mcp".to_string()),
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
        }));
        spec.mcp_as_skill.push(McpAsSkill {
            id: "postgres".to_string(),
            summary: Some("Query a DB.".to_string()),
        });

        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        // The generated SKILL.md pointer exists and carries the summary.
        let skill_md_path = config_dir.path().join("skills/postgres/SKILL.md");
        assert!(skill_md_path.exists());
        let content = std::fs::read_to_string(&skill_md_path).unwrap();
        assert!(content.contains("name: postgres"));
        assert!(content.contains("description: Query a DB."));

        // Invariant: the MCP stays injected as normal — this is a stepping
        // stone, not a replacement.
        let mcp_json_path = config_dir.path().join("mcp.json");
        let mcp_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        assert!(mcp_json["mcpServers"]["postgres"].is_object());
    }

    #[test]
    fn provision_no_mcp_as_skill_writes_no_skills_dir_entries() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));

        let claude = Claude::new();
        claude.provision(&spec, config_dir.path()).unwrap();

        // Byte-identical-config invariant: no mcp_as_skill entries means no
        // skills dir is created at all.
        assert!(!config_dir.path().join("skills").exists());
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

    #[test]
    fn provision_injects_model_flag_when_set() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.model = Some("sonnet".to_string());

        let launch = Claude::new().provision(&spec, config_dir.path()).unwrap();

        // `--model sonnet` appears as an adjacent pair in argv.
        let pair = launch
            .args
            .windows(2)
            .any(|w| w[0] == "--model" && w[1] == "sonnet");
        assert!(pair, "expected `--model sonnet` in argv: {:?}", launch.args);
    }

    #[test]
    fn provision_without_model_has_no_model_flag() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());

        let launch = Claude::new().provision(&spec, config_dir.path()).unwrap();
        assert!(
            !launch.args.iter().any(|a| a == "--model"),
            "no --model expected when spec.model is None: {:?}",
            launch.args
        );
    }

    #[test]
    fn login_points_home_at_capture_dir_and_names_credentials_file() {
        let home = tempfile::TempDir::new().unwrap();

        let plan = Claude::new().login(home.path()).unwrap();

        assert!(plan
            .launch
            .env
            .iter()
            .any(|(k, v)| k == "HOME" && v == &home.path().display().to_string()));
        assert!(!plan.credential_files.is_empty());
        assert!(plan.credential_files[0]
            .to_str()
            .unwrap()
            .ends_with(".credentials.json"));
    }

    #[test]
    fn discover_models_lists_curated_aliases() {
        let models = Claude::new().discover_models().unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"opus") && ids.contains(&"sonnet") && ids.contains(&"haiku"));
    }
}
