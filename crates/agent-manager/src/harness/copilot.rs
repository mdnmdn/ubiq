//! GitHub Copilot CLI provisioner.
//!
//! Transcribes `_docs/harness/copilot.md`, but several load-bearing facts in
//! that doc do **not** match the real installed binary (GitHub Copilot CLI
//! 1.0.69, verified 2026-07-19 via `copilot --help`, `copilot login --help`,
//! `copilot mcp add --help`, `copilot help environment`, `copilot help
//! config`) — this file follows the verified binary, not the doc, wherever
//! they disagree, and each disagreement is called out below so the doc can be
//! corrected too. **If you're touching this file and something still looks
//! off, re-verify against `copilot --help` before trusting either this file's
//! comments or copilot.md** — that's exactly the lesson this harness taught.
//!
//! **Scope decision: CLI front-end only.** GitHub Copilot has three surfaces
//! (VS Code extension, Copilot CLI, github.com's cloud agent/code review) that
//! share a `.github/` config layout but have distinct user-profile roots and
//! processes. `am` spawns processes; VS Code's extension and github.com's
//! cloud agent aren't processes `am` can launch, so this impl targets only the
//! `copilot` binary.
//!
//! **Isolation lever — Class A, not Class C (verified, corrects an earlier
//! draft of this file and of copilot.md).** `COPILOT_HOME` **does** relocate
//! the CLI's entire config/state store — confirmed empirically: `COPILOT_HOME=
//! <dir> copilot mcp add ...` writes `<dir>/mcp-config.json` directly (not
//! `<dir>/.copilot/mcp-config.json`, and not the real `~/.copilot/`).
//! `COPILOT_HOME` **is** the `~/.copilot`-equivalent directory itself, not a
//! parent whose child is `.copilot/` — so every path below is written directly
//! under the ephemeral `dir`, with no `.copilot/` prefix. This means Copilot
//! CLI needs **no `HOME` relocation at all** — the user's real toolchain
//! (`nvm`/`mise`/`pyenv`, shell rc, PATH shims) stays intact, unlike a
//! Class-C harness (Grok). `login --help` describes the credential-store
//! fallback as "a plain text config file under `~/.copilot/`" without naming
//! it; `config.json` is carried over from copilot.md as the best-effort guess
//! (plausible, not independently confirmed — this environment authenticates
//! via the OS keychain / an env var, so no plaintext fallback file was ever
//! observed here to name for certain). Re-verify the exact filename the next
//! time a real plaintext-fallback capture is exercised.
//!
//! **Corrections to copilot.md, verified against 1.0.69:**
//! - The login command is `copilot login`, **not** `copilot auth login` — this
//!   version has no `auth` subcommand namespace at all (no `login`/`logout`/
//!   `status`/`setup-token` under `auth`; `login` is a top-level command).
//!   This was a real bug in an earlier draft of this file (reported by a user
//!   hitting `Invalid command format` at `am account login --harness copilot`).
//! - The MCP config file is `mcp-config.json`, **not** `mcp.json`; its
//!   top-level key is `mcpServers`, **not** `servers`; and its per-server
//!   schema is `{"tools": [...], "type": "local"|"http"|"sse", ...}` (stdio
//!   servers are typed `"local"`, not `"stdio"`) — verified with
//!   `copilot mcp add` under a scratch `COPILOT_HOME` for all three
//!   transports, `--env`, and `--header`.
//! - Token env var precedence is `COPILOT_GITHUB_TOKEN` > `GH_TOKEN` >
//!   `GITHUB_TOKEN` (verified via `copilot login --help` and `copilot help
//!   environment`). There is **no** `COPILOT_TOKEN` env var and **no**
//!   `copilot auth setup-token` command in this version — copilot.md's CI
//!   short-lived-token story doesn't exist in the real binary.
//! - There is **no** documented native hook slot in this CLI version — `hook`
//!   does not appear anywhere in `copilot --help` or its help topics. An
//!   earlier draft of this file wrote a speculative `hooks/am-managed.json`
//!   the real binary never reads; `spec.hooks` is now a documented no-op here
//!   (same stance as opencode's hookless slot) rather than dead plumbing.
//! - `discover_models()` **can** shell out for a real, machine-parseable model
//!   list (`copilot help config`'s `` `model`: `` settings entry is a stable
//!   quoted bullet list) — an earlier draft of this file gave up on this and
//!   returned a curated static fallback; that was premature, not a genuine
//!   doc gap.
//! - Prompt seeding differs by mode: non-interactive is `-p <text>`
//!   (`--output-format json --allow-all --no-ask-user`, as copilot.md
//!   documents); interactive is `-i <text>` (`--interactive`, "start
//!   interactive mode and automatically execute this prompt") — **not** a
//!   bare trailing positional argument (`copilot`'s parser has no such seam;
//!   feeding it one is exactly what produced the reported
//!   `Invalid command format. Did you mean: copilot -i "..."?` error for the
//!   unrelated `login` args bug above). `--model <id>` and `--resume[=<id>]`
//!   are both general top-level flags valid in either mode (`--resume` takes
//!   its value via `=`, not a separate argv token — it's an optional-value
//!   option).

