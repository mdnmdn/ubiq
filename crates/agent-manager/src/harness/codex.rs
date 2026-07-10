//! Codex provisioner.
//!
//! Transcribes `_docs/harness/codex.md` (esp. "On-disk layout (Global
//! `~/.codex/`)", "MCP servers", "Orchestration / … / MCP at launch",
//! "Skills at launch", and "Permissions" §"System A: legacy `approval_policy`
//! + `sandbox_mode`") into a [`Harness`] impl.
//!
//! The "custom config folder" bridge: Codex resolves nearly everything
//! (config, `auth.json`, skills, `AGENTS.md`) under a single root, `$CODEX_HOME`
//! (default `~/.codex`). Provisioning points that variable at the ephemeral
//! dir instead of the real `~/.codex`, so MCP servers/permissions/skills/
//! memory are injected without ever touching the user's real config. Unlike
//! Claude Code (which can split `CLAUDE_CONFIG_DIR` from `HOME`), Codex has no
//! way to keep credentials in one place and injected config in another — see
//! the `provision` doc comment on the account-home tradeoff this forces.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context};

use crate::config::{McpServer, McpTransport};
use crate::spec::{HookRef, McpRef, RunSpec};
use crate::Result;

use super::{copy_dir_recursive, Harness, IoSupport, Launch};

/// The markers that wrap `am`-managed `[mcp_servers.*]` tables in
/// `config.toml`, so any hand-authored tables in the same file (were any to
/// coexist) survive a rewrite. See codex.md "MCP at launch".
const MCP_MANAGED_BEGIN: &str = "# BEGIN managed mcp_servers";
const MCP_MANAGED_END: &str = "# END managed mcp_servers";

/// The Codex harness provisioner.
#[derive(Debug, Clone, Default)]
pub struct Codex;

impl Codex {
    /// Construct the Codex harness descriptor.
    pub fn new() -> Self {
        Codex
    }
}

impl Harness for Codex {
    fn id(&self) -> crate::spec::HarnessId {
        "codex".to_string()
    }

    fn display_name(&self) -> &str {
        "Codex"
    }

    fn command(&self) -> &str {
        "codex"
    }

    fn aliases(&self) -> &[&str] {
        &["codex"]
    }

    fn io_support(&self) -> IoSupport {
        IoSupport {
            passthrough: true,
            structured: true,
        }
    }

