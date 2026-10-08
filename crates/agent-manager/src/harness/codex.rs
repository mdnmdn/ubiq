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
//! memory are injected without ever touching the user's real config. Codex is
//! Class A (`CODEX_HOME` relocates the whole tree, `auth.json` included), and
//! the child's real `HOME` is left intact.
//!
//! The shared-home run (`D194`, [`Codex::provision_home`]) is the other shape: `CODEX_HOME` is
//! the account's own persistent home, which holds `auth.json` and `sessions/` and which Codex
//! refreshes itself. Nothing per-run is written there — MCP, model, effort, permissions and
//! instructions go by `-c key=value` — and only what no flag carries, skills and `hooks.json`,
//! is placed into it. `codex-acp` runs the same way, save that its MCP servers travel in ACP's
//! `session/new`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use tracing::warn;

use crate::Result;
use crate::config::{McpServer, McpTransport};
use crate::spec::{HookRef, McpRef, RunSpec};

use super::{ConfigAnchor, Harness, Launch, Relocate};

/// The markers that wrap `am`-managed `[mcp_servers.*]` tables in
/// `config.toml`, so any hand-authored tables in the same file (were any to
/// coexist) survive a rewrite. See codex.md "MCP at launch".
const MCP_MANAGED_BEGIN: &str = "# BEGIN managed mcp_servers";
const MCP_MANAGED_END: &str = "# END managed mcp_servers";

/// The npm bin the ACP variant launches — `@agentclientprotocol/codex-acp`, the Agent Client
/// Protocol org's adapter for Codex. It drives the same `codex` the native variant does (and
/// honours the same `CODEX_HOME`), so everything provisioning writes into the ephemeral dir
/// still reaches the model; only the wire differs. Unlike Claude's `claude-agent-acp`, this
/// binary answers `--version` directly (verified against 1.10.0), so [`Harness::version`]'s
/// default (`<command> --version`) needs no override here.
const ACP_COMMAND: &str = "codex-acp";

/// The Codex harness provisioner.
///
/// One provisioner, two harnesses. `acp` picks which wire a *structured* run speaks: the
/// native `app-server --listen stdio://` JSON-RPC bridge ([`crate::io::codex::CodexBridge`]) or
/// the Agent Client Protocol ([`crate::io::AcpBridge`]) by way of [`ACP_COMMAND`]. Everything
/// else — config.toml, skills, AGENTS.md, hooks, accounts, login and the whole passthrough
/// argv — is the same harness and is shared rather than duplicated.
#[derive(Debug, Clone, Default)]
pub struct Codex {
    /// A structured run speaks ACP through the adapter rather than the native `app-server`.
    acp: bool,
}

impl Codex {
    /// Construct the Codex harness descriptor.
    pub fn new() -> Self {
        Codex { acp: false }
    }

    /// Construct the ACP-speaking Codex harness descriptor (`codex-acp`).
    pub fn new_acp() -> Self {
        Codex { acp: true }
    }

    /// The launch both provisioning shapes share: `config_args` (`-c` overrides, root flags)
    /// first, then the structured `app-server --listen stdio://` or the passthrough prompt, and
    /// `CODEX_HOME` set only when there is a `config_home` to point it at.
    fn launch(
        &self,
        spec: &RunSpec,
        config_home: Option<&Path>,
        config_args: Vec<String>,
    ) -> Result<Launch> {
        // Structured mode launches the JSON-RPC `app-server`, with the
        // prompt delivered via `turn/start` by the bridge rather than a
        // trailing positional argument; passthrough mode keeps the
        // interactive argv shape from P1.
        let structured = spec.io == crate::spec::IoModes::Structured;
        // The ACP variant's *structured* launch is the adapter, which takes no subcommand: the
        // prompt is a `session/prompt`, a resume is `session/load` (from
        // `Provisioned::resume`), and everything else it needs it reads from `CODEX_HOME`,
        // exactly as the native variant's child does. Its one flag is the same root-level
        // `-c key=value` Codex parses (`codex-acp`'s `main` takes Codex's own
        // `CliConfigOverrides`), so `config_args` reach it too. A passthrough run is unchanged
        // — `codex-acp` is not a TUI, so a pane still gets the real `codex`.
        let acp = self.acp && structured;

        let mut args = config_args;
        if structured && !acp {
            args.push("app-server".to_string());
            args.push("--listen".to_string());
            args.push("stdio://".to_string());
        }
        args.extend(spec.passthrough_args.iter().cloned());

        // Resume: codex has no CLI resume flag. Resuming a prior codex
        // session is an app-server `thread/resume` JSON-RPC call, which
        // `structured_bridge` makes from `Provisioned::resume` — nothing in
        // argv. Do NOT invent a flag.

        // Append prompt as trailing positional argument, passthrough mode
        // only — structured mode's bridge sends it via `turn/start`.
        if !structured && let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
            args.push(prompt.clone());
        }

        // Account: inject credential *references* into the child's env.
        let mut env = Vec::new();
        if let Some(home) = config_home {
            env.push(("CODEX_HOME".to_string(), home.display().to_string()));
        }
        if let Some(account) = &spec.account {
            // Codex has no separate auth-token env var (unlike Claude's
            // ANTHROPIC_API_KEY / ANTHROPIC_AUTH_TOKEN split): both
            // api_key_env and auth_token_env map to OPENAI_API_KEY. If both
            // are set on the account, api_key_env wins.
            if let Some(name) = account
                .api_key_env
                .as_ref()
                .or(account.auth_token_env.as_ref())
            {
                let value = super::shared::account_env(account, name)?;
                env.push(("OPENAI_API_KEY".to_string(), value));
            }
            // TODO(P2+): base_url → model_providers. Codex has no single env
            // var for a custom base URL; a custom endpoint requires a
            // `[model_providers.<name>]` table plus `model_provider = "<name>"`
            // in config.toml. Don't fake it with an env var codex won't read.
        }

        Ok(Launch {
            program: if acp { ACP_COMMAND } else { "codex" }.to_string(),
            args,
            env,
            env_remove: Vec::new(),
            env_clear: false,
        })
    }
}

