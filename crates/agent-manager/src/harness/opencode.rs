//! opencode provisioner.
//!
//! Transcribes `_docs/harness/opencode.md` (esp. "On-disk layout", "MCP servers",
//! "Skills", "Permissions", "Orchestration / headless invocation") into a [`Harness`] impl.
//!
//! The "dual-env" bridge: opencode's config (skills, MCP, memory) lives under
//! `OPENCODE_CONFIG_DIR` (a dir) and `OPENCODE_CONFIG` (a JSON file). Its
//! credential store (`opencode/auth.json`) lives under the XDG data dir, which
//! `XDG_DATA_HOME` relocates (verified against opencode 1.17.18 — see
//! `config_anchor`). Pointing both `OPENCODE_CONFIG_DIR` and `XDG_DATA_HOME` at
//! the ephemeral dir means a captured login can be *seeded* in (Class A-clean)
//! without ever relocating the child's `HOME` — leaving the user's real
//! toolchain intact. This mirrors Claude Code, unlike Codex which unifies
//! everything under a single `$CODEX_HOME`.

use std::path::Path;

use anyhow::{bail, Context};
use serde_json::{json, Value};

use crate::config::{McpServer, McpTransport};
use crate::spec::{McpRef, RunSpec, IoModes};
use crate::Result;

use super::{ConfigAnchor, Harness, IoSupport, Launch, Relocate, SeedFile};

/// The opencode harness provisioner.
#[derive(Debug, Clone, Default)]
pub struct Opencode;

impl Opencode {
    /// Construct the opencode harness descriptor.
    pub fn new() -> Self {
        Opencode
    }
}

impl Harness for Opencode {
    fn id(&self) -> crate::spec::HarnessId {
        "opencode".to_string()
    }

    fn display_name(&self) -> &str {
        "opencode"
    }

    fn command(&self) -> &str {
        "opencode"
    }

    fn aliases(&self) -> &[&str] {
        &["opencode"]
    }

    fn io_support(&self) -> IoSupport {
        IoSupport {
            passthrough: true,
            structured: true,
        }
    }