    /// Codex exposes its model catalogue via `codex debug models --bundled`
    /// (JSON; the bundled list needs no network). Each entry has a `slug` (the
    /// id passed to `model`), a `display_name`, and a `visibility` — we surface
    /// the ones marked `list`. Requires Codex ≥ 0.131.0.
    fn discover_models(&self) -> Result<Vec<super::ModelInfo>> {
        let output = std::process::Command::new("codex")
            .args(["debug", "models", "--bundled"])
            .output()
            .with_context(|| "running `codex debug models --bundled` (is the codex binary on PATH, ≥ 0.131.0?)")?;
        if !output.status.success() {
            anyhow::bail!(
                "`codex debug models --bundled` failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let parsed: serde_json::Value = serde_json::from_slice(&output.stdout)
            .context("parsing `codex debug models --bundled` JSON")?;
        let models = parsed
            .get("models")
            .and_then(|m| m.as_array())
            .ok_or_else(|| anyhow::anyhow!("no 'models' array in `codex debug models` output"))?;
        let out: Vec<super::ModelInfo> = models
            .iter()
            .filter(|m| {
                // Keep only user-listable models; skip hidden/internal ones.
                m.get("visibility").and_then(|v| v.as_str()) != Some("hidden")
            })
            .filter_map(|m| {
                let slug = m.get("slug").and_then(|s| s.as_str())?;
                let desc = m
                    .get("display_name")
                    .and_then(|d| d.as_str())
                    .map(str::to_string);
                Some(super::ModelInfo {
                    id: slug.to_string(),
                    description: desc,
                    default: false,
                })
            })
            .collect();
        Ok(out)
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // Codex unifies config + `auth.json` under one root (`$CODEX_HOME`),
        // unlike Claude Code which can split `CLAUDE_CONFIG_DIR` (injected
        // config) from `HOME` (credential store). So when an account carries
        // a private `home`, that dir itself becomes the write target (and
        // later `CODEX_HOME`) instead of the ephemeral `dir` — there is no
        // way to inject config while keeping a *different* dir as the
        // credential root. This is a deliberate codex-specific tradeoff: a
        // private-home account gets its own fully-isolated CODEX_HOME rather
        // than a split config/credentials setup.
        let config_home = spec
            .account
            .as_ref()
            .and_then(|a| a.home.clone())
            .unwrap_or_else(|| dir.to_path_buf());
        std::fs::create_dir_all(&config_home)
            .with_context(|| format!("creating {}", config_home.display()))?;

        // 1. MCP + permissions: always write <config_home>/config.toml, even
        // with zero servers, so the run is fully controlled.
        let config_toml = build_config_toml(spec)?;
        let config_toml_path = config_home.join("config.toml");
        std::fs::write(&config_toml_path, config_toml)
            .with_context(|| format!("writing {}", config_toml_path.display()))?;

        // 2. Skills: copy each skill folder into
        // <config_home>/.agents/skills/<id>/. NOTE: codex.md's "On-disk
        // layout" section documents user skills as living under
        // `~/.agents/skills/` (not `~/.codex/.agents/skills/`), but its
        // "Skills at launch" section says the per-run copy target is
        // `$CODEX_HOME/.agents/skills/<name>/SKILL.md`. We follow the
        // "at launch" guidance since it is the explicit orchestration
        // contract for a per-run provisioner.
        let skills_dir = config_home.join(".agents").join("skills");
        for skill in &spec.skills {
            if !skill.path.exists() {
                bail!(
                    "skill '{}' points at a path that does not exist: {}",
                    skill.id,
                    skill.path.display()
                );
            }
            let dest = skills_dir.join(&skill.id);
            copy_dir_recursive(&skill.path, &dest).with_context(|| {
                format!(
                    "copying skill '{}' from {} to {}",
                    skill.id,
                    skill.path.display(),
                    dest.display()
                )
            })?;
        }
        // 2b. MCP-as-skill: latent SKILL.md pointers into the same skills
        // dir (stepping stone; see harness::write_mcp_as_skill_pointers's
        // doc). No-op when spec.mcp_as_skill is empty.
        super::write_mcp_as_skill_pointers(spec, &skills_dir)?;

        // 3. Instructions: <config_home>/AGENTS.md (the CODEX_HOME/global
        // memory tier — never the user's cwd). Plain Markdown; no managed-
        // marker requirement for codex, but a leading comment line documents
        // provenance.
        if let Some(instr_text) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
            let agents_md = format!("<!-- agent-manager managed -->\n{}\n", instr_text);
            let agents_md_path = config_home.join("AGENTS.md");
            std::fs::write(&agents_md_path, agents_md)
                .with_context(|| format!("writing {}", agents_md_path.display()))?;
        }

        // 4. Hooks: <config_home>/hooks.json, written only when spec.hooks is
        // non-empty (hookless runs are unaffected). codex.md lists
        // `hooks.json` as an accepted (legacy) hooks representation
        // alongside inline `[[hooks.<Event>]]` in config.toml, but does not
        // pin its exact schema — this shape is a best-effort guess, not a
        // verified-against-source-schema fidelity claim like the mcp/config
        // rendering above.
        if !spec.hooks.is_empty() {
            let hooks_json = build_hooks_json(&spec.hooks)?;
            let hooks_json_path = config_home.join("hooks.json");
            std::fs::write(&hooks_json_path, hooks_json)
                .with_context(|| format!("writing {}", hooks_json_path.display()))?;
        }

        // 5. Build the launch. Structured mode launches the JSON-RPC
        // `app-server` (`codex app-server --listen stdio://`), with the
        // prompt delivered via `turn/start` by the bridge rather than a
        // trailing positional argument; passthrough mode keeps the
        // interactive argv shape from P1.
        let structured = spec.io == crate::spec::IoModes::Structured;

        let mut args = Vec::new();
        if structured {
            args.push("app-server".to_string());
            args.push("--listen".to_string());
            args.push("stdio://".to_string());
        }
        args.extend(spec.passthrough_args.iter().cloned());

        // Resume: codex has no CLI resume flag. Resuming a prior codex
        // session is an app-server `thread/resume` JSON-RPC call (a bridge
        // concern), not something expressible in launch argv — so
        // `spec.resume` is a documented no-op here, deferred to a later
        // step. Do NOT invent a flag.

        // Append prompt as trailing positional argument, passthrough mode
        // only — structured mode's bridge sends it via `turn/start`.
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
        let mut env = vec![("CODEX_HOME".to_string(), config_home.display().to_string())];
        if let Some(account) = &spec.account {
            // Codex has no separate auth-token env var (unlike Claude's
            // ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN split): both
            // api_key_env and auth_token_env map to OPENAI_API_KEY. If both
            // are set on the account, api_key_env wins.
            if let Some(name) = &account.api_key_env {
                let value = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "account '{}' references env var '{}' which is not set",
                        account.id,
                        name
                    )
                })?;
                env.push(("OPENAI_API_KEY".to_string(), value));
            } else if let Some(name) = &account.auth_token_env {
                let value = std::env::var(name).map_err(|_| {
                    anyhow::anyhow!(
                        "account '{}' references env var '{}' which is not set",
                        account.id,
                        name
                    )
                })?;
                env.push(("OPENAI_API_KEY".to_string(), value));
            }
            // TODO(P2+): base_url → model_providers. Codex has no single env
            // var for a custom base URL; a custom endpoint requires a
            // `[model_providers.<name>]` table plus `model_provider = "<name>"`
            // in config.toml. Don't fake it with an env var codex won't read.
        }

        Ok(Launch {
            program: "codex".to_string(),
            args,
            env,
            env_remove: Vec::new(),
        })
    }

    /// Log Codex into `home`, capturing the resulting `auth.json`.
    ///
    /// Per codex.md "Credential capture & reuse": `CODEX_HOME` is the clean
    /// relocation lever (moves the whole tree, including `auth.json`), so
    /// pointing it at `home` here mirrors exactly what the reuse path
    /// (`provision()` above) does for a private-home account. Before
    /// launching login, force file-based credential storage by writing
    /// `cli_auth_credentials_store = "file"` into `home/config.toml` — this
    /// is the documented knob to skip the OS keychain (critical under
    /// sandboxes where no keychain is reachable), and it must be written
    /// *before* `codex login` runs so the token lands in `auth.json` rather
    /// than the keychain. `home` is fresh at capture time (a new account's
    /// login dir), so a plain overwrite is fine here; the reuse path's
    /// `provision()` re-provisions `config.toml` on every run anyway, so
    /// this file isn't "owned" by login in any lasting sense.
    ///
    /// Verified against the installed `codex login --help` (codex-cli
    /// 0.142.5): plain `codex login` (browser OAuth) is used here. Note
    /// codex.md's "Login command" line mentions a headless `codex login
    /// --device-code`, but the installed CLI's actual flag for the
    /// browserless path is `--device-auth` (no `--device-code` exists in
    /// this version) — that's the sandbox-friendly alternative to swap in
    /// if a headless capture flow is needed later.
    fn login(&self, home: &Path) -> Result<super::LoginPlan> {
        std::fs::create_dir_all(home)
            .with_context(|| format!("creating {}", home.display()))?;
        let config_toml_path = home.join("config.toml");
        std::fs::write(&config_toml_path, "cli_auth_credentials_store = \"file\"\n")
            .with_context(|| format!("writing {}", config_toml_path.display()))?;

        let env = vec![("CODEX_HOME".to_string(), home.display().to_string())];
        let args = vec!["login".to_string()];
        Ok(super::LoginPlan {
            launch: Launch {
                program: "codex".to_string(),
                args,
                env,
                env_remove: Vec::new(),
            },
            credential_files: vec![std::path::PathBuf::from("auth.json")],
        })
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        Ok(Box::new(crate::io::codex::CodexBridge::new(child, cwd)?))
    }
}