impl Harness for Codex {
    fn id(&self) -> crate::spec::HarnessId {
        if self.acp { "codex-acp" } else { "codex" }.to_string()
    }

    fn display_name(&self) -> &str {
        if self.acp { "Codex (ACP)" } else { "Codex" }
    }

    fn command(&self) -> &str {
        if self.acp { ACP_COMMAND } else { "codex" }
    }

    fn aliases(&self) -> &[&str] {
        if self.acp { &["codex-acp"] } else { &["codex"] }
    }

    fn io_support(&self) -> super::IoSupport {
        super::IoSupport {
            passthrough: true,
            structured: true,
            multi_turn: true,
            acp: self.acp,
            // The native bridge sends a prompt into a live turn as `turn/steer`; `codex-acp` has
            // no such call.
            steer: !self.acp,
            // A short-lived `codex app-server` answers `account/rateLimits/read` with no thread
            // and no model call (`Self::quota`), and a running native bridge also pushes
            // `account/rateLimits/updated` — so the answer needs no conversation.
            quota: super::QuotaSource::Probe,
        }
    }

    /// Class A: `CODEX_HOME` relocates the entire tree — `auth.json` included —
    /// while the real `HOME` stays intact. See `_docs/profiles.md` §5.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: vec![("CODEX_HOME".to_string(), Relocate::All)],
            requires_home_relocation: false,
        }
    }

    /// Codex exposes its model catalogue via `codex debug models --bundled`
    /// (JSON; the bundled list needs no network). Each entry has a `slug` (the
    /// id passed to `model`), a `display_name`, and a `visibility` — we surface
    /// the ones marked `list`. Requires Codex ≥ 0.131.0.
    fn discover_models(&self) -> Result<Vec<super::ModelInfo>> {
        let parsed = bundled_models()?;
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

    /// Reads `default_reasoning_level` / `supported_reasoning_levels` off the same
    /// `codex debug models --bundled` value [`discover_models`](Self::discover_models)
    /// reads — no second probe, no second process spawn. Models whose level list is
    /// empty are skipped: an empty catalog entry is the honest "no reasoning knob" case,
    /// same as the default in [`super::Harness::discover_thinking`].
    fn discover_thinking(&self) -> Result<BTreeMap<String, super::ModelThinking>> {
        thinking_from_bundled(&bundled_models()?)
    }

    /// The three values [`map_sandbox_mode`] accepts. Fixed CLI enum, not probed — see
    /// [`super::Harness::modes`].
    fn modes(&self) -> Vec<super::ModeInfo> {
        [
            ("read-only", "Read only"),
            ("workspace-write", "Workspace write"),
            ("danger-full-access", "Danger full access"),
        ]
        .into_iter()
        .map(|(id, label)| super::ModeInfo {
            id: id.to_string(),
            label: label.to_string(),
            description: None,
        })
        .collect()
    }

    /// `danger-full-access` is Codex's own name for asking nothing — see
    /// [`super::Harness::unattended_mode`].
    fn unattended_mode(&self) -> Option<&'static str> {
        Some("danger-full-access")
    }

    /// Everything [`map_sandbox_mode`] accepts, which is [`Self::modes`] **plus** the documented
    /// `restricted` alias for `read-only`. The alias is a real value a profile or a `[presets]`
    /// block may carry (settings' own sample uses it), so validation must not drop it even
    /// though it is not a picker entry — see [`super::Harness::accepts_mode`].
    fn accepts_mode(&self, mode: &str) -> bool {
        map_sandbox_mode(mode).is_some()
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // The ephemeral `dir` is ALWAYS the config home (`$CODEX_HOME`), so
        // all injected config — `config.toml`, skills, `AGENTS.md`, hooks —
        // lands in the throwaway dir and never pollutes an account's
        // persistent home (nor collides across concurrent runs).
        let config_home = dir.to_path_buf();
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
        write_skills(spec, &config_home.join(".agents").join("skills"))?;

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

        // 5. The launch, with `CODEX_HOME` at the ephemeral dir and every
        // setting already in its config.toml, so no `-c` override.
        let launch = self.launch(spec, Some(&config_home), Vec::new())?;
        Ok(launch)
    }

    /// Both variants run from a shared home: `codex-acp` takes the same `-c` overrides, and its
    /// MCP servers go over the wire instead.
    fn shares_home(&self) -> bool {
        true
    }

    /// `$CODEX_HOME`, else `~/.codex`: the whole tree, `auth.json` included.
    fn default_homes(&self) -> Vec<std::path::PathBuf> {
        super::env_dir_or_home("CODEX_HOME", ".codex")
            .into_iter()
            .collect()
    }

    /// A run against the account's shared `CODEX_HOME` (`D194`), which holds `auth.json` and
    /// `sessions/` and which Codex refreshes itself. Nothing per-run is written into `home`;
    /// each per-run setting is a `-c key=value` override ahead of any subcommand
    /// ([`config_overrides`]) — MCP servers, model, reasoning effort, sandbox and approval
    /// policy, and the instructions as `developer_instructions`. A `-c` layers over the
    /// home's own `config.toml`, so a server the profile's user added there still loads: there
    /// is no strict-MCP switch.
    ///
    /// No flag carries skills or hooks, so they are the profile's: placed into the home at
    /// `.agents/skills/<id>/` and `hooks.json`, each swapped in whole under a lock
    /// ([`write_home_skills`], [`write_home_file`]). `RunSpec` does not tell a profile's skill
    /// from one added for this run, so two runs of one profile that differ there overwrite each
    /// other. With no `home` (a native run) the launch sets no `CODEX_HOME`, Codex runs from the
    /// user's own `~/.codex`, and skills and hooks are dropped with a warning — nothing is
    /// written into the user's config. `scratch` stays unused: no per-run file is needed.
    ///
    /// A structured `codex-acp` run takes the same `-c` overrides but for MCP: its servers go in
    /// `session/new`'s `mcpServers` ([`crate::provision::Provisioned::mcp_servers`], sent by
    /// [`Harness::structured_bridge`]). `codex-acp` adds them to the home's own, as a `-c` does.
    fn provision_home(
        &self,
        spec: &RunSpec,
        home: Option<&Path>,
        _scratch: &Path,
    ) -> Result<Launch> {
        let mcp_over_wire = self.acp && spec.io == crate::spec::IoModes::Structured;
        let config_args = config_overrides(spec, !mcp_over_wire)?;
        match home {
            Some(home) => {
                super::write_profile_skills(spec, &home.join(".agents").join("skills"))?;
                if !spec.hooks.is_empty() {
                    write_home_file(&home.join("hooks.json"), &build_hooks_json(&spec.hooks)?)?;
                }
            }
            None => {
                if !spec.skills.is_empty()
                    || !spec.mcp_as_skill.is_empty()
                    || !spec.hooks.is_empty()
                {
                    warn!(
                        skills = spec.skills.len() + spec.mcp_as_skill.len(),
                        hooks = spec.hooks.len(),
                        "codex has no per-run route for skills or hooks; a run with no profile drops them"
                    );
                }
            }
        }
        self.launch(spec, home, config_args)
    }

    /// `codex login` with `CODEX_HOME=<home>`: the login lands where every run of the profile
    /// reads it, and Codex refreshes it from then on. Nothing is written first and nothing is
    /// read back.
    fn login_home(&self, home: &Path) -> Result<Launch> {
        Ok(Launch {
            program: "codex".to_string(),
            args: vec!["login".to_string()],
            env: vec![("CODEX_HOME".to_string(), home.display().to_string())],
            env_remove: Vec::new(),
            env_clear: false,
        })
    }

    /// Both variants: they share the home and the login, and the sign-in is the app-server's.
    fn device_login(&self) -> bool {
        true
    }

    fn begin_device_login(&self, home: &Path) -> Result<DeviceLogin> {
        begin_device_login(home)
    }

    /// Codex keeps its login in `auth.json` at the root of `CODEX_HOME` — the home itself.
    /// On macOS a harness may keep the login in the OS keychain instead, so an absent file is
    /// not proof of no login.
    fn login_files(&self) -> Vec<PathBuf> {
        vec![PathBuf::from("auth.json")]
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        if self.acp {
            // Empty on the legacy path, whose MCP is in the run's own `config.toml`.
            return Ok(Box::new(crate::io::AcpBridge::with_mcp_servers(
                child,
                cwd,
                provisioned.resume.as_deref(),
                provisioned.model.as_deref(),
                &provisioned.mcp_servers,
            )?));
        }
        // A resume is a `thread/resume` over the wire — Codex has no resume flag — so the id
        // provisioning carried rides into the handshake. A fork is a `thread/fork` for the same
        // reason: the thread lives in `CODEX_HOME`, which an account's runs share, so copying the
        // run directory forks nothing — resuming the id there would put two conversations on one
        // thread.
        let open = thread_open(provisioned);
        Ok(Box::new(crate::io::codex::CodexBridge::open(
            child, cwd, open,
        )?))
    }

    /// The `codex` limit's five-hour and weekly windows, read by [`crate::quota::codex`] from a
    /// short-lived app-server on the account's home. The same answer for both variants: they
    /// share the home and the login.
    fn quota(&self, account: &str, home: Option<&Path>) -> Result<crate::quota::QuotaSnapshot> {
        crate::quota::codex(account, &self.id(), home)
    }
}