use std::path::Path;

use anyhow::Context;
use serde_json::{json, Value};

use crate::config::{McpServer, McpTransport};
use crate::spec::{McpRef, RunSpec};
use crate::Result;

use super::{ConfigAnchor, Harness, IoSupport, Launch, Relocate, SeedFile};

/// The GitHub Copilot CLI harness provisioner.
#[derive(Debug, Clone, Default)]
pub struct Copilot;

impl Copilot {
    /// Construct the GitHub Copilot CLI harness descriptor.
    pub fn new() -> Self {
        Copilot
    }
}

impl Harness for Copilot {
    fn id(&self) -> crate::spec::HarnessId {
        "copilot".to_string()
    }

    fn display_name(&self) -> &str {
        "GitHub Copilot"
    }

    fn command(&self) -> &str {
        "copilot"
    }

    fn aliases(&self) -> &[&str] {
        &[]
    }

    fn io_support(&self) -> IoSupport {
        IoSupport {
            passthrough: true,
            structured: true,
        }
    }

    /// Class A: `COPILOT_HOME` relocates the CLI's entire config/state tree —
    /// verified against the installed binary (see the module doc). A captured
    /// login's `config.json` is seeded into the ephemeral dir the same way
    /// Claude/Codex do; `HOME` is never touched.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: vec![("COPILOT_HOME".to_string(), Relocate::All)],
            login_seed: vec![SeedFile::new("config.json", "config.json")],
            requires_home_relocation: false,
        }
    }

    /// `copilot help config`'s `` `model`: `` settings entry is a stable,
    /// machine-parseable quoted bullet list (verified against 1.0.69) — shell
    /// out and scrape that one block rather than falling back to a curated
    /// static list. Needs no auth/network (shown in plain `--help` text).
    fn discover_models(&self) -> Result<Vec<super::ModelInfo>> {
        let output = std::process::Command::new("copilot")
            .args(["help", "config"])
            .output()
            .with_context(|| "running `copilot help config` (is the copilot binary on PATH?)")?;
        if !output.status.success() {
            anyhow::bail!(
                "`copilot help config` failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let ids = parse_model_ids(&stdout);
        if ids.is_empty() {
            anyhow::bail!(
                "no model ids found in `copilot help config` output — its format may have \
                 changed; re-verify against the installed binary"
            );
        }
        let mut out: Vec<super::ModelInfo> = ids.into_iter().map(super::ModelInfo::new).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // `dir` IS `$COPILOT_HOME` directly (no `.copilot/` prefix — see
        // module doc), so every injected file below lands straight under it.
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        // 1. Skills: copy each skill folder into <dir>/skills/<id>/ (the CLI
        // user-tier location per copilot.md's on-disk layout table, minus the
        // `.copilot/` prefix COPILOT_HOME already relocates away).
        let skills_dir = dir.join("skills");
        for skill in &spec.skills {
            let dest = skills_dir.join(&skill.id);
            skill
                .source
                .materialize(&dest, crate::source::LinkMode::Copy, true)
                .with_context(|| format!("copying skill '{}' into {}", skill.id, dest.display()))?;
        }
        // 1b. MCP-as-skill: latent SKILL.md pointers (stepping stone; see
        // harness::write_mcp_as_skill_pointers's doc). No-op when
        // spec.mcp_as_skill is empty.
        super::write_mcp_as_skill_pointers(spec, &skills_dir)?;

        // 2. MCP: write <dir>/mcp-config.json with a top-level `mcpServers`
        // key (verified against the installed binary — NOT `mcp.json`/
        // `servers`, which is what copilot.md documents; see module doc).
        // Written only when there are servers to inject — no documented stub-
        // file requirement, mirrors Grok's "unused runs stay minimal"
        // convention.
        let mcp_map = build_mcp_servers(&spec.mcps)?;
        if !mcp_map.is_empty() {
            let mcp_json = json!({ "mcpServers": Value::Object(mcp_map) });
            let mcp_json_path = dir.join("mcp-config.json");
            std::fs::write(&mcp_json_path, serde_json::to_string_pretty(&mcp_json)?)
                .with_context(|| format!("writing {}", mcp_json_path.display()))?;
        }

        // 3. Hooks: no-op. `hook` does not appear anywhere in the installed
        // binary's `--help`/help topics (verified) — there is no native hook
        // slot to write into for this CLI version, so `spec.hooks` is a
        // documented fidelity gap here, not a user mistake (same stance as
        // opencode's hookless slot).

        // 4. Instructions: <dir>/copilot-instructions.md — per copilot.md's
        // documented global personal-instructions path (`--no-custom-
        // instructions` existing in `--help` confirms *some* default
        // instructions file is loaded; this exact path is not independently
        // re-verified, but writing it is harmless if wrong — worst case it's
        // silently unread, unlike the hooks case above).
        if let Some(instr_text) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
            let instructions_path = dir.join("copilot-instructions.md");
            std::fs::write(&instructions_path, instr_text)
                .with_context(|| format!("writing {}", instructions_path.display()))?;
        }

        // 5. Build the launch. Verified against `copilot --help`: prompt
        // seeding differs by mode (`-p` non-interactive vs `-i` interactive —
        // there is no bare positional-prompt seam); `--model`/`--resume` are
        // general flags valid in either mode; `--resume` takes its value via
        // `=` (an optional-value option, not a separate argv token).
        let structured = spec.io == crate::spec::IoModes::Structured;

        let mut args = Vec::new();
        if structured {
            if let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
                args.push("-p".to_string());
                args.push(prompt.clone());
            }
            args.push("--output-format".to_string());
            args.push("json".to_string());
            args.push("--allow-all".to_string());
            args.push("--no-ask-user".to_string());
            if let Some(model) = &spec.model {
                args.push("--model".to_string());
                args.push(model.clone());
            }
            if let Some(id) = &spec.resume {
                args.push(format!("--resume={id}"));
            }
            args.extend(spec.passthrough_args.iter().cloned());
        } else {
            args = spec.passthrough_args.clone();
            if let Some(model) = &spec.model {
                args.push("--model".to_string());
                args.push(model.clone());
            }
            if let Some(id) = &spec.resume {
                args.push(format!("--resume={id}"));
            }
            if let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
                args.push("-i".to_string());
                args.push(prompt.clone());
            }
        }

        // 6. Account: inject credential *references* into the child's env.
        // `am`'s account store never holds secret material — only env-var
        // NAMES, a base URL, a helper command, and/or a home dir path. The
        // only place a secret value is ever touched is the transient
        // `std::env::var` read below; it lands in `Launch.env` (in-memory,
        // passed to the child process) and is never written to disk.
        let mut env = vec![("COPILOT_HOME".to_string(), dir.display().to_string())];
        if let Some(account) = &spec.account {
            // `api_key_env` maps to `COPILOT_GITHUB_TOKEN` (highest
            // precedence per `copilot login --help`/`copilot help
            // environment`; accepts fine-grained PATs and OAuth tokens
            // alike). `auth_token_env` maps to `GITHUB_TOKEN` (lowest of the
            // three documented token env vars, still real and functional) —
            // if both are set, api_key_env wins (this codebase's established
            // convention). There is no `COPILOT_TOKEN` — see module doc.
            if let Some(name) = &account.api_key_env {
                let value = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "account '{}' references env var '{}' which is not set",
                        account.id,
                        name
                    )
                })?;
                env.push(("COPILOT_GITHUB_TOKEN".to_string(), value));
            } else if let Some(name) = &account.auth_token_env {
                let value = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "account '{}' references env var '{}' which is not set",
                        account.id,
                        name
                    )
                })?;
                env.push(("GITHUB_TOKEN".to_string(), value));
            }
            // `base_url` maps to `COPILOT_GH_HOST` (verified via `copilot
            // help environment`: "GitHub hostname used only by Copilot CLI
            // ... overriding GH_HOST when set" — a GitHub Enterprise Cloud
            // data-residency hostname, e.g. "mycompany.ghe.com", NOT a full
            // https:// URL despite the field's generic name).
            if let Some(base_url) = &account.base_url {
                env.push(("COPILOT_GH_HOST".to_string(), base_url.clone()));
            }

            // Reuse a prior `am account login` by *seeding* the captured
            // `config.json` into the relocated `$COPILOT_HOME`. No-op when
            // the account home holds no captured login yet. The seed list is
            // declared once in `config_anchor()`.
            if let Some(login) = spec.account_login.clone().or_else(|| account.home.clone().map(crate::source::Source::Dir)) {
                super::seed_login(dir, &login, &self.config_anchor().login_seed)?;
            }
        }

        Ok(Launch {
            program: "copilot".to_string(),
            args,
            env,
            env_remove: Vec::new(),
        })
    }

    /// Log Copilot CLI into `home`, capturing the resulting `config.json`.
    ///
    /// Verified against the installed binary (`copilot login --help`,
    /// copilot CLI 1.0.69): the command is `copilot login` — **not**
    /// `copilot auth login`, which this version rejects (`Invalid command
    /// format`; there is no `auth` subcommand namespace at all here). File
    /// storage is the default fallback when no OS credential store is
    /// reachable — no force-file-storage config write is needed here, unlike
    /// Codex's `cli_auth_credentials_store = "file"`.
    fn login(&self, home: &Path) -> Result<super::LoginPlan> {
        let env = vec![("COPILOT_HOME".to_string(), home.display().to_string())];
        let args = vec!["login".to_string()];
        Ok(super::LoginPlan {
            launch: Launch {
                program: "copilot".to_string(),
                args,
                env,
                env_remove: Vec::new(),
            },
            credential_files: vec![std::path::PathBuf::from("config.json")],
        })
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        Ok(Box::new(crate::io::copilot::CopilotBridge::new(child)?))
    }
}