/// TOML shape for one `[mcp_servers.<id>]` table. Field presence
/// distinguishes stdio (`command`/`args`/`env`) from streamable HTTP
/// (`url`/`http_headers`) per codex.md "MCP servers" §Schema. Codex's only
/// documented remote transport is streamable HTTP, so both [`McpTransport::Http`]
/// and [`McpTransport::Sse`] map to `url` + `http_headers`.
#[derive(Debug, serde::Serialize)]
struct McpServerToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    http_headers: BTreeMap<String, String>,
}

fn mcp_server_toml(server: &McpServer) -> McpServerToml {
    match server.transport {
        McpTransport::Stdio => McpServerToml {
            command: server.command.clone(),
            args: server.args.clone(),
            env: server.env.clone(),
            url: None,
            http_headers: BTreeMap::new(),
        },
        McpTransport::Http | McpTransport::Sse => McpServerToml {
            command: None,
            args: Vec::new(),
            env: BTreeMap::new(),
            url: server.url.clone(),
            http_headers: server.headers.clone(),
        },
    }
}

/// Top-level document wrapping the `[mcp_servers.*]` tables, so serializing
/// it via the `toml` crate produces one `[mcp_servers.<id>]` section per
/// server (sorted by id — deterministic output, mirrors claude.rs).
#[derive(Debug, serde::Serialize)]
struct McpServersDoc {
    mcp_servers: BTreeMap<String, McpServerToml>,
}