    /// Class A-clean: `OPENCODE_CONFIG_DIR` relocates the config tier and
    /// `XDG_DATA_HOME` relocates the data/credential tier — opencode reads its
    /// auth store from `$XDG_DATA_HOME/opencode/auth.json` (verified
    /// empirically against opencode 1.17.18: `opencode auth list` reads that
    /// path and it overrides the HOME-relative `~/.local/share/opencode/auth.json`
    /// default). So a captured login is a single file seeded into the ephemeral
    /// dir while the real `HOME` (and the user's toolchain) stays intact — no
    /// HOME relocation needed. Resolves `_docs/profiles.md` open
    /// decision B-1 as Class A-clean.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: vec![
                ("OPENCODE_CONFIG_DIR".to_string(), Relocate::Config),
                ("XDG_DATA_HOME".to_string(), Relocate::Data),
            ],
            login_seed: vec![SeedFile::new(
                ".local/share/opencode/auth.json",
                "opencode/auth.json",
            )],
            requires_home_relocation: false,
        }
    }

    /// opencode lists the models available to the configured providers via
    /// `opencode models`, one `provider/model-id` per line. We shell out to it
    /// (it uses the ambient login/config) and take each non-empty line as an id.
    fn discover_models(&self) -> Result<Vec<super::ModelInfo>> {
        let output = std::process::Command::new("opencode")
            .arg("models")
            .output()
            .with_context(|| "running `opencode models` (is the opencode binary on PATH?)")?;
        if !output.status.success() {
            anyhow::bail!(
                "`opencode models` failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let out: Vec<super::ModelInfo> = stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(super::ModelInfo::new)
            .collect();
        Ok(out)
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // Ensure the target directory exists.
        std::fs::create_dir_all(dir)
            .with_context(|| format!("creating {}", dir.display()))?;

        // 1. Build opencode.json (always written, even with zero MCP servers).
        let opencode_json = build_opencode_json(spec, dir)?;
        let config_json_path = dir.join("opencode.json");
        std::fs::write(&config_json_path, opencode_json)
            .with_context(|| format!("writing {}", config_json_path.display()))?;

        // 2. Skills: copy each skill folder into <dir>/skills/<id>/.
        let skills_dir = dir.join("skills");
        for skill in &spec.skills {
            let dest = skills_dir.join(&skill.id);
            skill
                .source
                .materialize(&dest, crate::source::LinkMode::Copy, true)
                .with_context(|| format!("copying skill '{}' into {}", skill.id, dest.display()))?;
        }
        // 2b. MCP-as-skill: latent SKILL.md pointers (stepping stone; see
        // harness::write_mcp_as_skill_pointers's doc). No-op when
        // spec.mcp_as_skill is empty.
        super::write_mcp_as_skill_pointers(spec, &skills_dir)?;

        // Hooks: opencode has no documented native hook slot (unlike Claude
        // Code's `settings.json` `hooks` or Codex's `hooks.json` /
        // `[[hooks.<Event>]]`), so we don't invent one here. If `spec.hooks`
        // is non-empty this is simply a no-op for opencode — not an error —
        // since selecting a hook that a particular harness can't render is a
        // fidelity gap, not a user mistake.

        // 3. Instructions: write <dir>/AGENTS.md if instructions are present.
        if let Some(instructions) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
            let agents_md_path = dir.join("AGENTS.md");
            std::fs::write(&agents_md_path, instructions)
                .with_context(|| format!("writing {}", agents_md_path.display()))?;
        }

        // 4. Build the launch: different argv for structured vs passthrough.
        let args = match spec.io {
            IoModes::Structured => {
                // Structured mode: `opencode run --format json --dangerously-skip-permissions [args...] [prompt]`
                let mut structured_args = vec![
                    "run".to_string(),
                    "--format".to_string(),
                    "json".to_string(),
                    "--dangerously-skip-permissions".to_string(),
                ];
                // Model selection: `--model <provider/model-id>`. Only added
                // when set, so runs without `--model` keep byte-identical argv.
                if let Some(model) = &spec.model {
                    structured_args.push("--model".to_string());
                    structured_args.push(model.clone());
                }
                // Resume: `--session <id>` is only meaningful for the
                // structured `opencode run` form; only added when a resume
                // id is set, so resumeless runs keep byte-identical argv.
                if let Some(id) = &spec.resume {
                    structured_args.push("--session".to_string());
                    structured_args.push(id.clone());
                }
                structured_args.extend(spec.passthrough_args.clone());
                if let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
                    structured_args.push(prompt.clone());
                }
                structured_args
            }
            IoModes::Passthrough => {
                // Passthrough mode: just the original args + prompt (current
                // behavior). No CLI resume flag exists for interactive
                // opencode, so `spec.resume` is intentionally ignored here.
                let mut passthrough_args = spec.passthrough_args.clone();
                // Model selection: `--model <provider/model-id>`.
                if let Some(model) = &spec.model {
                    passthrough_args.push("--model".to_string());
                    passthrough_args.push(model.clone());
                }
                if let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
                    passthrough_args.push(prompt.clone());
                }
                passthrough_args
            }
        };

        // 5. Account: inject credential *references* into the child's env.
        // `am`'s account store never holds secret material — only env-var NAMES,
        // a base URL, and/or a home dir path. The only place a secret value is
        // ever touched is the transient `std::env::var` read below; it lands
        // in `Launch.env` (in-memory, passed to the child process) and is never
        // written to disk.
        let mut env = vec![
            ("OPENCODE_CONFIG".to_string(), config_json_path.display().to_string()),
            ("OPENCODE_CONFIG_DIR".to_string(), dir.display().to_string()),
            // Relocate the data/credential tier into the same ephemeral dir:
            // opencode reads its auth store from `$XDG_DATA_HOME/opencode/auth.json`
            // (verified — see `config_anchor`), so seeding a captured login there
            // needs no HOME relocation.
            ("XDG_DATA_HOME".to_string(), dir.display().to_string()),
        ];
        if let Some(account) = &spec.account {
            // opencode is provider-agnostic. We don't know which provider
            // (Anthropic, OpenAI, Google, etc.) the account uses, so we set
            // both ANTHROPIC_API_KEY and OPENAI_API_KEY (harmless extra env;
            // opencode uses whichever provider is configured).
            // TODO(P2+): provider-specific account env.
            if let Some(name) = &account.api_key_env {
                let value = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "account '{}' references env var '{}' which is not set",
                        account.id,
                        name
                    )
                })?;
                env.push(("ANTHROPIC_API_KEY".to_string(), value.clone()));
                env.push(("OPENAI_API_KEY".to_string(), value));
            } else if let Some(name) = &account.auth_token_env {
                let value = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "account '{}' references env var '{}' which is not set",
                        account.id,
                        name
                    )
                })?;
                env.push(("ANTHROPIC_API_KEY".to_string(), value.clone()));
                env.push(("OPENAI_API_KEY".to_string(), value));
            }
            // TODO(P2+): base_url → provider.options.baseURL config; opencode
            // uses provider-specific config, not a single env var.
            if let Some(login) = spec.account_login.clone().or_else(|| account.home.clone().map(crate::source::Source::Dir)) {
                // Reuse a prior `am account login` by *seeding* the captured
                // auth store into the relocated data dir
                // (`$XDG_DATA_HOME/opencode/auth.json`, i.e. `dir/opencode/auth.json`)
                // — deliberately WITHOUT overriding the child's `HOME`. Since
                // `XDG_DATA_HOME` (set above) relocates opencode's data/credential
                // tier, the seeded auth.json resolves without stripping the user's
                // real toolchain (nvm/mise/pyenv, shell rc, PATH shims). The seed
                // list is declared once in `config_anchor()`.
                super::seed_login(dir, &login, &self.config_anchor().login_seed)?;
            }
        }

        Ok(Launch {
            program: "opencode".to_string(),
            args,
            env,
            env_remove: Vec::new(),
        })
    }

    /// Log opencode into `home`, capturing the resulting `auth.json`.
    ///
    /// Per opencode.md "Credential capture & reuse": `~/.local/share/opencode/auth.json`
    /// is the sole auth store and is **always plaintext** (no keychain, so no
    /// force-file-storage knob is needed here, unlike Claude Code/Codex).
    /// Login relocates `HOME` to the capture home so the default
    /// HOME-relative layout (`<home>/.local/share/opencode/auth.json`) is
    /// written where the reuse path can find it: `provision()` above *seeds*
    /// that file into the ephemeral data dir (`$XDG_DATA_HOME/opencode/auth.json`,
    /// via [`super::seed_login`] driven by [`Opencode::config_anchor`]) rather
    /// than relocating the child's `HOME`. Deliberately does NOT set
    /// `OPENCODE_CONFIG`/`OPENCODE_CONFIG_DIR`/`XDG_DATA_HOME` — login only needs
    /// the auth store at the default HOME-relative path; the reuse path injects
    /// config separately.
    ///
    /// Login command: `opencode auth login` (interactive TUI: pick provider,
    /// paste key or complete OAuth). Not verified against the installed
    /// binary in this environment (opencode is not on `PATH` here) — this
    /// matches the documented command in opencode.md and should be
    /// re-verified against `opencode auth --help` when the binary is
    /// available.
    fn login(&self, home: &Path) -> Result<super::LoginPlan> {
        let env = vec![("HOME".to_string(), home.display().to_string())];
        let args = vec!["auth".to_string(), "login".to_string()];
        Ok(super::LoginPlan {
            launch: Launch {
                program: "opencode".to_string(),
                args,
                env,
                env_remove: Vec::new(),
            },
            credential_files: vec![std::path::PathBuf::from(
                ".local/share/opencode/auth.json",
            )],
        })
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        Ok(Box::new(crate::io::opencode::OpencodeBridge::new(child)?))
    }
}