/// How the native bridge opens its thread for a provisioned run: fresh, resumed, or forked.
fn thread_open(provisioned: &crate::provision::Provisioned) -> crate::io::codex::ThreadOpen {
    use crate::io::codex::ThreadOpen;
    match provisioned.resume.as_deref() {
        Some(id) if provisioned.fork => ThreadOpen::Fork(id.to_string()),
        Some(id) => ThreadOpen::Resume(id.to_string()),
        None => ThreadOpen::Start,
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

/// Run `codex debug models --bundled` (JSON; the bundled list needs no network) and parse its
/// stdout. Shared by [`Codex::discover_models`] and [`Codex::discover_thinking`] so the model
/// catalog is read from a single process spawn, not two. Requires Codex ≥ 0.131.0.
fn bundled_models() -> Result<serde_json::Value> {
    let mut cmd = std::process::Command::new("codex");
    cmd.args(["debug", "models", "--bundled"]);
    cmd.current_dir(super::shared::probe_cwd());
    // Headless model-catalog probe: nothing reads a window, and the app has already freed its own
    // console (`ubiq_app::detach_console`), so a console-subsystem child would otherwise flash one.
    #[cfg(windows)]
    super::shared::no_window(&mut cmd);
    let output = cmd.output().with_context(
        || "running `codex debug models --bundled` (is the codex binary on PATH, ≥ 0.131.0?)",
    )?;
    if !output.status.success() {
        anyhow::bail!(
            "`codex debug models --bundled` failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("parsing `codex debug models --bundled` JSON")
}

/// How long the account probe waits for `codex app-server` to answer, start to finish. A cold
/// start answers `initialize` in well under a second (0.161.0, observed); the reads behind it
/// go to the network, hence the margin.
const ACCOUNT_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// What a Codex home says about its login, read from a short-lived app-server.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AccountProbe {
    /// `account/read`'s `account`: `{ type: "chatgpt", email, planType }`, `{ type: "apiKey" }`,
    /// or `Null` when the home is signed out.
    pub account: serde_json::Value,
    /// `account/rateLimits/read`'s result, or the sentence the app-server refused it with —
    /// "codex account authentication required to read rate limits" for an API-key or signed-out
    /// home.
    pub rate_limits: std::result::Result<serde_json::Value, String>,
}

/// Ask a Codex home who it is signed in as and how much of the plan is left, by running the
/// app-server the IDE extension uses against it: `initialize`, `initialized`, `account/read`,
/// `account/rateLimits/read`, then close. No thread is started and no model is called.
///
/// `home` is the account's `CODEX_HOME`, or the user's own default when `None`. The login stays
/// where Codex keeps it; nothing here opens `auth.json` or the keychain.
pub(crate) fn account_via_app_server(home: Option<&Path>) -> Result<AccountProbe> {
    account_probe("codex", home, &[], &super::shared::probe_cwd())
}

/// [`account_via_app_server`] against any program speaking the app-server's stdio protocol, with
/// extra environment and a working directory, so a test can stand in a fake.
fn account_probe(
    program: &str,
    home: Option<&Path>,
    envs: &[(&str, &str)],
    cwd: &Path,
) -> Result<AccountProbe> {
    let deadline = std::time::Instant::now() + ACCOUNT_PROBE_TIMEOUT;
    let mut server = AppServer::spawn(program, home, envs, cwd, deadline)?;
    let account = server.call("account/read", serde_json::json!({}), deadline)?;
    if let Some(error) = account.get("error") {
        bail!("codex could not read the account: {}", rpc_message(error));
    }
    let limits = server.call(
        "account/rateLimits/read",
        serde_json::json!({"excludeResetCreditDetails": true}),
        deadline,
    )?;
    Ok(AccountProbe {
        account: account
            .pointer("/result/account")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        rate_limits: match limits.get("error") {
            Some(error) => Err(rpc_message(error)),
            None => Ok(limits
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null)),
        },
    })
}

/// A short-lived `codex app-server` for one-off account work — the probe, a device-code sign-in.
/// Initialized on spawn; killed (and its reader joined) on drop. Every wait is bounded by the
/// caller's deadline.
struct AppServer {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    rx: std::sync::mpsc::Receiver<serde_json::Value>,
    reader: Option<std::thread::JoinHandle<()>>,
    next_id: i64,
}

impl AppServer {
    fn spawn(
        program: &str,
        home: Option<&Path>,
        envs: &[(&str, &str)],
        cwd: &Path,
        deadline: std::time::Instant,
    ) -> Result<Self> {
        use std::io::BufRead;
        use std::process::{Command, Stdio};

        let mut cmd = Command::new(program);
        cmd.args(["app-server", "--listen", "stdio://"])
            .envs(envs.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(home) = home {
            cmd.env("CODEX_HOME", home);
        }
        cmd.current_dir(cwd);
        #[cfg(windows)]
        super::shared::no_window(&mut cmd);
        let mut child = cmd
            .spawn()
            .context("spawning `codex app-server` (is codex on PATH?)")?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let (Some(stdin), Some(stdout)) = (stdin, stdout) else {
            let _ = child.kill();
            let _ = child.wait();
            bail!("codex app-server's pipes were not opened");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                    && tx.send(value).is_err()
                {
                    break;
                }
            }
        });
        let mut server = Self {
            child,
            stdin,
            rx,
            reader: Some(reader),
            next_id: 1,
        };
        let init = server.call(
            "initialize",
            serde_json::json!({
                "clientInfo": {"name": "agent-manager", "version": env!("CARGO_PKG_VERSION")},
                "capabilities": {"experimentalApi": true},
            }),
            deadline,
        )?;
        if let Some(error) = init.get("error") {
            bail!(
                "codex app-server refused initialize: {}",
                rpc_message(error)
            );
        }
        server
            .write(&serde_json::json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}))?;
        Ok(server)
    }

    fn write(&mut self, value: &serde_json::Value) -> Result<()> {
        use std::io::Write;
        writeln!(self.stdin, "{value}").context("writing to codex app-server")?;
        self.stdin.flush().context("writing to codex app-server")
    }

    /// Send a request and wait for its response — the whole response, `error` included.
    fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
        deadline: std::time::Instant,
    ) -> Result<serde_json::Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(
            &serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        )?;
        self.wait_for(deadline, |v| {
            v.get("method").is_none() && v.get("id").and_then(|i| i.as_i64()) == Some(id)
        })
        .with_context(|| format!("waiting for codex to answer `{method}`"))
    }

    /// Wait for the first frame `wanted` accepts; everything before it is dropped.
    fn wait_for(
        &mut self,
        deadline: std::time::Instant,
        wanted: impl Fn(&serde_json::Value) -> bool,
    ) -> Result<serde_json::Value> {
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let value = self
                .rx
                .recv_timeout(left)
                .map_err(|_| anyhow::anyhow!("codex app-server went quiet"))?;
            if wanted(&value) {
                return Ok(value);
            }
        }
    }
}