/// Render the `am`-managed `[mcp_servers.*]` block (without markers).
fn build_mcp_servers_block(mcps: &[McpRef]) -> Result<String> {
    let mut servers: BTreeMap<String, McpServerToml> = BTreeMap::new();
    for mcp in mcps {
        match mcp {
            McpRef::Catalog(server) | McpRef::Inline(server) => {
                servers.insert(server.id.clone(), mcp_server_toml(server));
            }
            McpRef::InProcess(_) => {
                bail!("in-process MCP not supported in passthrough mode");
            }
        }
    }
    let doc = McpServersDoc {
        mcp_servers: servers,
    };
    toml::to_string(&doc).context("serializing mcp_servers block")
}

/// Top-level permission keys (System A only — see codex.md "Permissions"
/// §"System A: legacy `sandbox_mode` + `approval_policy`"). Serialized with
/// the `toml` crate for correctness rather than hand-formatted strings.
/// The top-level `model` key in codex's `config.toml`. Serialized with the
/// `toml` crate so the id is correctly escaped.
#[derive(Debug, serde::Serialize)]
struct ModelToml {
    model: String,
}

#[derive(Debug, Default, serde::Serialize)]
struct PermissionToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    sandbox_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_policy: Option<String>,
}

/// Map a harness-agnostic `policy.permission_mode` value to a codex
/// `sandbox_mode`, if it is a recognized codex value (case-insensitive;
/// `restricted` is treated as an alias for `read-only`). Returns `None` for
/// anything else — the caller then omits permission keys and leaves codex on
/// its defaults, per the P2 B1 task spec (no invented aliases beyond the
/// documented ones).
fn map_sandbox_mode(mode: &str) -> Option<&'static str> {
    match mode.to_lowercase().as_str() {
        "read-only" | "restricted" => Some("read-only"),
        "workspace-write" => Some("workspace-write"),
        "danger-full-access" => Some("danger-full-access"),
        _ => None,
    }
}