/// Render one [`McpServer`] into the JSON shape opencode's `opencode.json`
/// expects, keyed by transport.
fn mcp_server_json(server: &McpServer) -> Value {
    match server.transport {
        McpTransport::Stdio => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), json!("local"));
            // command is an array: the server command first, then each args element.
            let mut command_array = vec![json!(server.command.as_deref().unwrap_or(""))];
            command_array.extend(server.args.iter().map(|arg| json!(arg)));
            obj.insert("command".to_string(), Value::Array(command_array));
            if !server.env.is_empty() {
                obj.insert("environment".to_string(), json!(server.env));
            }
            obj.insert("enabled".to_string(), json!(true));
            Value::Object(obj)
        }
        McpTransport::Http | McpTransport::Sse => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), json!("remote"));
            obj.insert("url".to_string(), json!(server.url.as_deref().unwrap_or("")));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
            obj.insert("enabled".to_string(), json!(true));
            Value::Object(obj)
        }
    }
}

/// Build the full `opencode.json` document from `spec`.
fn build_opencode_json(spec: &RunSpec, dir: &Path) -> Result<String> {
    let mut root = serde_json::Map::new();

    // Always include the schema.
    root.insert(
        "$schema".to_string(),
        json!("https://opencode.ai/config.json"),
    );

    // 1. MCP servers (may be empty).
    let mut mcp_map = serde_json::Map::new();
    for mcp in &spec.mcps {
        match mcp {
            McpRef::Catalog(server) | McpRef::Inline(server) => {
                mcp_map.insert(server.id.clone(), mcp_server_json(server));
            }
            McpRef::InProcess(_) => {
                bail!("in-process MCP not supported in passthrough mode");
            }
        }
    }
    root.insert("mcp".to_string(), Value::Object(mcp_map));

    // 2. Instructions: if present, write AGENTS.md and add to the JSON.
    if spec.initial.as_ref().and_then(|i| i.instructions.as_ref()).is_some() {
        // The path is already written by provision(); reference it here.
        let agents_md_path = dir.join("AGENTS.md");
        root.insert(
            "instructions".to_string(),
            json!(vec![agents_md_path.display().to_string()]),
        );
    }

    // 3. Permissions: best-effort mapping of policy.
    if let Some(policy) = &spec.policy {
        let mut permission = serde_json::Map::new();
        for rule in &policy.deny {
            permission.insert(rule.clone(), json!("deny"));
        }
        for rule in &policy.ask {
            permission.insert(rule.clone(), json!("ask"));
        }
        for rule in &policy.allow {
            permission.insert(rule.clone(), json!("allow"));
        }
        if !permission.is_empty() {
            root.insert("permission".to_string(), Value::Object(permission));
        }
        // Add a comment explaining best-effort translation.
        root.insert(
            "# best-effort: full Claude-rule→opencode-pattern translation is P2+".to_string(),
            json!(""),
        );
    }

    serde_json::to_string_pretty(&Value::Object(root))
        .context("serializing opencode.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ConfigStrategy, Instructions, McpRef, Policy, SkillRef};
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
    fn provision_writes_opencode_json_skills_agents_md_and_launch_without_touching_home() {
        // A stand-in for the user's real `$HOME`. `provision()` never reads
        // or writes an env-derived home directory (it only touches the `dir`
        // it is explicitly given), so this must stay untouched.
        let fake_home = tempfile::TempDir::new().unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");

        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            source: crate::source::Source::Dir(skill_path),
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
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER: be helpful".to_string()),
            prompt: Some("say hello world".to_string()),
        });

        let opencode = Opencode::new();
        let launch = opencode.provision(&spec, config_dir.path()).unwrap();

        // opencode.json exists and has the right structure.
        let config_json_path = config_dir.path().join("opencode.json");
        assert!(config_json_path.exists());
        let config_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_json_path).unwrap()).unwrap();

        // Check schema.
        assert_eq!(
            config_json.get("$schema").unwrap().as_str(),
            Some("https://opencode.ai/config.json")
        );

        // Check MCP servers.
        let mcp = config_json.get("mcp").unwrap().as_object().unwrap();
        assert_eq!(mcp.len(), 2);
        // stdio MCP: type should be "local", command should be an array.
        assert_eq!(
            mcp.get("postgres").unwrap().get("type").unwrap().as_str(),
            Some("local")
        );
        let postgres_cmd = mcp
            .get("postgres")
            .unwrap()
            .get("command")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(postgres_cmd[0].as_str(), Some("postgres-mcp"));
        assert_eq!(postgres_cmd[1].as_str(), Some("--flag"));
        // HTTP MCP: type should be "remote", url should be present.
        assert_eq!(
            mcp.get("docs").unwrap().get("type").unwrap().as_str(),
            Some("remote")
        );
        assert_eq!(
            mcp.get("docs").unwrap().get("url").unwrap().as_str(),
            Some("https://example.com/mcp/")
        );

        // Skill copied.
        let skill_md = config_dir.path().join("skills/my-skill/SKILL.md");
        assert!(skill_md.exists());

        // AGENTS.md contains the instructions.
        let agents_md_path = config_dir.path().join("AGENTS.md");
        assert!(agents_md_path.exists());
        let agents_md = std::fs::read_to_string(&agents_md_path).unwrap();
        assert!(agents_md.contains("REMEMBER: be helpful"));

        // Check instructions array in JSON.
        let instructions = config_json
            .get("instructions")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(instructions.len(), 1);
        assert!(instructions[0].as_str().unwrap().ends_with("AGENTS.md"));

        // Launch shape.
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "OPENCODE_CONFIG" && v.ends_with("opencode.json")));
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "OPENCODE_CONFIG_DIR" && v == &config_dir.path().display().to_string()));
        assert_eq!(launch.args.last(), Some(&"say hello world".to_string()));

        // Invariant: nothing written under the stand-in home dir.
        let home_entries: Vec<_> = std::fs::read_dir(fake_home.path()).unwrap().collect();
        assert!(
            home_entries.is_empty(),
            "expected no writes under the fake home dir, found: {home_entries:?}"
        );
    }

    #[test]
    fn provision_missing_skill_path_is_an_error() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "missing".to_string(),
            source: crate::source::Source::Dir(PathBuf::from("/definitely/does/not/exist/anywhere")),
        });

        let opencode = Opencode::new();
        let err = opencode.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn provision_empty_mcps_still_writes_opencode_json() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        let opencode = Opencode::new();
        opencode.provision(&spec, config_dir.path()).unwrap();

        let config_json_path = config_dir.path().join("opencode.json");
        assert!(config_json_path.exists());
        let config_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_json_path).unwrap()).unwrap();
        assert_eq!(
            config_json.get("mcp").unwrap().as_object().unwrap().len(),
            0
        );
    }

    #[test]
    fn provision_mcp_in_process_is_an_error() {
        use crate::mcp::{McpService, ToolDef};
        use crate::spec::InProcessMcpHandle;
        use std::sync::Arc;

        struct NoopService;
        impl McpService for NoopService {
            fn tools(&self) -> Vec<ToolDef> {
                Vec::new()
            }
            fn call(&self, _name: &str, _arguments: Value) -> crate::Result<Value> {
                anyhow::bail!("not implemented")
            }
        }

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.mcps.push(McpRef::InProcess(InProcessMcpHandle {
            name: "in-proc".to_string(),
            service: Arc::new(NoopService),
        }));

        let opencode = Opencode::new();
        let err = opencode.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("in-process"));
    }

    #[test]
    fn provision_account_api_key_env_maps_to_anthropic_and_openai() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "path-account".to_string(),
            api_key_env: Some("PATH".to_string()),
            ..Default::default()
        });

        let expected = std::env::var("PATH").expect("PATH should be set in the test environment");

        let opencode = Opencode::new();
        let launch = opencode.provision(&spec, config_dir.path()).unwrap();

        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "ANTHROPIC_API_KEY" && v == &expected));
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "OPENAI_API_KEY" && v == &expected));

        // No-secret-on-disk invariant.
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
    fn provision_mcp_as_skill_writes_skill_md_and_keeps_mcp_injected() {
        use crate::spec::McpAsSkill;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
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

        let opencode = Opencode::new();
        opencode.provision(&spec, config_dir.path()).unwrap();

        let skill_md_path = config_dir.path().join("skills/postgres/SKILL.md");
        assert!(skill_md_path.exists());
        let content = std::fs::read_to_string(&skill_md_path).unwrap();
        assert!(content.contains("name: postgres"));
        assert!(content.contains("description: Query a DB."));

        // Invariant: the MCP stays injected as normal in opencode.json.
        let config_json: Value = serde_json::from_str(
            &std::fs::read_to_string(config_dir.path().join("opencode.json")).unwrap(),
        )
        .unwrap();
        assert!(config_json["mcp"]["postgres"].is_object());
    }

    #[test]
    fn provision_account_unset_api_key_env_is_an_error_naming_the_var() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "broken".to_string(),
            api_key_env: Some("__AM_DEFINITELY_UNSET_VAR__".to_string()),
            ..Default::default()
        });

        let opencode = Opencode::new();
        let err = opencode.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("__AM_DEFINITELY_UNSET_VAR__"));
    }

    #[test]
    fn provision_account_seeds_auth_into_data_dir_without_touching_home() {
        use crate::account::Account;

        // A persistent per-account "home" holding a captured login, laid out
        // exactly as `login()` writes it: `<home>/.local/share/opencode/auth.json`.
        let account_home = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(account_home.path().join(".local/share/opencode")).unwrap();
        std::fs::write(
            account_home.path().join(".local/share/opencode/auth.json"),
            r#"{"anthropic":{"type":"api","key":"tok"}}"#,
        )
        .unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "private-home".to_string(),
            home: Some(account_home.path().to_path_buf()),
            ..Default::default()
        });

        let opencode = Opencode::new();
        let launch = opencode.provision(&spec, config_dir.path()).unwrap();

        // XDG_DATA_HOME relocates the data/credential tier into the ephemeral dir.
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "XDG_DATA_HOME" && v == &config_dir.path().display().to_string()));

        // The captured login is seeded INTO the ephemeral data dir at the
        // XDG-relative path opencode reads (`$XDG_DATA_HOME/opencode/auth.json`).
        let seeded_auth = config_dir.path().join("opencode/auth.json");
        assert!(
            seeded_auth.exists(),
            "auth.json should be seeded into $XDG_DATA_HOME/opencode/"
        );
        assert!(std::fs::read_to_string(&seeded_auth).unwrap().contains("anthropic"));

        // ...and the child's HOME is left untouched, so the user's real
        // toolchain (nvm/mise/pyenv, shell rc, PATH shims) still resolves.
        assert!(
            !launch.env.iter().any(|(k, _)| k == "HOME"),
            "HOME must not be overridden by a `home` account: {:?}",
            launch.env
        );

        // Config stays in the ephemeral dir, not in the account home.
        assert!(config_dir.path().join("opencode.json").exists());
    }

    #[test]
    fn provision_permissions_are_mapped() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.policy = Some(Policy {
            permission_mode: None,
            allow: vec!["bash".to_string(), "read".to_string()],
            ask: vec!["edit".to_string()],
            deny: vec!["external_directory".to_string()],
        });

        let opencode = Opencode::new();
        opencode.provision(&spec, config_dir.path()).unwrap();

        let config_json_path = config_dir.path().join("opencode.json");
        let config_json: Value =
            serde_json::from_str(&std::fs::read_to_string(&config_json_path).unwrap()).unwrap();
        let permission = config_json.get("permission").unwrap().as_object().unwrap();
        assert_eq!(permission.get("bash").unwrap().as_str(), Some("allow"));
        assert_eq!(permission.get("edit").unwrap().as_str(), Some("ask"));
        assert_eq!(
            permission.get("external_directory").unwrap().as_str(),
            Some("deny")
        );
    }

    #[test]
    fn resolve_opencode_by_id() {
        assert_eq!(super::super::resolve("opencode").unwrap().id(), "opencode");
    }

    #[test]
    fn provision_structured_mode_builds_correct_argv() {
        use crate::spec::ConfigStrategy;
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("hello world".to_string()),
        });

        let opencode = Opencode::new();
        let launch = opencode.provision(&spec, config_dir.path()).unwrap();

        // Structured mode should have "run", "--format", "json", "--dangerously-skip-permissions"
        assert!(launch.args.len() >= 4);
        assert_eq!(launch.args[0], "run");
        assert_eq!(launch.args[1], "--format");
        assert_eq!(launch.args[2], "json");
        assert_eq!(launch.args[3], "--dangerously-skip-permissions");
        // Prompt should be the final positional argument
        assert_eq!(launch.args.last(), Some(&"hello world".to_string()));
    }

    #[test]
    fn provision_structured_resume_appends_session_flag() {
        use crate::spec::ConfigStrategy;
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;
        spec.resume = Some("abc".to_string());

        let opencode = Opencode::new();
        let launch = opencode.provision(&spec, config_dir.path()).unwrap();

        let idx = launch
            .args
            .iter()
            .position(|a| a == "--session")
            .expect("--session present");
        assert_eq!(launch.args.get(idx + 1), Some(&"abc".to_string()));
    }

    #[test]
    fn provision_structured_no_resume_omits_session_flag() {
        use crate::spec::ConfigStrategy;
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;

        let opencode = Opencode::new();
        let launch = opencode.provision(&spec, config_dir.path()).unwrap();

        assert!(!launch.args.contains(&"--session".to_string()));
    }

    #[test]
    fn provision_passthrough_resume_has_no_cli_flag() {
        use crate::spec::ConfigStrategy;
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.resume = Some("abc".to_string());
        // spec.io defaults to Passthrough.

        let opencode = Opencode::new();
        let launch = opencode.provision(&spec, config_dir.path()).unwrap();

        assert!(!launch.args.contains(&"--session".to_string()));
        assert!(!launch.args.contains(&"abc".to_string()));
    }

    #[test]
    fn login_points_home_at_capture_dir_and_names_auth_json() {
        let home = tempfile::TempDir::new().unwrap();

        let plan = Opencode::new().login(home.path()).unwrap();

        assert!(plan
            .launch
            .env
            .iter()
            .any(|(k, v)| k == "HOME" && v == &home.path().display().to_string()));
        assert!(!plan.credential_files.is_empty());
        assert!(plan.credential_files[0]
            .to_str()
            .unwrap()
            .ends_with("opencode/auth.json"));
    }

    #[test]
    fn provision_passthrough_mode_builds_simple_argv() {
        use crate::spec::ConfigStrategy;
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("opencode".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Passthrough;
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("hello world".to_string()),
        });

        let opencode = Opencode::new();
        let launch = opencode.provision(&spec, config_dir.path()).unwrap();

        // Passthrough mode: no "run", "--format", etc. — just the prompt
        assert!(!launch.args.contains(&"run".to_string()));
        assert!(!launch.args.contains(&"--format".to_string()));
        assert_eq!(launch.args.last(), Some(&"hello world".to_string()));
    }
}