/// Render one [`McpServer`] into the JSON shape `mcp-config.json`'s
/// `mcpServers` map expects — verified against the installed binary via
/// `copilot mcp add` (stdio/http/sse, `--env`, `--header`) under a scratch
/// `COPILOT_HOME`. Every entry always carries `tools`; `am` has no per-server
/// tool filter today, so this always emits `["*"]` (all tools allowed).
fn mcp_server_json(server: &McpServer) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("tools".to_string(), json!(["*"]));
    match server.transport {
        // Verified: stdio servers are typed "local", not "stdio".
        McpTransport::Stdio => {
            obj.insert("type".to_string(), json!("local"));
            if let Some(command) = &server.command {
                obj.insert("command".to_string(), json!(command));
            }
            if !server.args.is_empty() {
                obj.insert("args".to_string(), json!(server.args));
            }
            if !server.env.is_empty() {
                obj.insert("env".to_string(), json!(server.env));
            }
        }
        McpTransport::Http => {
            obj.insert("type".to_string(), json!("http"));
            obj.insert("url".to_string(), json!(server.url));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
        }
        McpTransport::Sse => {
            obj.insert("type".to_string(), json!("sse"));
            obj.insert("url".to_string(), json!(server.url));
            if !server.headers.is_empty() {
                obj.insert("headers".to_string(), json!(server.headers));
            }
        }
    }
    Value::Object(obj)
}