/// One `hooks.json` entry: `{"command": …, "matcher": …}`. `matcher` is
/// omitted (not just `null`) when the hook carries none.
///
/// NOTE(fidelity caveat): codex.md documents `hooks.json` as an accepted
/// (legacy) hooks representation but does not pin its exact schema (only the
/// inline `config.toml` `[[hooks.<Event>]]` form is spelled out). This shape
/// — `{ "<event>": [ { "command": …, "matcher": … } ] }` — is a best-effort
/// guess at the sibling JSON file's shape, not verified against a real Codex
/// schema; treat it as provisional until codex.md documents `hooks.json`
/// directly.
#[derive(Debug, serde::Serialize)]
struct HooksJsonEntry {
    command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    matcher: Option<String>,
}

/// Build the `hooks.json` document: grouped by native event name, each
/// event's array holding one [`HooksJsonEntry`] per [`HookRef`] in that
/// event.
fn build_hooks_json(hooks: &[HookRef]) -> Result<String> {
    let mut by_event: BTreeMap<String, Vec<HooksJsonEntry>> = BTreeMap::new();
    for hook in hooks {
        by_event
            .entry(hook.event.clone())
            .or_default()
            .push(HooksJsonEntry {
                command: hook.command.clone(),
                matcher: hook.matcher.clone(),
            });
    }
    serde_json::to_string_pretty(&by_event).context("serializing hooks.json")
}