impl Drop for AppServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// How long a device-code sign-in may wait for the person to finish it.
const DEVICE_LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// A device-code sign-in under way: the page to open and the code to type there. Started by
/// [`begin_device_login`]; [`DeviceLogin::wait`] blocks until Codex says it finished.
pub struct DeviceLogin {
    server: AppServer,
    login_id: String,
    /// Where the person signs in.
    pub verification_url: String,
    /// The one-time code they type there.
    pub user_code: String,
}

impl DeviceLogin {
    /// Block until the sign-in completes (`account/login/completed` for this login), fails, or
    /// [`DEVICE_LOGIN_TIMEOUT`] passes. On success the login is in the home Codex keeps it in.
    pub fn wait(mut self) -> Result<()> {
        let deadline = std::time::Instant::now() + DEVICE_LOGIN_TIMEOUT;
        let login_id = self.login_id.clone();
        let done = self.server.wait_for(deadline, |v| {
            v.get("method").and_then(|m| m.as_str()) == Some("account/login/completed")
                && v.pointer("/params/loginId").and_then(|l| l.as_str()) == Some(&login_id)
        })?;
        if done.pointer("/params/success").and_then(|s| s.as_bool()) == Some(true) {
            return Ok(());
        }
        bail!(
            "codex sign-in failed: {}",
            done.pointer("/params/error")
                .and_then(|e| e.as_str())
                .unwrap_or("no reason given")
        )
    }
}

/// Sign `home` (an account's `CODEX_HOME`) in with ChatGPT by device code, through the
/// app-server's `account/login/start { type: "chatgptDeviceCode" }` — no terminal, no browser
/// callback: the caller shows [`DeviceLogin::verification_url`] and [`DeviceLogin::user_code`]
/// and then waits. The pane path (`Codex::login_home`, `codex login`) is unchanged.
pub fn begin_device_login(home: &Path) -> Result<DeviceLogin> {
    device_login("codex", home, &[], &super::shared::probe_cwd())
}