/// Build the `mcpServers` map from `spec.mcps`, keyed by server id.
fn build_mcp_servers(mcps: &[McpRef]) -> Result<serde_json::Map<String, Value>> {
    let mut servers = serde_json::Map::new();
    for mcp in mcps {
        match mcp {
            McpRef::Catalog(server) | McpRef::Inline(server) => {
                servers.insert(server.id.clone(), mcp_server_json(server));
            }
            McpRef::InProcess(_) => {
                anyhow::bail!("in-process MCP not supported in passthrough mode");
            }
        }
    }
    Ok(servers)
}

/// Scrape the `` `model`: `` settings entry out of `copilot help config`'s
/// output: a quoted bullet (`    - "id"`) per line, ending at the first
/// non-bullet, non-blank line after the block starts. Pure (no I/O) so it's
/// unit-testable directly against a captured snippet of real `--help` output.
fn parse_model_ids(help_text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut in_model_block = false;
    for line in help_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("`model`:") {
            in_model_block = true;
            continue;
        }
        if !in_model_block {
            continue;
        }
        if let Some(id) = trimmed.strip_prefix("- \"").and_then(|s| s.strip_suffix('"')) {
            ids.push(id.to_string());
        } else if !trimmed.is_empty() {
            break;
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ConfigStrategy, Instructions, McpRef, SkillRef};
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
    fn provision_writes_mcp_config_json_skills_and_launch_without_touching_home() {
        // Stand-in for the user's real `$HOME`. `provision()` never overrides
        // `HOME` at all for Copilot (Class A via `COPILOT_HOME`) and never
        // writes outside the `dir` it's given, so this must stay empty.
        let fake_home = tempfile::TempDir::new().unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");

        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
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

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        // mcp-config.json exists directly under `dir` (no `.copilot/`
        // prefix) with the right `mcpServers` shape.
        let mcp_json_path = config_dir.path().join("mcp-config.json");
        assert!(mcp_json_path.exists());
        let parsed: Value =
            serde_json::from_str(&std::fs::read_to_string(&mcp_json_path).unwrap()).unwrap();
        let servers = parsed.get("mcpServers").unwrap().as_object().unwrap();
        assert!(parsed.get("servers").is_none());
        assert_eq!(servers.len(), 2);
        // stdio -> "local" type + command + args + default tools.
        assert_eq!(servers["postgres"]["type"].as_str(), Some("local"));
        assert_eq!(servers["postgres"]["command"].as_str(), Some("postgres-mcp"));
        assert_eq!(servers["postgres"]["args"].as_array().unwrap().len(), 1);
        assert_eq!(servers["postgres"]["tools"].as_array().unwrap()[0].as_str(), Some("*"));
        // http remote: type + url.
        assert_eq!(servers["docs"]["type"].as_str(), Some("http"));
        assert_eq!(servers["docs"]["url"].as_str(), Some("https://example.com/mcp/"));

        // Skill copied directly under <dir>/skills.
        let skill_md = config_dir.path().join("skills/my-skill/SKILL.md");
        assert!(skill_md.exists());

        // Launch relocates COPILOT_HOME, and does NOT override HOME at all.
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "COPILOT_HOME" && v == &config_dir.path().display().to_string()));
        assert!(!launch.env.iter().any(|(k, _)| k == "HOME"));
        assert_eq!(launch.program, "copilot");

        // Invariant: nothing written under the stand-in home dir.
        let home_entries: Vec<_> = std::fs::read_dir(fake_home.path()).unwrap().collect();
        assert!(
            home_entries.is_empty(),
            "expected no writes under the fake home dir, found: {home_entries:?}"
        );
    }

    #[test]
    fn provision_empty_mcps_writes_no_mcp_config_json() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        let copilot = Copilot::new();
        copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(!config_dir.path().join("mcp-config.json").exists());
    }

    #[test]
    fn provision_missing_skill_path_is_an_error() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "missing".to_string(),
            source: crate::source::Source::Dir(PathBuf::from("/definitely/does/not/exist/anywhere")),
        });

        let copilot = Copilot::new();
        let err = copilot.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("missing"));
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
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.mcps.push(McpRef::InProcess(InProcessMcpHandle {
            name: "in-proc".to_string(),
            service: Arc::new(NoopService),
        }));

        let copilot = Copilot::new();
        let err = copilot.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("in-process"));
    }

    #[test]
    fn provision_hooks_are_a_documented_noop() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.hooks.push(crate::spec::HookRef {
            id: "notify".to_string(),
            event: "PreToolUse".to_string(),
            command: "notify-send hi".to_string(),
            matcher: Some("Bash".to_string()),
        });

        let copilot = Copilot::new();
        // Must not error and must not write anything hook-shaped — this CLI
        // version has no native hook slot.
        copilot.provision(&spec, config_dir.path()).unwrap();
        assert!(!config_dir.path().join("hooks").exists());
    }

    #[test]
    fn provision_instructions_writes_copilot_instructions_md() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER: be terse".to_string()),
            prompt: None,
        });

        let copilot = Copilot::new();
        copilot.provision(&spec, config_dir.path()).unwrap();

        let instructions_path = config_dir.path().join("copilot-instructions.md");
        assert!(instructions_path.exists());
        let content = std::fs::read_to_string(&instructions_path).unwrap();
        assert!(content.contains("REMEMBER: be terse"));
    }

    #[test]
    fn provision_account_api_key_env_maps_to_copilot_github_token() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "pat".to_string(),
            api_key_env: Some("PATH".to_string()),
            ..Default::default()
        });

        let expected = std::env::var("PATH").expect("PATH should be set in the test environment");

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "COPILOT_GITHUB_TOKEN" && v == &expected));
        assert!(!launch.env.iter().any(|(k, _)| k == "GITHUB_TOKEN"));

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
    fn provision_account_auth_token_env_maps_to_github_token() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "fallback".to_string(),
            auth_token_env: Some("PATH".to_string()),
            ..Default::default()
        });

        let expected = std::env::var("PATH").expect("PATH should be set in the test environment");

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "GITHUB_TOKEN" && v == &expected));
        assert!(!launch.env.iter().any(|(k, _)| k == "COPILOT_GITHUB_TOKEN"));
    }

    #[test]
    fn provision_account_base_url_maps_to_copilot_gh_host() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "enterprise".to_string(),
            base_url: Some("mycompany.ghe.com".to_string()),
            ..Default::default()
        });

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "COPILOT_GH_HOST" && v == "mycompany.ghe.com"));
    }

    #[test]
    fn provision_account_unset_api_key_env_is_an_error_naming_the_var() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "broken".to_string(),
            api_key_env: Some("__AM_DEFINITELY_UNSET_VAR__".to_string()),
            ..Default::default()
        });

        let copilot = Copilot::new();
        let err = copilot.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("__AM_DEFINITELY_UNSET_VAR__"));
    }

    #[test]
    fn provision_account_home_seeds_config_json_without_touching_home_env() {
        use crate::account::Account;

        // A persistent per-account "home" holding a captured login, laid out
        // exactly as `login()` writes it: `<home>/config.json`.
        let account_home = tempfile::TempDir::new().unwrap();
        std::fs::write(
            account_home.path().join("config.json"),
            r#"{"lastLoggedInUser":{"login":"octocat"}}"#,
        )
        .unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "private-home".to_string(),
            home: Some(account_home.path().to_path_buf()),
            ..Default::default()
        });

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        // COPILOT_HOME relocates to the ephemeral dir, NOT the account's home.
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "COPILOT_HOME" && v == &config_dir.path().display().to_string()));
        assert!(!launch.env.iter().any(|(k, _)| k == "HOME"));

        // The captured login is SEEDED into the ephemeral dir directly (no
        // `.copilot/` prefix).
        let seeded = config_dir.path().join("config.json");
        assert!(seeded.exists(), "config.json should be seeded into the ephemeral dir");
        assert!(std::fs::read_to_string(&seeded).unwrap().contains("octocat"));
    }

    #[test]
    fn provision_structured_builds_headless_argv() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;
        spec.model = Some("gpt-5.4".to_string());
        spec.resume = Some("sess-123".to_string());
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("say hello world".to_string()),
        });

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert_eq!(launch.args.first(), Some(&"-p".to_string()));
        assert_eq!(launch.args.get(1), Some(&"say hello world".to_string()));
        assert!(launch.args.contains(&"--output-format".to_string()));
        assert!(launch.args.contains(&"json".to_string()));
        assert!(launch.args.contains(&"--allow-all".to_string()));
        assert!(launch.args.contains(&"--no-ask-user".to_string()));
        let model_idx = launch.args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(launch.args.get(model_idx + 1), Some(&"gpt-5.4".to_string()));
        // --resume takes its value via `=` (an optional-value option), not a
        // separate argv token.
        assert!(launch.args.contains(&"--resume=sess-123".to_string()));
    }

    #[test]
    fn provision_passthrough_builds_interactive_argv() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("copilot".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.model = Some("gpt-5.4".to_string());
        spec.resume = Some("sess-123".to_string());
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("say hello world".to_string()),
        });

        let copilot = Copilot::new();
        let launch = copilot.provision(&spec, config_dir.path()).unwrap();

        assert!(!launch.args.contains(&"-p".to_string()));
        assert!(!launch.args.contains(&"--output-format".to_string()));
        assert!(!launch.args.contains(&"--allow-all".to_string()));
        let model_idx = launch.args.iter().position(|a| a == "--model").unwrap();
        assert_eq!(launch.args.get(model_idx + 1), Some(&"gpt-5.4".to_string()));
        assert!(launch.args.contains(&"--resume=sess-123".to_string()));
        // Interactive prompt seeding is `-i <text>`, never a bare positional.
        let i_idx = launch.args.iter().position(|a| a == "-i").unwrap();
        assert_eq!(launch.args.get(i_idx + 1), Some(&"say hello world".to_string()));
    }

    #[test]
    fn login_points_copilot_home_at_capture_dir_and_names_config_json() {
        let home = tempfile::TempDir::new().unwrap();

        let plan = Copilot::new().login(home.path()).unwrap();

        assert_eq!(plan.launch.program, "copilot");
        assert_eq!(plan.launch.args, vec!["login".to_string()]);
        assert!(plan
            .launch
            .env
            .iter()
            .any(|(k, v)| k == "COPILOT_HOME" && v == &home.path().display().to_string()));
        assert!(!plan.launch.env.iter().any(|(k, _)| k == "HOME"));
        assert_eq!(
            plan.credential_files[0],
            std::path::PathBuf::from("config.json")
        );
    }

    #[test]
    fn resolve_copilot_by_id() {
        assert_eq!(super::super::resolve("copilot").unwrap().id(), "copilot");
    }

    #[test]
    fn parse_model_ids_extracts_the_quoted_bullet_list() {
        // A trimmed real excerpt of `copilot help config`'s `model` entry.
        let help_text = r#"
  `logLevel`: log level for CLI; defaults to "default".

  `model`: AI model to use for Copilot CLI; can be changed with /model command or --model flag option.
    - "claude-sonnet-5"
    - "gpt-5.4"
    - "gemini-3.5-flash"

  `contextTier`: context window tier for tiered-pricing models.
"#;
        let ids = parse_model_ids(help_text);
        assert_eq!(ids, vec!["claude-sonnet-5", "gpt-5.4", "gemini-3.5-flash"]);
    }

    #[test]
    fn parse_model_ids_missing_block_returns_empty() {
        assert_eq!(parse_model_ids("no model entry here at all"), Vec::<String>::new());
    }

    #[test]
    fn copilot_supports_passthrough_and_structured() {
        let copilot = Copilot::new();
        let support = copilot.io_support();
        assert!(support.passthrough);
        assert!(support.structured);
    }
}