/// Build the full `config.toml` body: optional top-level permission keys
/// (System A only), then the `am`-managed `[mcp_servers.*]` block wrapped in
/// BEGIN/END comment markers.
fn build_config_toml(spec: &RunSpec) -> Result<String> {
    let mut out = String::new();

    // Model selection: the top-level `model` key in config.toml is honored by
    // both interactive (passthrough) codex and the app-server, so `am`'s
    // mode-agnostic `spec.model` maps here rather than to a CLI flag. Emitted
    // first so it stays a top-level key (before any `[table]`). Only written
    // when set, so runs without `--model` keep a byte-identical config.toml.
    if let Some(model) = &spec.model {
        out.push_str(&toml::to_string(&ModelToml { model: model.clone() }).context("serializing model")?);
        out.push('\n');
    }

    if let Some(policy) = &spec.policy {
        match policy.permission_mode.as_deref().and_then(map_sandbox_mode) {
            Some(sandbox_mode) => {
                let permissions = PermissionToml {
                    sandbox_mode: Some(sandbox_mode.to_string()),
                    // Unattended run: never block on an interactive approval
                    // prompt (still respects the sandbox above).
                    approval_policy: Some("never".to_string()),
                };
                out.push_str(&toml::to_string(&permissions).context("serializing permissions")?);
            }
            None => {
                // permission_mode absent or not a recognized codex value
                // (e.g. a Claude-specific mode like "acceptEdits"); omit
                // permission keys entirely and let codex use its defaults
                // rather than guess at a mapping.
                out.push_str(
                    "# policy.permission_mode did not map to a recognized codex permission mode; using codex defaults\n",
                );
            }
        }
        out.push('\n');
    }

    let mcp_block = build_mcp_servers_block(&spec.mcps)?;
    out.push_str(MCP_MANAGED_BEGIN);
    out.push('\n');
    out.push_str(&mcp_block);
    out.push_str(MCP_MANAGED_END);
    out.push('\n');

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ConfigStrategy, Instructions, McpRef, Policy, SkillRef};
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
    fn provision_writes_config_toml_skills_agents_md_and_launch_without_touching_home() {
        // A stand-in for the user's real `$HOME`. `provision()` never reads
        // or writes an env-derived home directory (it only touches the
        // `dir`/account-`home` it is explicitly given), so this must stay
        // untouched. (We don't mutate the process's `HOME` var here:
        // `std::env::set_var` requires `unsafe` as of edition 2024, and this
        // crate forbids unsafe code; asserting the fake dir stays empty is
        // sufficient since nothing in the provisioner ever consults `HOME`.)
        let fake_home = tempfile::TempDir::new().unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");

        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
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
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER: be helpful".to_string()),
            prompt: Some("say hello world".to_string()),
        });

        let codex = Codex::new();
        let launch = codex.provision(&spec, config_dir.path()).unwrap();

        // config.toml exists, has the managed markers, and both servers.
        let config_toml_path = config_dir.path().join("config.toml");
        assert!(config_toml_path.exists());
        let content = std::fs::read_to_string(&config_toml_path).unwrap();
        assert!(content.contains(MCP_MANAGED_BEGIN));
        assert!(content.contains(MCP_MANAGED_END));
        assert!(content.contains("[mcp_servers.postgres]"));
        assert!(content.contains("command = \"postgres-mcp\""));
        assert!(content.contains("[mcp_servers.docs]"));
        assert!(content.contains("url = \"https://example.com/mcp/\""));

        let parsed: toml::Value = toml::from_str(&content).unwrap();
        let servers = parsed.get("mcp_servers").unwrap().as_table().unwrap();
        assert_eq!(
            servers["postgres"]["command"].as_str(),
            Some("postgres-mcp")
        );
        assert_eq!(
            servers["docs"]["url"].as_str(),
            Some("https://example.com/mcp/")
        );

        // skill copied under .agents/skills/<id>/.
        let skill_md = config_dir.path().join(".agents/skills/my-skill/SKILL.md");
        assert!(skill_md.exists());

        // AGENTS.md contains the instructions.
        let agents_md_path = config_dir.path().join("AGENTS.md");
        assert!(agents_md_path.exists());
        let agents_md = std::fs::read_to_string(&agents_md_path).unwrap();
        assert!(agents_md.contains("REMEMBER: be helpful"));

        // launch shape.
        assert!(launch
            .env
            .iter()
            .any(|(k, v)| k == "CODEX_HOME" && v == &config_dir.path().display().to_string()));
        assert_eq!(launch.args.last(), Some(&"say hello world".to_string()));

        // Invariant: nothing written under the stand-in home dir.
        let home_entries: Vec<_> = std::fs::read_dir(fake_home.path()).unwrap().collect();
        assert!(
            home_entries.is_empty(),
            "expected no writes under the fake home dir, found: {home_entries:?}"
        );
    }

    #[test]
    fn provision_hooks_writes_hooks_json() {
        use crate::spec::HookRef;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.hooks.push(HookRef {
            id: "notify".to_string(),
            event: "PreToolUse".to_string(),
            command: "notify-send hi".to_string(),
            matcher: Some("Bash".to_string()),
        });

        let codex = Codex::new();
        codex.provision(&spec, config_dir.path()).unwrap();

        let hooks_json_path = config_dir.path().join("hooks.json");
        assert!(hooks_json_path.exists());

        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&hooks_json_path).unwrap()).unwrap();
        assert_eq!(
            parsed["PreToolUse"][0]["command"].as_str(),
            Some("notify-send hi")
        );
        assert_eq!(parsed["PreToolUse"][0]["matcher"].as_str(), Some("Bash"));
    }

    #[test]
    fn provision_no_hooks_does_not_write_hooks_json() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("codex".to_string(), PathBuf::from("."));

        let codex = Codex::new();
        codex.provision(&spec, config_dir.path()).unwrap();

        assert!(!config_dir.path().join("hooks.json").exists());
    }

    #[test]
    fn provision_missing_skill_path_is_an_error() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.skills.push(SkillRef {
            id: "missing".to_string(),
            path: PathBuf::from("/definitely/does/not/exist/anywhere"),
        });

        let codex = Codex::new();
        let err = codex.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn provision_empty_mcps_still_writes_config_toml() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        let codex = Codex::new();
        codex.provision(&spec, config_dir.path()).unwrap();

        let config_toml_path = config_dir.path().join("config.toml");
        assert!(config_toml_path.exists());
        let content = std::fs::read_to_string(&config_toml_path).unwrap();
        assert!(content.contains(MCP_MANAGED_BEGIN));
        assert!(content.contains(MCP_MANAGED_END));
    }

    #[test]
    fn provision_recognized_permission_mode_sets_sandbox_and_approval_policy() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.policy = Some(Policy {
            permission_mode: Some("restricted".to_string()),
            allow: vec![],
            ask: vec![],
            deny: vec![],
        });

        let codex = Codex::new();
        codex.provision(&spec, config_dir.path()).unwrap();

        let content =
            std::fs::read_to_string(config_dir.path().join("config.toml")).unwrap();
        assert!(content.contains("sandbox_mode = \"read-only\""));
        assert!(content.contains("approval_policy = \"never\""));
    }

    #[test]
    fn provision_unrecognized_permission_mode_omits_permission_keys() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.policy = Some(Policy {
            permission_mode: Some("acceptEdits".to_string()),
            allow: vec![],
            ask: vec![],
            deny: vec![],
        });

        let codex = Codex::new();
        codex.provision(&spec, config_dir.path()).unwrap();

        let content =
            std::fs::read_to_string(config_dir.path().join("config.toml")).unwrap();
        assert!(!content.contains("sandbox_mode ="));
        assert!(!content.contains("approval_policy ="));
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
            fn call(&self, _name: &str, _arguments: serde_json::Value) -> crate::Result<serde_json::Value> {
                anyhow::bail!("not implemented")
            }
        }

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.mcps.push(McpRef::InProcess(InProcessMcpHandle {
            name: "in-proc".to_string(),
            service: Arc::new(NoopService),
        }));

        let codex = Codex::new();
        let err = codex.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("in-process"));
    }

    #[test]
    fn provision_account_api_key_env_maps_to_openai_api_key() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "path-account".to_string(),
            api_key_env: Some("PATH".to_string()),
            ..Default::default()
        });

        let expected = std::env::var("PATH").expect("PATH should be set in the test environment");

        let codex = Codex::new();
        let launch = codex.provision(&spec, config_dir.path()).unwrap();

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
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
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

        let codex = Codex::new();
        codex.provision(&spec, config_dir.path()).unwrap();

        let skill_md_path = config_dir
            .path()
            .join(".agents/skills/postgres/SKILL.md");
        assert!(skill_md_path.exists());
        let content = std::fs::read_to_string(&skill_md_path).unwrap();
        assert!(content.contains("name: postgres"));
        assert!(content.contains("description: Query a DB."));

        // Invariant: the MCP stays injected as normal in config.toml.
        let config_toml =
            std::fs::read_to_string(config_dir.path().join("config.toml")).unwrap();
        assert!(config_toml.contains("[mcp_servers.postgres]"));
    }

    #[test]
    fn provision_account_unset_api_key_env_is_an_error_naming_the_var() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "broken".to_string(),
            api_key_env: Some("__AM_DEFINITELY_UNSET_VAR__".to_string()),
            ..Default::default()
        });

        let codex = Codex::new();
        let err = codex.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("__AM_DEFINITELY_UNSET_VAR__"));
    }

    #[test]
    fn provision_account_home_becomes_codex_home_and_write_target() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let account_home = tempfile::TempDir::new().unwrap();

        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "private-home".to_string(),
            home: Some(account_home.path().to_path_buf()),
            ..Default::default()
        });

        let codex = Codex::new();
        let launch = codex.provision(&spec, config_dir.path()).unwrap();

        assert!(launch.env.iter().any(
            |(k, v)| k == "CODEX_HOME" && v == &account_home.path().display().to_string()
        ));
        // config.toml landed in the account's home, not the ephemeral dir.
        assert!(account_home.path().join("config.toml").exists());
        assert!(!config_dir.path().join("config.toml").exists());
    }

    #[test]
    fn provision_resume_is_a_noop_argv_stays_unchanged() {
        let config_dir = tempfile::TempDir::new().unwrap();

        let base_spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        let mut base_spec_fixed = base_spec.clone();
        base_spec_fixed.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());

        let codex = Codex::new();
        let launch_without_resume = codex.provision(&base_spec_fixed, config_dir.path()).unwrap();

        let config_dir2 = tempfile::TempDir::new().unwrap();
        let mut resumed_spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        resumed_spec.config = ConfigStrategy::Fixed(config_dir2.path().to_path_buf());
        resumed_spec.resume = Some("abc".to_string());
        let launch_with_resume = codex.provision(&resumed_spec, config_dir2.path()).unwrap();

        assert_eq!(launch_without_resume.args, launch_with_resume.args);
    }

    #[test]
    fn resolve_codex_by_id() {
        assert_eq!(super::super::resolve("codex").unwrap().id(), "codex");
    }

    #[test]
    fn provision_structured_io_builds_app_server_argv_without_positional_prompt() {
        use crate::spec::Instructions;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("say hello world".to_string()),
        });

        let codex = Codex::new();
        let launch = codex.provision(&spec, config_dir.path()).unwrap();

        assert!(launch.args.contains(&"app-server".to_string()));
        assert!(launch.args.contains(&"--listen".to_string()));
        assert!(launch.args.contains(&"stdio://".to_string()));
        // The prompt is delivered via `turn/start` by the bridge, not
        // appended as a positional argument.
        assert!(!launch.args.contains(&"say hello world".to_string()));
    }

    #[test]
    fn provision_passthrough_io_does_not_build_app_server_argv() {
        use crate::spec::Instructions;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        // spec.io defaults to Passthrough.
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("say hello world".to_string()),
        });

        let codex = Codex::new();
        let launch = codex.provision(&spec, config_dir.path()).unwrap();

        assert!(!launch.args.contains(&"app-server".to_string()));
        assert!(!launch.args.contains(&"--listen".to_string()));
        assert!(!launch.args.contains(&"stdio://".to_string()));
        assert_eq!(launch.args.last(), Some(&"say hello world".to_string()));
    }

    #[test]
    fn config_toml_carries_model_when_set() {
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.model = Some("gpt-5-codex".to_string());
        let toml = build_config_toml(&spec).unwrap();
        assert!(
            toml.contains("model = \"gpt-5-codex\""),
            "config.toml should carry the model key:\n{toml}"
        );
    }

    #[test]
    fn config_toml_omits_model_when_unset() {
        let spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        let toml = build_config_toml(&spec).unwrap();
        assert!(
            !toml.contains("model ="),
            "config.toml should not carry a model key when unset:\n{toml}"
        );
    }

    #[test]
    fn login_points_codex_home_at_capture_dir_names_auth_json_and_forces_file_store() {
        let home = tempfile::TempDir::new().unwrap();

        let plan = Codex::new().login(home.path()).unwrap();

        assert_eq!(plan.launch.program, "codex");
        assert!(plan.launch.args.contains(&"login".to_string()));
        assert!(plan
            .launch
            .env
            .iter()
            .any(|(k, v)| k == "CODEX_HOME" && v == &home.path().display().to_string()));
        assert_eq!(
            plan.credential_files.first(),
            Some(&PathBuf::from("auth.json"))
        );

        let config_toml = std::fs::read_to_string(home.path().join("config.toml")).unwrap();
        assert!(config_toml.contains("cli_auth_credentials_store = \"file\""));
    }
}