fn device_login(
    program: &str,
    home: &Path,
    envs: &[(&str, &str)],
    cwd: &Path,
) -> Result<DeviceLogin> {
    let deadline = std::time::Instant::now() + ACCOUNT_PROBE_TIMEOUT;
    let mut server = AppServer::spawn(program, Some(home), envs, cwd, deadline)?;
    let started = server.call(
        "account/login/start",
        serde_json::json!({"type": "chatgptDeviceCode"}),
        deadline,
    )?;
    if let Some(error) = started.get("error") {
        bail!("codex refused the sign-in: {}", rpc_message(error));
    }
    let field = |key: &str| {
        started
            .pointer(&format!("/result/{key}"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("codex's sign-in answer has no {key}"))
    };
    Ok(DeviceLogin {
        login_id: field("loginId")?,
        verification_url: field("verificationUrl")?,
        user_code: field("userCode")?,
        server,
    })
}

/// A JSON-RPC error's `message`, or the whole error where it has none.
fn rpc_message(error: &serde_json::Value) -> String {
    error
        .get("message")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string())
}

/// Parse `default_reasoning_level` / `supported_reasoning_levels` out of a
/// `codex debug models --bundled` value (see [`bundled_models`]), keyed by `slug`. A model
/// whose `supported_reasoning_levels` is missing or empty has no reasoning knob and is
/// skipped, not inserted with an empty vec — the honest "no entry" is more useful to a caller
/// than an entry it has to check for emptiness anyway.
fn thinking_from_bundled(
    parsed: &serde_json::Value,
) -> Result<BTreeMap<String, super::ModelThinking>> {
    let models = parsed
        .get("models")
        .and_then(|m| m.as_array())
        .ok_or_else(|| anyhow::anyhow!("no 'models' array in `codex debug models` output"))?;
    let mut out = BTreeMap::new();
    for model in models {
        let Some(slug) = model.get("slug").and_then(|s| s.as_str()) else {
            continue;
        };
        let Some(supported) = model
            .get("supported_reasoning_levels")
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        if supported.is_empty() {
            continue;
        }
        let levels: Vec<super::ThinkingLevel> = supported
            .iter()
            .filter_map(|level| {
                let effort = level.get("effort").and_then(|e| e.as_str())?;
                let description = level
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(str::to_string);
                Some(super::ThinkingLevel {
                    value: effort.to_string(),
                    label: super::effort_label(effort),
                    description,
                })
            })
            .collect();
        if levels.is_empty() {
            continue;
        }
        let default_level = model
            .get("default_reasoning_level")
            .and_then(|d| d.as_str())
            .map(str::to_string);
        out.insert(
            slug.to_string(),
            super::ModelThinking {
                levels,
                default_level,
            },
        );
    }
    Ok(out)
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
/// The top-level `model` / `model_reasoning_effort` keys in codex's `config.toml`. Serialized
/// with the `toml` crate so ids are correctly escaped. Both optional so the one struct covers
/// "model only", "effort only" and "neither" without three call sites.
#[derive(Debug, Default, serde::Serialize)]
struct ModelToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_reasoning_effort: Option<String>,
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

/// The `approval_policy` a sandbox mode runs under: the unattended mode asks nothing (`never`),
/// every other mode asks when it needs to (`on-request`), and the question reaches a person —
/// the native bridge parks it as a permission request, a terminal pane shows Codex's own prompt.
fn approval_policy(sandbox_mode: &str) -> &'static str {
    if sandbox_mode == "danger-full-access" {
        "never"
    } else {
        "on-request"
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

    // Model selection + reasoning effort: the top-level `model` /
    // `model_reasoning_effort` keys in config.toml are honored by both interactive
    // (passthrough) codex and the app-server, so `am`'s mode-agnostic `spec.model`/
    // `spec.thinking` map here rather than to CLI flags. Emitted first so they stay
    // top-level keys (before any `[table]`). Only written when set, so runs without
    // either keep a byte-identical config.toml. `spec.thinking` is filtered for an
    // empty string: codex rejects `model_reasoning_effort = ""`.
    let thinking = spec.thinking.clone().filter(|v| !v.is_empty());
    if spec.model.is_some() || thinking.is_some() {
        out.push_str(
            &toml::to_string(&ModelToml {
                model: spec.model.clone(),
                model_reasoning_effort: thinking,
            })
            .context("serializing model")?,
        );
        out.push('\n');
    }

    if let Some(policy) = &spec.policy {
        match policy.permission_mode.as_deref().and_then(map_sandbox_mode) {
            Some(sandbox_mode) => {
                let permissions = PermissionToml {
                    sandbox_mode: Some(sandbox_mode.to_string()),
                    approval_policy: Some(approval_policy(sandbox_mode).to_string()),
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

/// Copy each of `spec`'s skills into `<skills_dir>/<id>/`, and write the MCP-as-skill pointers
/// beside them (stepping stone; see `harness::write_mcp_as_skill_pointers`).
fn write_skills(spec: &RunSpec, skills_dir: &Path) -> Result<()> {
    for skill in &spec.skills {
        let dest = skills_dir.join(&skill.id);
        skill
            .source
            .materialize(&dest, crate::source::LinkMode::Copy, true)
            .with_context(|| format!("copying skill '{}' into {}", skill.id, dest.display()))?;
    }
    super::write_mcp_as_skill_pointers(spec, skills_dir)
}

/// Write `contents` to `path` in a shared home: staged beside it and renamed over it, under a
/// lock beside it, so a running Codex never reads a half-written file.
fn write_home_file(path: &Path, contents: &str) -> Result<()> {
    // Released when `_lock` drops, at the end of this function.
    let _lock = super::lock_beside(path)?;
    let mut staged = path.as_os_str().to_owned();
    staged.push(".am-tmp");
    let staged = std::path::PathBuf::from(staged);
    std::fs::write(&staged, contents).with_context(|| format!("writing {}", staged.display()))?;
    std::fs::rename(&staged, path).with_context(|| format!("writing {}", path.display()))
}

/// One `-c key=value` pair; `value` is rendered as an inline TOML value, which is how Codex
/// parses the right-hand side of an override.
fn push_override(args: &mut Vec<String>, key: &str, value: toml::Value) {
    args.push("-c".to_string());
    args.push(format!("{key}={value}"));
}

/// The `-c key=value` overrides a shared-home run passes in place of the `config.toml` a
/// fixed dir gets (codex.md "Config-layer precedence": CLI overrides, dot notation, sit above
/// the user's `config.toml`). The same keys [`build_config_toml`] writes — `model`,
/// `model_reasoning_effort`, `sandbox_mode` + `approval_policy`, one `mcp_servers.<id>` inline
/// table per server — plus `developer_instructions` for the run's instructions, a config key
/// codex.md lists among a custom agent's config-layer keys and one the app-server's
/// `thread/start` sets as `developerInstructions`. `mcp_by_flag` false leaves the servers out,
/// for a run that sends them over the wire; an in-process one is refused either way.
fn config_overrides(spec: &RunSpec, mcp_by_flag: bool) -> Result<Vec<String>> {
    let mut args = Vec::new();
    if let Some(model) = &spec.model {
        push_override(&mut args, "model", toml::Value::String(model.clone()));
    }
    // Codex rejects `model_reasoning_effort = ""`, as for config.toml.
    if let Some(effort) = spec.thinking.as_ref().filter(|v| !v.is_empty()) {
        push_override(
            &mut args,
            "model_reasoning_effort",
            toml::Value::String(effort.clone()),
        );
    }
    if let Some(sandbox_mode) = spec
        .policy
        .as_ref()
        .and_then(|p| p.permission_mode.as_deref())
        .and_then(map_sandbox_mode)
    {
        push_override(
            &mut args,
            "sandbox_mode",
            toml::Value::String(sandbox_mode.to_string()),
        );
        push_override(
            &mut args,
            "approval_policy",
            toml::Value::String(approval_policy(sandbox_mode).to_string()),
        );
    }
    if let Some(instructions) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
        push_override(
            &mut args,
            "developer_instructions",
            toml::Value::String(instructions.clone()),
        );
    }
    for mcp in &spec.mcps {
        match mcp {
            McpRef::Catalog(_) | McpRef::Inline(_) if !mcp_by_flag => {}
            McpRef::Catalog(server) | McpRef::Inline(server) => {
                let table = toml::Value::try_from(mcp_server_toml(server))
                    .with_context(|| format!("serializing MCP server '{}'", server.id))?;
                push_override(&mut args, &format!("mcp_servers.{}", server.id), table);
            }
            McpRef::InProcess(_) => {
                bail!("in-process MCP not supported in passthrough mode");
            }
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{ConfigStrategy, Instructions, McpRef, Policy, SkillRef};
    use std::path::PathBuf;

    super::super::shared::harness_conformance_tests!(Codex, "codex");

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
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "CODEX_HOME" && v == &config_dir.path().display().to_string())
        );
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

        let content = std::fs::read_to_string(config_dir.path().join("config.toml")).unwrap();
        assert!(content.contains("sandbox_mode = \"read-only\""));
        // Only the unattended mode asks nothing; a read-only run asks a person.
        assert!(content.contains("approval_policy = \"on-request\""));
        assert_eq!(approval_policy("danger-full-access"), "never");
        assert_eq!(approval_policy("workspace-write"), "on-request");
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

        let content = std::fs::read_to_string(config_dir.path().join("config.toml")).unwrap();
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
            fn call(
                &self,
                _name: &str,
                _arguments: serde_json::Value,
            ) -> crate::Result<serde_json::Value> {
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

        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "OPENAI_API_KEY" && v == &expected)
        );

        // No-secret-on-disk invariant.
        super::super::shared::assert_no_secret_on_disk(config_dir.path(), &expected);
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

        let skill_md_path = config_dir.path().join(".agents/skills/postgres/SKILL.md");
        assert!(skill_md_path.exists());
        let content = std::fs::read_to_string(&skill_md_path).unwrap();
        assert!(content.contains("name: postgres"));
        assert!(content.contains("description: Query a DB."));

        // Invariant: the MCP stays injected as normal in config.toml.
        let config_toml = std::fs::read_to_string(config_dir.path().join("config.toml")).unwrap();
        assert!(config_toml.contains("[mcp_servers.postgres]"));
    }

    #[test]
    fn provision_resume_is_a_noop_argv_stays_unchanged() {
        let config_dir = tempfile::TempDir::new().unwrap();

        let base_spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        let mut base_spec_fixed = base_spec.clone();
        base_spec_fixed.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());

        let codex = Codex::new();
        let launch_without_resume = codex
            .provision(&base_spec_fixed, config_dir.path())
            .unwrap();

        let config_dir2 = tempfile::TempDir::new().unwrap();
        let mut resumed_spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        resumed_spec.config = ConfigStrategy::Fixed(config_dir2.path().to_path_buf());
        resumed_spec.resume = Some("abc".to_string());
        let launch_with_resume = codex.provision(&resumed_spec, config_dir2.path()).unwrap();

        assert_eq!(launch_without_resume.args, launch_with_resume.args);
    }

    /// A fork of a thread on a shared `CODEX_HOME` is `thread/fork`, never a resume of the same id.
    #[test]
    fn a_forked_resume_opens_a_fork() {
        use crate::io::codex::ThreadOpen;
        let mut provisioned = crate::provision::Provisioned {
            dir: PathBuf::from("."),
            launch: crate::harness::Launch {
                program: "codex".to_string(),
                args: vec![],
                env: vec![],
                env_remove: vec![],
                env_clear: false,
            },
            ephemeral: false,
            home: None,
            resume: None,
            fork: true,
            model: None,
            mcp_servers: Vec::new(),
            #[cfg(feature = "inproc-mcp")]
            inproc_servers: Vec::new(),
        };
        assert_eq!(super::thread_open(&provisioned), ThreadOpen::Start);
        provisioned.resume = Some("t1".to_string());
        assert_eq!(
            super::thread_open(&provisioned),
            ThreadOpen::Fork("t1".to_string())
        );
        provisioned.fork = false;
        assert_eq!(
            super::thread_open(&provisioned),
            ThreadOpen::Resume("t1".to_string())
        );
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
    fn config_toml_carries_model_reasoning_effort_when_thinking_set() {
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.thinking = Some("xhigh".to_string());
        let toml_str = build_config_toml(&spec).unwrap();
        let parsed: toml::Value = toml::from_str(&toml_str).unwrap();
        assert_eq!(
            parsed
                .get("model_reasoning_effort")
                .and_then(|v| v.as_str()),
            Some("xhigh")
        );
    }

    #[test]
    fn config_toml_omits_model_reasoning_effort_when_thinking_unset() {
        let spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        let toml = build_config_toml(&spec).unwrap();
        assert!(
            !toml.contains("model_reasoning_effort"),
            "config.toml should not carry model_reasoning_effort when unset:\n{toml}"
        );
    }

    #[test]
    fn config_toml_never_emits_empty_model_reasoning_effort() {
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.thinking = Some(String::new());
        let toml = build_config_toml(&spec).unwrap();
        assert!(
            !toml.contains("model_reasoning_effort"),
            "config.toml should never emit an empty model_reasoning_effort:\n{toml}"
        );
    }

    #[test]
    fn every_mode_id_maps_through_map_sandbox_mode() {
        for mode in Codex::new().modes() {
            assert!(
                map_sandbox_mode(&mode.id).is_some(),
                "mode '{}' did not map through map_sandbox_mode",
                mode.id
            );
        }
    }

    #[test]
    fn thinking_from_bundled_parses_fixture() {
        let fixture = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/codex-debug-models-bundled.json"
        ))
        .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&fixture).unwrap();
        let thinking = thinking_from_bundled(&parsed).unwrap();

        let gpt55 = thinking
            .get("gpt-5.5")
            .expect("gpt-5.5 has reasoning levels");
        assert_eq!(gpt55.default_level.as_deref(), Some("medium"));
        let values: Vec<&str> = gpt55.levels.iter().map(|l| l.value.as_str()).collect();
        assert_eq!(values, vec!["low", "medium", "high", "xhigh"]);
        assert_eq!(gpt55.levels[3].label, "Extra high");

        assert!(
            !thinking.contains_key("gpt-5-chat-latest"),
            "a model with an empty supported_reasoning_levels must be absent from the map"
        );
    }

    /// Live check against a real `codex` on PATH. Skipped when the binary is missing so unit
    /// CI without Codex still passes.
    #[test]
    fn discover_thinking_live_when_codex_available() {
        let has_codex = std::process::Command::new("codex")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !has_codex {
            eprintln!("skipping: `codex` not on PATH");
            return;
        }
        let thinking = Codex::new()
            .discover_thinking()
            .expect("live `codex debug models --bundled` discovery");
        assert!(
            thinking.values().any(|t| !t.levels.is_empty()),
            "expected at least one model with at least one reasoning level"
        );
    }

    /// A spec carrying one of everything a shared-home run passes by `-c`.
    fn home_spec(home: &Path, scratch: &Path) -> RunSpec {
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("/tmp/project"));
        spec.config = ConfigStrategy::Home {
            home: home.to_path_buf(),
            scratch: scratch.to_path_buf(),
        };
        spec.io = crate::spec::IoModes::Structured;
        spec.model = Some("gpt-5.5".to_string());
        spec.thinking = Some("high".to_string());
        spec.policy = Some(Policy {
            permission_mode: Some("read-only".to_string()),
            ..Default::default()
        });
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER \"ME\"\nline two".to_string()),
            prompt: None,
        });
        spec.mcps.push(McpRef::Inline(McpServer {
            id: "docs".to_string(),
            transport: McpTransport::Http,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some("https://example.com/mcp/".to_string()),
            headers: BTreeMap::new(),
        }));
        spec
    }

    /// The `-c` values joined into one TOML document, as Codex layers them.
    fn overrides_as_toml(args: &[String]) -> toml::Table {
        let doc: Vec<&str> = args
            .windows(2)
            .filter(|w| w[0] == "-c")
            .map(|w| w[1].as_str())
            .collect();
        toml::from_str(&doc.join("\n")).unwrap()
    }

    #[test]
    fn provision_home_writes_nothing_into_home_and_passes_every_setting_by_c() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let spec = home_spec(home.path(), scratch.path());

        let launch = Codex::new()
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();

        let home_entries: Vec<_> = std::fs::read_dir(home.path()).unwrap().collect();
        assert!(home_entries.is_empty(), "home got: {home_entries:?}");

        // Root flags come before the subcommand.
        let app_server = launch.args.iter().position(|a| a == "app-server").unwrap();
        assert_eq!(
            launch.args[app_server..],
            ["app-server", "--listen", "stdio://"]
        );
        assert!(launch.args[..app_server].chunks(2).all(|c| c[0] == "-c"));

        let doc = overrides_as_toml(&launch.args);
        assert_eq!(doc["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(doc["model_reasoning_effort"].as_str(), Some("high"));
        assert_eq!(doc["sandbox_mode"].as_str(), Some("read-only"));
        assert_eq!(doc["approval_policy"].as_str(), Some("on-request"));
        assert_eq!(
            doc["developer_instructions"].as_str(),
            Some("REMEMBER \"ME\"\nline two")
        );
        assert_eq!(
            doc["mcp_servers"]["docs"]["url"].as_str(),
            Some("https://example.com/mcp/")
        );
        assert_eq!(
            launch.env,
            vec![("CODEX_HOME".to_string(), home.path().display().to_string())]
        );
    }

    #[test]
    fn provision_home_without_a_home_sets_no_codex_home_and_writes_no_skill() {
        let scratch = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Native {
            scratch: scratch.path().to_path_buf(),
        };
        spec.model = Some("gpt-5.5".to_string());
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            source: crate::source::Source::Dir(write_skill(skills_src.path(), "my-skill")),
        });

        let launch = Codex::new()
            .provision_home(&spec, None, scratch.path())
            .unwrap();

        assert!(!launch.env.iter().any(|(k, _)| k == "CODEX_HOME"));
        assert_eq!(
            overrides_as_toml(&launch.args)["model"].as_str(),
            Some("gpt-5.5")
        );
        let scratch_entries: Vec<_> = std::fs::read_dir(scratch.path()).unwrap().collect();
        assert!(
            scratch_entries.is_empty(),
            "scratch got: {scratch_entries:?}"
        );
    }

    #[test]
    fn provision_home_places_skills_and_hooks_in_the_home_and_replaces_them_whole() {
        use crate::spec::HookRef;

        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");
        std::fs::write(skill_path.join("extra.md"), "old").unwrap();
        let mut spec = RunSpec::new("codex".to_string(), PathBuf::from("."));
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            source: crate::source::Source::Dir(skill_path.clone()),
        });
        spec.hooks.push(HookRef {
            id: "notify".to_string(),
            event: "PreToolUse".to_string(),
            command: "notify-send hi".to_string(),
            matcher: None,
        });

        let codex = Codex::new();
        codex
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();
        // The second run's copy replaces the first's whole: a file gone from the source is gone.
        std::fs::remove_file(skill_path.join("extra.md")).unwrap();
        codex
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();

        let skills_dir = home.path().join(".agents/skills");
        assert!(skills_dir.join("my-skill/SKILL.md").is_file());
        assert!(!skills_dir.join("my-skill/extra.md").exists());
        assert!(!skills_dir.join(".am-stage").exists());
        assert!(!skills_dir.join(".am-old").exists());
        let hooks: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(home.path().join("hooks.json")).unwrap())
                .unwrap();
        assert_eq!(
            hooks["PreToolUse"][0]["command"].as_str(),
            Some("notify-send hi")
        );
        // Neither config.toml nor AGENTS.md: those stay the profile user's.
        assert!(!home.path().join("config.toml").exists());
        assert!(!home.path().join("AGENTS.md").exists());
    }

    /// `codex-acp` takes the native variant's `-c` overrides, with no subcommand and no MCP
    /// server among them — those go over the wire — and `CODEX_HOME` only under a home.
    #[test]
    fn codex_acp_shares_a_home_by_c_with_mcp_left_to_the_wire() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let spec = home_spec(home.path(), scratch.path());
        let acp = Codex::new_acp();
        assert!(acp.shares_home() && Codex::new().shares_home());

        let launch = acp
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();
        assert_eq!(launch.program, "codex-acp");
        assert!(
            launch.args.chunks(2).all(|c| c[0] == "-c"),
            "{:?}",
            launch.args
        );
        let doc = overrides_as_toml(&launch.args);
        assert_eq!(doc["model"].as_str(), Some("gpt-5.5"));
        assert_eq!(doc["sandbox_mode"].as_str(), Some("read-only"));
        assert!(doc["developer_instructions"].is_str());
        assert!(!doc.contains_key("mcp_servers"));
        assert_eq!(
            launch.env,
            vec![("CODEX_HOME".to_string(), home.path().display().to_string())]
        );

        let native = acp.provision_home(&spec, None, scratch.path()).unwrap();
        assert!(!native.env.iter().any(|(k, _)| k == "CODEX_HOME"));
        assert_eq!(native.args, launch.args);

        // A passthrough pane is the real `codex`, so its servers stay on the command line.
        let mut passthrough = spec.clone();
        passthrough.io = crate::spec::IoModes::Passthrough;
        let pane = acp
            .provision_home(&passthrough, Some(home.path()), scratch.path())
            .unwrap();
        assert_eq!(pane.program, "codex");
        assert!(overrides_as_toml(&pane.args).contains_key("mcp_servers"));

        assert_eq!(
            acp.login_home(home.path()).unwrap().args,
            Codex::new().login_home(home.path()).unwrap().args
        );
    }

    #[test]
    fn login_home_logs_in_through_codex_home_and_writes_nothing() {
        let home = tempfile::TempDir::new().unwrap();
        let launch = Codex::new().login_home(home.path()).unwrap();

        assert_eq!(launch.program, "codex");
        assert_eq!(launch.args, vec!["login"]);
        assert_eq!(
            launch.env,
            vec![("CODEX_HOME".to_string(), home.path().display().to_string())]
        );
        assert!(std::fs::read_dir(home.path()).unwrap().next().is_none());
    }

    fn fake_appserver() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-codex-appserver.sh").to_string()
    }

    #[test]
    fn account_probe_reads_who_and_the_limits_and_exits() {
        let cwd = tempfile::TempDir::new().unwrap();
        let probe = account_probe(&fake_appserver(), None, &[], cwd.path()).unwrap();
        assert_eq!(probe.account["email"], "dev@example.com");
        assert_eq!(probe.account["planType"], "pro");
        let limits = probe.rate_limits.unwrap();
        assert_eq!(limits["rateLimits"]["primary"]["windowDurationMins"], 300);
    }

    #[test]
    fn account_probe_carries_the_refusal_as_a_sentence() {
        let cwd = tempfile::TempDir::new().unwrap();
        let probe = account_probe(
            &fake_appserver(),
            None,
            &[("AM_FAKE_CODEX_SIGNED_OUT", "1")],
            cwd.path(),
        )
        .unwrap();
        assert_eq!(
            probe.rate_limits,
            Err("codex account authentication required to read rate limits".to_string())
        );
    }

    #[test]
    fn device_login_states_the_code_then_completes() {
        let cwd = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        let login = device_login(&fake_appserver(), home.path(), &[], cwd.path()).unwrap();
        assert_eq!(login.verification_url, "https://auth.example/device");
        assert_eq!(login.user_code, "ABCD-1234");
        login.wait().unwrap();
    }

    #[test]
    fn codex_states_its_limits_by_probe() {
        assert_eq!(
            Codex::new().io_support().quota,
            crate::quota::QuotaSource::Probe
        );
        assert_eq!(
            Codex::new_acp().io_support().quota,
            crate::quota::QuotaSource::Probe
        );
    }
}
