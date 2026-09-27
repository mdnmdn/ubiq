//! Claude Code provisioner.
//!
//! Transcribes `_docs/harness/claude-code.md` (esp. "Orchestration / headless
//! invocation", "MCP at launch", "Skills at launch") into a [`Harness`] impl.
//!
//! The "custom config folder" bridge: Claude Code's user config dir can be
//! relocated with the `CLAUDE_CONFIG_DIR` environment variable. Provisioning
//! points that variable at the ephemeral dir instead of the real `~/.claude`,
//! so skills/settings/memory are injected without ever touching the user's
//! real config. Under a profile's shared home (`D193`) the variable names that
//! home instead, and every per-run file reaches the run by flag from its
//! scratch dir — see [`Claude::provision_home`](super::Harness::provision_home).
//! `claude-code-acp`, which takes no flag, gets its MCP servers over the wire
//! and the rest as profile-owned content in the home.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::Result;
use crate::config::{McpServer, McpTransport};
use crate::spec::{HookRef, McpRef, RunSpec};

use super::{ConfigAnchor, Harness, Launch, ModelInfo, Relocate};

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

/// The name of a shared-home run's per-run plugin, and so the prefix its skills are invoked by
/// (`am:<skill>`). Stable, since a resumed transcript names skills by it.
const PLUGIN_NAME: &str = "am";

/// The npm bin the ACP variant launches — `@agentclientprotocol/claude-agent-acp`, the Agent
/// Client Protocol org's adapter for the Claude Agent SDK. It drives the same `claude` the
/// native variant does (and honours the same `CLAUDE_CONFIG_DIR`), so everything provisioning
/// writes into the ephemeral dir still reaches the model; only the wire differs.
///
/// Chosen over Zed's `claude-code-acp`, which is the other adapter for the same job, because
/// this one carries the two things a column needs and that one drops: it sends real
/// `usage_update` notifications (`used`, `size` from the model's context window, and a
/// cumulative `cost`) and it speaks the native subagent-session extension, where Zed's
/// flattens a delegate's output into the main transcript with no marker at all.
const ACP_COMMAND: &str = "claude-agent-acp";

/// The Claude Code harness provisioner.
///
/// One provisioner, two harnesses. `acp` picks which wire a *structured* run speaks: the
/// native `-p --output-format stream-json` NDJSON stream ([`crate::io::JsonlBridge`]) or the
/// Agent Client Protocol ([`crate::io::AcpBridge`]) by way of [`ACP_COMMAND`]. Everything else
/// — the config-dir relocation, skills, MCP, memory, accounts, login, model discovery and the
/// whole passthrough argv — is the same harness and is shared rather than duplicated.
#[derive(Debug, Clone, Default)]
pub struct Claude {
    /// A structured run speaks ACP through the adapter rather than Claude Code's own stream.
    acp: bool,
}

impl Claude {
    /// Construct the Claude Code harness descriptor.
    pub fn new() -> Self {
        Claude { acp: false }
    }

    /// Construct the ACP-speaking Claude Code harness descriptor (`claude-code-acp`).
    pub fn new_acp() -> Self {
        Claude { acp: true }
    }

    /// The launch shared by [`Harness::provision`] and [`Harness::provision_home`]:
    /// `config_dir` becomes `CLAUDE_CONFIG_DIR` (none is set for `None`, a native run), and
    /// `config_args` — the flags naming this run's own files — go right after the headless flags.
    fn launch(
        &self,
        spec: &RunSpec,
        config_dir: Option<&Path>,
        config_args: Vec<String>,
    ) -> Result<Launch> {
        // 5. Build the launch. Structured mode launches Claude Code headless
        // (`-p --output-format stream-json --input-format stream-json`),
        // with the prompt delivered as an NDJSON line on stdin by the
        // bridge rather than a trailing positional argument; passthrough
        // mode keeps the interactive argv shape from P1.
        let structured = spec.io == crate::spec::IoModes::Structured;
        // The ACP variant's *structured* launch is the adapter, and the adapter takes no argv at
        // all: the prompt is a `session/prompt`, a resume is `session/load` (from
        // `Provisioned::resume`) and everything else it needs it reads from `CLAUDE_CONFIG_DIR`
        // below, exactly as the native variant's child does. A passthrough run is unchanged —
        // `claude-code-acp` is not a TUI, so a pane still gets the real `claude`.
        // ponytail: no thinking level and no `--mcp-config` reach the adapter by argv. The model
        // is set over the wire by the bridge; under a shared home the MCP servers ride
        // `session/new`'s `mcpServers` (`provision_acp_home`), on the legacy path none reach it.
        // Skills, settings and memory arrive through the config dir. `_meta.claudeCode.options`
        // would carry the rest per run, if that is ever wanted (`G378`).
        let acp = self.acp && structured;

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
        args.extend(config_args);
        // Model selection: `--model <id>` works in both passthrough and
        // structured invocation. Only added when a model is set, so runs
        // without `--model` keep byte-identical argv.
        if let Some(model) = &spec.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        // Reasoning effort: `--effort <value>` right after the model pair. Only added when set,
        // so runs without a chosen level keep byte-identical argv.
        if let Some(thinking) = &spec.thinking {
            args.push("--effort".to_string());
            args.push(thinking.clone());
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
            // The ask channel. Without `--permission-prompt-tool stdio` Claude emits no
            // `control_request` at all in `-p` mode and auto-*denies* every gated tool (the
            // refusal shows up only in `result.permission_denials[]`), so a caller that wants to
            // approve anything has to opt in here — see `_docs/harness/claude-code.md`
            // §"Tool approval in headless mode".
            args.push("--permission-prompt-tool".to_string());
            args.push("stdio".to_string());
            // A mode `resolve` wrote onto `spec.policy` (from `--permission-mode` or a `--safe`
            // preset) is still passed through, for the settings-level default it also renders.
            // Nothing is passed when nothing chose a mode: `-p` runs report
            // `permissionMode:"default"` whatever this flag says (verified against 2.1.258), so
            // the old `bypassPermissions` default bought no bypass — it only hid the fact that
            // the bridge, not the flag, decides what runs. With a prompt channel present that
            // decision belongs to the caller.
            let chosen_mode = spec
                .policy
                .as_ref()
                .and_then(|p| p.permission_mode.as_deref());
            if let Some(mode) = chosen_mode {
                args.push("--permission-mode".to_string());
                args.push(mode.to_string());
            }
            args.push("--disallowedTools".to_string());
            args.push("AskUserQuestion".to_string());
        }
        args.extend(spec.passthrough_args.iter().cloned());

        // Append prompt as trailing positional argument, passthrough mode
        // only — structured mode's bridge sends it as NDJSON on stdin.
        if !structured && let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
            args.push(prompt.clone());
        }

        // 6. Account: inject credential *references* into the child's env. A native run sets no
        // `CLAUDE_CONFIG_DIR` and does not strip an inherited one either: a user who exported it
        // in their shell made that dir their default, and a bare `claude` typed there reads it —
        // so does this run. The same holds nested inside another run, whose dir a `claude`
        // typed in its shell would also read.
        let mut env: Vec<(String, String)> = config_dir
            .map(|dir| ("CLAUDE_CONFIG_DIR".to_string(), dir.display().to_string()))
            .into_iter()
            .collect();
        if let Some(account) = &spec.account {
            if let Some(base_url) = &account.base_url {
                env.push(("ANTHROPIC_BASE_URL".to_string(), base_url.clone()));
            }
            if let Some(name) = &account.api_key_env {
                let value = super::shared::account_env(account, name)?;
                env.push(("ANTHROPIC_API_KEY".to_string(), value));
            }
            if let Some(name) = &account.auth_token_env {
                let value = super::shared::account_env(account, name)?;
                env.push(("ANTHROPIC_AUTH_TOKEN".to_string(), value));
            }
        }

        // 7. The wire. The ACP variant's structured launch is the adapter, and the adapter
        // takes no argv: the prompt is a `session/prompt`, a resume is `session/load` (from
        // `Provisioned::resume`), and the rest it reads out of `CLAUDE_CONFIG_DIR` exactly as
        // the native child does. A passthrough run is unchanged either way — `claude-code-acp`
        // is not a TUI, so a pane still gets the real `claude`.
        Ok(Launch {
            program: if acp { ACP_COMMAND } else { "claude" }.to_string(),
            args: if acp {
                spec.passthrough_args.clone()
            } else {
                args
            },
            env,
            env_remove: ENV_HYGIENE.iter().map(|s| s.to_string()).collect(),
            env_clear: false,
        })
    }

    /// A structured `claude-code-acp` run against a shared `CLAUDE_CONFIG_DIR` (`D193`). The
    /// adapter takes no argv, so the one per-run route is the wire: its MCP servers travel in
    /// `session/new`'s `mcpServers` ([`crate::provision::Provisioned::mcp_servers`], sent by
    /// [`Harness::structured_bridge`]), and nothing goes into `scratch`. Settings, instructions
    /// and skills have no per-run route (`G378`), so they are the profile's, written into `home`
    /// where the adapter's Claude Code reads its user config:
    ///
    /// - skills and MCP-as-skill pointers → `<home>/skills/<id>/`, each swapped in whole;
    /// - permissions, hooks, `apiKeyHelper` → those keys of `<home>/settings.json`, set in a
    ///   locked read-modify-write that keeps every other key;
    /// - instructions → `am`'s managed block in `<home>/CLAUDE.md`, the rest of the file kept.
    ///
    /// `RunSpec` cannot tell a profile's piece from a run's, so two runs of one profile that
    /// differ there overwrite each other. With no `home` (a native run) the launch sets no
    /// `CLAUDE_CONFIG_DIR` and all three are dropped with a warning — the user's own config is
    /// never written.
    fn provision_acp_home(&self, spec: &RunSpec, home: Option<&Path>) -> Result<Launch> {
        if spec.mcps.iter().any(|m| matches!(m, McpRef::InProcess(_))) {
            bail!("in-process MCP not supported in CLI/passthrough mode");
        }
        let instructions = spec.initial.as_ref().and_then(|i| i.instructions.as_ref());
        match home {
            Some(home) => {
                super::write_profile_skills(spec, &home.join("skills"))?;
                write_profile_settings(spec, home)?;
                if let Some(text) = instructions {
                    write_profile_instructions(&home.join("CLAUDE.md"), text)?;
                }
            }
            None => {
                let skills = spec.skills.len() + spec.mcp_as_skill.len();
                let settings = settings_keys(spec).len();
                if skills > 0 || settings > 0 || instructions.is_some() {
                    warn!(
                        skills,
                        settings,
                        instructions = instructions.is_some(),
                        "claude-code-acp has no per-run route for skills, settings or instructions; a run with no profile drops them"
                    );
                }
            }
        }
        self.launch(spec, home, Vec::new())
    }
}

impl Harness for Claude {
    fn id(&self) -> crate::spec::HarnessId {
        if self.acp {
            "claude-code-acp"
        } else {
            "claude-code"
        }
        .to_string()
    }

    fn display_name(&self) -> &str {
        if self.acp {
            "Claude Code (ACP)"
        } else {
            "Claude Code"
        }
    }

    fn command(&self) -> &str {
        if self.acp { ACP_COMMAND } else { "claude" }
    }

    fn aliases(&self) -> &[&str] {
        if self.acp {
            &["claude-acp"]
        } else {
            &["claude"]
        }
    }

    fn io_support(&self) -> super::IoSupport {
        super::IoSupport {
            passthrough: true,
            structured: true,
            multi_turn: true,
            acp: self.acp,
            // A stored OAuth token and one request, so the answer needs no process at all —
            // which is exactly the case that matters, since the question is asked *before* a
            // long run. The live bridge's own `RateLimitUpdate` is a fresher reading of the
            // same fact, not a second one.
            quota: super::QuotaSource::Probe,
        }
    }

    /// `claude --version`, for both variants. The ACP adapter has no version flag — it reads
    /// JSON-RPC off stdin the moment it starts, so probing it would hang — and it is a shim over
    /// the same binary anyway, so the installed Claude Code's version is the honest cache key.
    fn version(&self) -> Result<String> {
        let mut cmd = Command::new("claude");
        cmd.arg("--version");
        cmd.current_dir(super::shared::probe_cwd());
        #[cfg(windows)]
        super::shared::no_window(&mut cmd);
        let output = cmd
            .output()
            .context("running `claude --version` (is it on PATH?)")?;
        if !output.status.success() {
            anyhow::bail!(
                "`claude --version` failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().next().unwrap_or_default().trim().to_string())
    }

    /// Class A: `CLAUDE_CONFIG_DIR` relocates the entire config — credentials
    /// and `.claude.json` included (verified against Claude Code 2.1.206) —
    /// while the real `HOME` stays intact. See `_docs/profiles.md` §5. With the
    /// keychain reachable, Claude Code keeps the login in a generic password
    /// named `Claude Code-credentials-<sha256($CLAUDE_CONFIG_DIR)[:8]>`, stable
    /// for a profile's fixed home (`D193`); otherwise in `.credentials.json`.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: vec![("CLAUDE_CONFIG_DIR".to_string(), Relocate::All)],
            requires_home_relocation: false,
        }
    }

    /// Claude Code writes one JSONL transcript per conversation under
    /// `<config-dir>/projects/<slugged-cwd>/<session-uuid>.jsonl`. Everything
    /// directly under a project dir that ends in `.jsonl` is one such record.
    fn transcripts(&self, config_dir: &Path) -> Vec<std::path::PathBuf> {
        let Ok(projects) = std::fs::read_dir(config_dir.join("projects")) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        for project in projects.flatten() {
            let Ok(files) = std::fs::read_dir(project.path()) else {
                continue;
            };
            for file in files.flatten() {
                let path = file.path();
                if path.extension().is_some_and(|ext| ext == "jsonl") {
                    found.push(path);
                }
            }
        }
        found
    }

    /// `<home>/projects/<slug(cwd)>/<session_id>.jsonl`, plus the `<session_id>/` directory
    /// beside it when there is one (Claude Code keeps a session's subagent transcripts there).
    /// With no `config_home` the root is Claude Code's own default, [`default_config_dir`]. The
    /// slug is Claude Code's — every character but an ASCII letter or digit becomes `-` — and a
    /// cwd it shortens past that (a very long path) is found by the id alone, under any project.
    fn session_transcripts(
        &self,
        config_home: Option<&Path>,
        cwd: &Path,
        session_id: &str,
    ) -> Option<Vec<std::path::PathBuf>> {
        let Some(root) = config_home
            .map(Path::to_path_buf)
            .or_else(default_config_dir)
        else {
            return Some(Vec::new());
        };
        // A session id is one path segment; anything else names no session of ours.
        if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id == ".." {
            return Some(Vec::new());
        }
        let projects = root.join("projects");
        let file = format!("{session_id}.jsonl");
        let slugged = projects.join(project_slug(cwd)).join(&file);
        let jsonl = if slugged.is_file() {
            Some(slugged)
        } else {
            std::fs::read_dir(&projects).ok().and_then(|dirs| {
                dirs.flatten()
                    .map(|project| project.path().join(&file))
                    .find(|path| path.is_file())
            })
        };
        let Some(jsonl) = jsonl else {
            return Some(Vec::new());
        };
        let mut found = vec![jsonl.clone()];
        let companion = jsonl.with_extension("");
        if companion.is_dir() {
            found.push(companion);
        }
        Some(found)
    }

    /// Live model list via headless stream-json + the `/model` slash command.
    ///
    /// Claude Code has no dedicated list/JSON CLI. The preferred path (see
    /// `_docs/harness/claude-code.md` §"Model discovery & selection") is to
    /// launch `claude -p` with stream-json I/O, write a single NDJSON user
    /// line whose text is `"/model"`, and parse the synthetic free-text
    /// `Available: …` clause. That path is zero-token (`message.model:
    /// "<synthetic>"`) and preferred over plain `claude -p "/model"` for
    /// subscription/orchestration reasons. Requires `claude` on `PATH` and a
    /// working auth for process launch (the slash command itself does not
    /// bill tokens).
    fn discover_models(&self) -> Result<Vec<super::ModelInfo>> {
        discover_models_via_jsonl()
    }

    /// `claude --help` advertises `--effort <level>` with the accepted set in
    /// parentheses (see [`parse_effort_levels`]). Applied uniformly to every model
    /// [`Harness::discover_models`] returns: our Claude model ids are the aliases
    /// scraped from `/model` (`opus`, `sonnet`), not full API ids, so there is no
    /// per-model allow table to key a subset on — over-offering and letting the
    /// CLI reject an unsupported level is the right fallback. `default_level` is
    /// always `None`: `--help` names no default.
    fn discover_thinking(&self) -> Result<BTreeMap<String, super::ModelThinking>> {
        let models = self.discover_models()?;
        let mut cmd = Command::new("claude");
        cmd.arg("--help");
        cmd.current_dir(super::shared::probe_cwd());
        #[cfg(windows)]
        super::shared::no_window(&mut cmd);
        let output = cmd
            .output()
            .with_context(|| "running `claude --help` (is the claude binary on PATH?)")?;
        if !output.status.success() {
            bail!(
                "`claude --help` failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let help = String::from_utf8_lossy(&output.stdout);
        let mut values = parse_effort_levels(&help);
        if values.is_empty() {
            // `--help` didn't advertise the flag's accepted set; fall back to the
            // levels every observed build has supported, so the picker still works.
            values = vec!["low".to_string(), "medium".to_string(), "high".to_string()];
        }
        let levels: Vec<super::ThinkingLevel> = values
            .into_iter()
            .map(|value| super::ThinkingLevel {
                label: super::effort_label(&value),
                value,
                description: None,
            })
            .collect();
        Ok(models
            .into_iter()
            .map(|m| {
                (
                    m.id,
                    super::ModelThinking {
                        levels: levels.clone(),
                        default_level: None,
                    },
                )
            })
            .collect())
    }

    /// The six modes `claude --help`'s `--permission-mode <mode>` choices list (verified against
    /// Claude Code 2.1.261). Fixed CLI enum, not probed — see [`super::Harness::modes`].
    fn modes(&self) -> Vec<super::ModeInfo> {
        [
            ("acceptEdits", "Accept edits"),
            ("auto", "Auto"),
            ("bypassPermissions", "Bypass permissions"),
            ("manual", "Manual"),
            ("dontAsk", "Don't ask"),
            ("plan", "Plan"),
        ]
        .into_iter()
        .map(|(id, label)| super::ModeInfo {
            id: id.to_string(),
            label: label.to_string(),
            description: None,
        })
        .collect()
    }

    /// `bypassPermissions` is Claude Code's own name for asking nothing — see
    /// [`super::Harness::unattended_mode`].
    fn unattended_mode(&self) -> Option<&'static str> {
        Some("bypassPermissions")
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // 1. Skills: copy each skill folder into <dir>/skills/<id>/.
        write_skills(spec, &dir.join("skills"))?;

        // 2. MCP: always write <dir>/mcp.json, even if empty, so
        // --strict-mcp-config yields a fully-controlled server set.
        let mcp_path = write_mcp_json(spec, dir)?;

        // 3. Policy + account helper + hooks: <dir>/settings.json, written
        // when any of a policy, an account helper, or hooks is present.
        write_settings_json(spec, dir)?;

        // 4. Instructions: <dir>/CLAUDE.md, wrapped in a managed block.
        if let Some(instr_text) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
            let claude_md = format!("{MANAGED_BEGIN}\n{}\n{MANAGED_END}\n", instr_text);
            let claude_md_path = dir.join("CLAUDE.md");
            std::fs::write(&claude_md_path, claude_md)
                .with_context(|| format!("writing {}", claude_md_path.display()))?;
        }

        let config_args = vec![
            "--mcp-config".to_string(),
            mcp_path.display().to_string(),
            "--strict-mcp-config".to_string(),
        ];
        let launch = self.launch(spec, Some(dir), config_args)?;
        Ok(launch)
    }

    /// Both variants run from a shared home. `claude-code-acp`'s adapter takes no argv, so its
    /// MCP servers go over the wire and the rest is profile-owned — see `provision_acp_home`.
    fn shares_home(&self) -> bool {
        true
    }

    /// [`default_config_dir`], and with no `CLAUDE_CONFIG_DIR` the `~/.claude.json` Claude Code
    /// keeps beside it in `$HOME`, which that variable otherwise moves inside the dir.
    fn default_homes(&self) -> Vec<std::path::PathBuf> {
        let mut homes: Vec<_> = default_config_dir().into_iter().collect();
        if std::env::var_os("CLAUDE_CONFIG_DIR").is_none_or(|dir| dir.is_empty()) {
            homes.extend(directories::BaseDirs::new().map(|b| b.home_dir().join(".claude.json")));
        }
        homes
    }

    /// A run against a profile's shared `CLAUDE_CONFIG_DIR` (`D193`). Nothing per-run is written
    /// into `home`; each per-run file goes into `scratch` and is passed by flag:
    ///
    /// - MCP → `<scratch>/mcp.json`, `--mcp-config … --strict-mcp-config`, as a fixed dir has it;
    /// - permissions, hooks, `apiKeyHelper` → `<scratch>/settings.json`, `--settings <file>`. No
    ///   `--setting-sources`: the home's own user settings (the theme and TUI templates) keep
    ///   applying, and `--settings` layers over them;
    /// - instructions → `--append-system-prompt <text>`, plain — the managed-block markers only
    ///   separate `am`'s text from a user's inside a shared `CLAUDE.md`;
    /// - skills and MCP-as-skill pointers → a per-run plugin, `<scratch>/plugin/` with
    ///   `.claude-plugin/plugin.json` and `skills/<id>/`, passed by `--plugin-dir` only when
    ///   there is a skill. Claude Code names a plugin's skill `<plugin>:<skill>`, so a skill
    ///   here is `am:<id>` (layout checked against 2.1.283's `claude plugin details`).
    ///
    /// No login is seeded: it lives in the home, put there by [`Harness::login_home`] or by the
    /// first terminal run's own login screen. With no `home` (a native run) the launch sets no
    /// `CLAUDE_CONFIG_DIR`, and Claude Code runs from the user's own config and login.
    ///
    /// A structured `claude-code-acp` run takes none of those flags — see `provision_acp_home`.
    /// Its passthrough pane is the real `claude`, composed exactly as here.
    fn provision_home(
        &self,
        spec: &RunSpec,
        home: Option<&Path>,
        scratch: &Path,
    ) -> Result<Launch> {
        if self.acp && spec.io == crate::spec::IoModes::Structured {
            return self.provision_acp_home(spec, home);
        }
        let mcp_path = write_mcp_json(spec, scratch)?;
        let mut config_args = vec![
            "--mcp-config".to_string(),
            mcp_path.display().to_string(),
            "--strict-mcp-config".to_string(),
        ];
        if let Some(settings_path) = write_settings_json(spec, scratch)? {
            config_args.push("--settings".to_string());
            config_args.push(settings_path.display().to_string());
        }
        if let Some(instr_text) = spec.initial.as_ref().and_then(|i| i.instructions.as_ref()) {
            config_args.push("--append-system-prompt".to_string());
            config_args.push(instr_text.clone());
        }
        let plugin_dir = scratch.join("plugin");
        if write_skills(spec, &plugin_dir.join("skills"))? {
            let manifest_path = plugin_dir.join(".claude-plugin").join("plugin.json");
            std::fs::create_dir_all(manifest_path.parent().unwrap_or(&plugin_dir))
                .with_context(|| format!("creating {}", plugin_dir.display()))?;
            let manifest = json!({
                "name": PLUGIN_NAME,
                "description": "This run's skills, composed by agent-manager.",
            });
            std::fs::write(&manifest_path, serde_json::to_string_pretty(&manifest)?)
                .with_context(|| format!("writing {}", manifest_path.display()))?;
            config_args.push("--plugin-dir".to_string());
            config_args.push(plugin_dir.display().to_string());
        }
        self.launch(spec, home, config_args)
    }

    /// `claude auth login` with `CLAUDE_CONFIG_DIR=<home>`, and the real `HOME` left alone: the
    /// login lands where every run of the profile reads it, in the file or the Keychain item
    /// Claude Code itself picks for that home, and Claude Code refreshes it from then on.
    fn login_home(&self, home: &Path) -> Result<Launch> {
        Ok(Launch {
            program: "claude".to_string(),
            args: vec!["auth".to_string(), "login".to_string()],
            env: vec![("CLAUDE_CONFIG_DIR".to_string(), home.display().to_string())],
            env_remove: ENV_HYGIENE.iter().map(|s| s.to_string()).collect(),
            env_clear: false,
        })
    }

    /// One request against the OAuth token Claude Code keeps in `home`. Unofficial and
    /// rate-limited, so a failure is a sentence the user reads and never a number —
    /// `crate::quota::claude` owns the endpoint, the header and the defensive parse, because
    /// naming a provider's URL is this library's job and nobody else's.
    fn quota(&self, account: &str, home: Option<&Path>) -> Result<crate::quota::QuotaSnapshot> {
        crate::quota::claude(account, &self.id(), home)
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        if self.acp {
            // The adapter's only per-run MCP route. Empty on the legacy path.
            return Ok(Box::new(crate::io::AcpBridge::with_mcp_servers(
                child,
                cwd,
                provisioned.resume.as_deref(),
                provisioned.model.as_deref(),
                &provisioned.mcp_servers,
            )?));
        }
        Ok(Box::new(crate::io::JsonlBridge::new(child)?))
    }

    /// User-editable preference defaults, merged into the run by
    /// [`super::apply_templates`] rather than hardcoded here — see
    /// `~/.config/agent-manager/templates/claude-code/`. `settings.json`
    /// carries `theme`/`tui` (unset otherwise triggers Claude Code's
    /// first-run theme picker); `.claude.json` carries
    /// `claudeInChromeDefaultEnabled` (unset triggers the Claude-in-Chrome
    /// opt-in prompt). Both are genuine user preferences, unlike the
    /// structural fix-ups in [`Claude::post_seed`].
    fn templates(&self) -> Vec<super::TemplateFile> {
        vec![
            super::TemplateFile {
                name: "settings.json",
                default: || json!({ "theme": "dark", "tui": "fullscreen" }),
            },
            super::TemplateFile {
                name: ".claude.json",
                default: || json!({ "claudeInChromeDefaultEnabled": false }),
            },
        ]
    }

    /// Structural fix-ups to `.claude.json`, always forced
    /// (never template-overridable, unlike [`Claude::templates`] — these
    /// aren't preferences, they're correctness requirements for `am`'s
    /// ephemeral-config model):
    ///
    /// 1. A login made by `claude auth login` never runs the interactive
    ///    onboarding wizard, so the file lacks `hasCompletedOnboarding`, and
    ///    Claude Code, fully authenticated, still opens its onboarding UI.
    /// 2. Claude Code gates a per-project trust dialog on
    ///    `projects[cwd].hasTrustDialogAccepted`, keyed by the exact cwd
    ///    string. A fresh/ephemeral `CLAUDE_CONFIG_DIR` has no record of
    ///    `spec.cwd`, so every run would otherwise hit that dialog too.
    ///
    /// Under a shared home ([`crate::spec::ConfigStrategy::Home`], with `dir` the home) both
    /// are added only when missing, under a lock — see `post_seed_shared_home`.
    fn post_seed(&self, spec: &RunSpec, dir: &Path) -> Result<()> {
        if matches!(&spec.config, crate::spec::ConfigStrategy::Home { home, .. } if home == dir) {
            return post_seed_shared_home(spec, dir);
        }
        debug!(dir = %dir.display(), "post_seed: fixing up .claude.json");
        let path = dir.join(".claude.json");
        let mut doc: Value = match std::fs::read_to_string(&path) {
            Ok(raw) => {
                serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?
            }
            Err(_) => json!({}),
        };
        let Value::Object(map) = &mut doc else {
            return Ok(());
        };

        map.insert("hasCompletedOnboarding".to_string(), json!(true));

        let projects = map
            .entry("projects")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Value::Object(projects) = projects {
            let cwd_key = spec.cwd.display().to_string();
            let entry = projects
                .entry(cwd_key)
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if let Value::Object(entry) = entry {
                entry.insert("hasTrustDialogAccepted".to_string(), json!(true));
            }
        }

        std::fs::write(&path, serde_json::to_string_pretty(&doc)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

/// Claude Code's config dir when `am` names none: `$CLAUDE_CONFIG_DIR` when the environment
/// sets it (a native run inherits it — see `Claude::launch`), else `~/.claude`.
fn default_config_dir() -> Option<std::path::PathBuf> {
    super::env_dir_or_home("CLAUDE_CONFIG_DIR", ".claude")
}

/// The directory name Claude Code files a cwd's transcripts under in `projects/`: the path
/// with every character but an ASCII letter or digit replaced by `-` (`/tmp/a.b` →
/// `-tmp-a-b`).
fn project_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
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

/// Copy each of `spec.skills` into `<skills_dir>/<id>/`, and write the MCP-as-skill pointers
/// beside them (stepping stone; see `harness::write_mcp_as_skill_pointers`'s doc). Touches
/// nothing when there is neither; answers whether anything was written.
fn write_skills(spec: &RunSpec, skills_dir: &Path) -> Result<bool> {
    for skill in &spec.skills {
        let dest = skills_dir.join(&skill.id);
        skill
            .source
            .materialize(&dest, crate::source::LinkMode::Copy, true)
            .with_context(|| format!("copying skill '{}' into {}", skill.id, dest.display()))?;
    }
    super::write_mcp_as_skill_pointers(spec, skills_dir)?;
    Ok(!spec.skills.is_empty() || !spec.mcp_as_skill.is_empty())
}

/// Write `<dir>/mcp.json` from `spec.mcps` — always, even if empty, so `--strict-mcp-config`
/// yields a fully-controlled server set. Returns the path written.
fn write_mcp_json(spec: &RunSpec, dir: &Path) -> Result<std::path::PathBuf> {
    let mcp_json = build_mcp_json(&spec.mcps)?;
    let mcp_path = dir.join("mcp.json");
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&mcp_json)?)
        .with_context(|| format!("writing {}", mcp_path.display()))?;
    Ok(mcp_path)
}

/// Write `<dir>/settings.json` when any of a policy, an account helper, or hooks is present.
/// Returns the path written, or `None` when there was nothing to write.
fn write_settings_json(spec: &RunSpec, dir: &Path) -> Result<Option<std::path::PathBuf>> {
    let settings_obj = settings_keys(spec);
    if settings_obj.is_empty() {
        return Ok(None);
    }
    let settings_path = dir.join("settings.json");
    std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&Value::Object(settings_obj))?,
    )
    .with_context(|| format!("writing {}", settings_path.display()))?;
    Ok(Some(settings_path))
}

/// The `settings.json` keys `am` owns, from `spec`: `permissions` (a policy), `apiKeyHelper`
/// (an account helper) and `hooks`, each only when the spec has one.
fn settings_keys(spec: &RunSpec) -> serde_json::Map<String, Value> {
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
    settings_obj
}

/// Set the keys `am` owns ([`settings_keys`]) in a shared home's `settings.json`, as
/// profile-owned content for `claude-code-acp` (`D193`, `G378`): a read-modify-write under a
/// lock beside the file, every other key — the theme and TUI templates, the user's own — left
/// as it was, and nothing rewritten when the values are already there. A key the spec does not
/// set is left alone too, so a policy dropped from the profile stays until changed by hand.
fn write_profile_settings(spec: &RunSpec, home: &Path) -> Result<()> {
    let owned = settings_keys(spec);
    if owned.is_empty() {
        return Ok(());
    }
    let path = home.join("settings.json");
    // Released when `_lock` drops, at the end of this function.
    let _lock = super::lock_beside(&path)?;
    let mut doc: Value = match std::fs::read_to_string(&path) {
        Ok(raw) => {
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let Value::Object(map) = &mut doc else {
        warn!(path = %path.display(), "settings.json is not an object; left alone");
        return Ok(());
    };
    let mut changed = false;
    for (key, value) in owned {
        if map.get(&key) != Some(&value) {
            map.insert(key, value);
            changed = true;
        }
    }
    if changed {
        replace_in_home(&path, &serde_json::to_string_pretty(&doc)?)?;
    }
    Ok(())
}

/// Put `text` into a shared home's `CLAUDE.md` as `am`'s managed block, as profile-owned
/// instructions for `claude-code-acp` (`D193`, `G378`): the block replaced in place when the
/// file has one, appended when it has not, and the rest of the file — the user's own memory —
/// kept byte for byte. A read-modify-write under a lock beside the file.
fn write_profile_instructions(path: &Path, text: &str) -> Result<()> {
    // Released when `_lock` drops, at the end of this function.
    let _lock = super::lock_beside(path)?;
    let current = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let block = format!("{MANAGED_BEGIN}\n{text}\n{MANAGED_END}\n");
    let next = match (current.find(MANAGED_BEGIN), current.find(MANAGED_END)) {
        (Some(start), Some(end)) if end > start => {
            let mut tail = end + MANAGED_END.len();
            if current[tail..].starts_with('\n') {
                tail += 1;
            }
            format!("{}{block}{}", &current[..start], &current[tail..])
        }
        _ if current.is_empty() => block,
        _ => {
            let sep = if current.ends_with('\n') {
                "\n"
            } else {
                "\n\n"
            };
            format!("{current}{sep}{block}")
        }
    };
    if next != current {
        replace_in_home(path, &next)?;
    }
    Ok(())
}

/// Write `contents` beside `path` and rename it over, so a Claude Code running from the same
/// home never reads half a file. The caller holds the lock beside `path`.
fn replace_in_home(path: &Path, contents: &str) -> Result<()> {
    let mut staged = path.as_os_str().to_owned();
    staged.push(".am-tmp");
    let staged = std::path::PathBuf::from(staged);
    std::fs::write(&staged, contents).with_context(|| format!("writing {}", staged.display()))?;
    std::fs::rename(&staged, path).with_context(|| format!("writing {}", path.display()))
}

/// [`Claude::post_seed`] for a shared home: the onboarding flag and the cwd's trust entry are
/// added to `<home>/.claude.json` **only when missing**, as a read-modify-write under an
/// exclusive lock on a sibling lock file, written beside the file and renamed onto it. Every
/// other key — the login's identity, other runs' projects, Claude Code's own state — is left
/// as it was, and a document already holding both is not rewritten at all.
fn post_seed_shared_home(spec: &RunSpec, home: &Path) -> Result<()> {
    let path = home.join(".claude.json");
    // Released when `_lock` drops, at the end of this function.
    let _lock = super::lock_beside(&path)?;

    let mut doc: Value = match std::fs::read_to_string(&path) {
        Ok(raw) => {
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let Value::Object(map) = &mut doc else {
        warn!(path = %path.display(), "post_seed: .claude.json is not an object; left alone");
        return Ok(());
    };

    let mut changed = false;
    if !map.contains_key("hasCompletedOnboarding") {
        map.insert("hasCompletedOnboarding".to_string(), json!(true));
        changed = true;
    }
    let projects = map
        .entry("projects")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Value::Object(projects) = projects {
        let entry = projects
            .entry(spec.cwd.display().to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Value::Object(entry) = entry
            && !entry.contains_key("hasTrustDialogAccepted")
        {
            entry.insert("hasTrustDialogAccepted".to_string(), json!(true));
            changed = true;
        }
    }
    if changed {
        debug!(path = %path.display(), "post_seed: adding missing keys to a shared .claude.json");
        super::write_credential(&path, serde_json::to_string_pretty(&doc)?.as_bytes())?;
    }
    Ok(())
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

/// Shell out to Claude Code stream-json with prompt `/model` and parse the
/// synthetic available-alias list. See `_docs/harness/claude-code.md`
/// §"Model discovery & selection".
fn discover_models_via_jsonl() -> Result<Vec<ModelInfo>> {
    let mut cmd = Command::new("claude");
    cmd.args([
        "-p",
        "--output-format",
        "stream-json",
        "--input-format",
        "stream-json",
        // stream-json output requires --verbose when using -p.
        "--verbose",
        "--permission-mode",
        "bypassPermissions",
        "--max-turns",
        "1",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    // Strip nested-session identity so discovery doesn't inherit a parent
    // Claude Code session (same hygiene as provisioned launches).
    for key in ENV_HYGIENE {
        cmd.env_remove(key);
    }
    // A real session's cwd is `RunSpec::cwd`, a project folder the caller chose. This is a
    // probe, not a session, so it runs in agent-manager's own scratch dir instead of whatever
    // folder the embedding app happens to be running from — see `shared::probe_cwd`.
    cmd.current_dir(super::shared::probe_cwd());
    #[cfg(windows)]
    super::shared::no_window(&mut cmd);
    let mut child = cmd.spawn().with_context(
        || "spawning `claude` for model discovery via stream-json (is the claude binary on PATH?)",
    )?;

    let prompt = json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [{"type": "text", "text": "/model"}],
        },
    });
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("claude stdin not piped"))?;
        writeln!(stdin, "{prompt}").context("writing /model prompt to claude stdin")?;
        // Drop closes stdin so Claude sees EOF after the single user line.
    }

    let output = child
        .wait_with_output()
        .context("waiting for claude stream-json /model discovery")?;
    if !output.status.success() {
        bail!(
            "`claude` stream-json /model discovery failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let text = extract_slash_result_text(&stdout).ok_or_else(|| {
        anyhow::anyhow!(
            "no /model result text in claude stream-json stdout (got {} bytes); stderr: {}",
            output.stdout.len(),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    })?;

    let models = parse_model_slash_output(&text);
    if models.is_empty() {
        bail!("could not parse any model ids from claude /model output: {text:?}");
    }
    Ok(models)
}

/// Pull free-text from a stream-json NDJSON stdout for a synthetic slash
/// command: prefer `result.result`, fall back to the first assistant text
/// block.
fn extract_slash_result_text(stdout: &str) -> Option<String> {
    let mut assistant_text: Option<String> = None;
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("result") => {
                if let Some(r) = v.get("result").and_then(|r| r.as_str()) {
                    return Some(r.to_string());
                }
            }
            Some("assistant") if assistant_text.is_none() => {
                if let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) {
                    for block in content {
                        if block.get("type").and_then(|t| t.as_str()) == Some("text")
                            && let Some(t) = block.get("text").and_then(|t| t.as_str())
                        {
                            assistant_text = Some(t.to_string());
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    assistant_text
}

/// Parse the free-text body of a headless `/model` response into
/// [`ModelInfo`] entries.
///
/// Expected shape (verified Claude Code ≥ 2.1.207):
///
/// ```text
/// Current model: Opus 4.8 (effort: high)
/// Usage: /model <name>. Available: sonnet, opus, haiku, …, default, or a full model ID.
/// ```
///
/// The `default` alias (if present) is marked [`ModelInfo::default`]. The
/// `Current model: …` line, when present, is attached as the description on
/// that default entry (or left unused if `default` is absent).
fn parse_model_slash_output(text: &str) -> Vec<ModelInfo> {
    let current_line = text
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("Current model:"))
        .map(str::to_string);

    let Some(available_part) = text.split("Available:").nth(1) else {
        return Vec::new();
    };
    // Take only the first line of the Available clause (defensive against
    // future multi-paragraph responses).
    let available_part = available_part.lines().next().unwrap_or(available_part);
    let cleaned = available_part
        .trim()
        .trim_end_matches('.')
        .replace(", or a full model ID", "")
        .replace("or a full model ID", "");

    let mut models = Vec::new();
    for part in cleaned.split(',') {
        let id = part.trim();
        if id.is_empty() {
            continue;
        }
        let mut info = ModelInfo::new(id);
        if id == "default" {
            info = info.as_default();
            if let Some(ref cur) = current_line {
                info = info.with_description(cur.clone());
            }
        }
        models.push(info);
    }
    models
}

/// `claude --help` advertises `--effort <level>` with the accepted set in parentheses.
/// Verified against 2.1.261, where the parenthetical wraps onto the following line, so the
/// scan runs over the whole help text rather than per line. Returns an empty vec (rather than
/// a hard-coded fallback) when `--effort` or its parenthetical is absent — the caller decides
/// what an unadvertised build should fall back to.
fn parse_effort_levels(help: &str) -> Vec<String> {
    let Some(flag_pos) = help.find("--effort") else {
        return Vec::new();
    };
    let rest = &help[flag_pos..];
    let Some(open) = rest.find('(') else {
        return Vec::new();
    };
    let after_open = &rest[open + 1..];
    let Some(close) = after_open.find(')') else {
        return Vec::new();
    };
    after_open[..close]
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{McpServer, McpTransport};
    use crate::spec::{ConfigStrategy, McpRef, Policy, SkillRef};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    super::super::shared::harness_conformance_tests!(Claude, "claude-code");

    /// The harness's own record is what an embedder archives, so it must find
    /// the per-project JSONL files and nothing else that shares the directory.
    #[test]
    fn transcripts_finds_project_jsonl_and_ignores_other_files() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let project = config_dir.path().join("projects/-tmp-project");
        std::fs::create_dir_all(&project).unwrap();
        let transcript = project.join("2f0c1e6a-0000-4000-8000-000000000000.jsonl");
        std::fs::write(&transcript, b"{}\n").unwrap();
        std::fs::write(project.join("notes.txt"), b"not a transcript").unwrap();

        assert_eq!(
            Claude::new().transcripts(config_dir.path()),
            vec![transcript]
        );
    }

    /// One session out of a shared home: its own transcript under the cwd's slug and the
    /// companion dir beside it, never another run's; a slug Claude Code shortened is found by
    /// the id alone; an unknown or unsafe id finds nothing.
    #[test]
    fn session_transcripts_names_one_sessions_files_in_a_shared_home() {
        let home = tempfile::TempDir::new().unwrap();
        let cwd = Path::new("/tmp/my.project");
        assert_eq!(project_slug(cwd), "-tmp-my-project");
        let project = home.path().join("projects/-tmp-my-project");
        std::fs::create_dir_all(project.join("sess-a/subagents")).unwrap();
        std::fs::write(project.join("sess-a.jsonl"), b"{}\n").unwrap();
        std::fs::write(project.join("sess-b.jsonl"), b"{}\n").unwrap();
        let elsewhere = home.path().join("projects/-shortened-1a2b");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("sess-c.jsonl"), b"{}\n").unwrap();

        let claude = Claude::new();
        let found = |id: &str| claude.session_transcripts(Some(home.path()), cwd, id);
        assert_eq!(
            found("sess-a"),
            Some(vec![project.join("sess-a.jsonl"), project.join("sess-a")])
        );
        assert_eq!(found("sess-b"), Some(vec![project.join("sess-b.jsonl")]));
        assert_eq!(found("sess-c"), Some(vec![elsewhere.join("sess-c.jsonl")]));
        assert_eq!(found("sess-z"), Some(Vec::new()));
        assert_eq!(found("../projects"), Some(Vec::new()));
    }

    /// A native run: no `CLAUDE_CONFIG_DIR` in the launch, every per-run file passed by flag
    /// from scratch, and no trust or onboarding write anywhere.
    #[test]
    fn provision_home_without_a_home_names_no_config_dir() {
        let scratch = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("/tmp/p"));
        spec.config = crate::spec::ConfigStrategy::Native {
            scratch: scratch.path().to_path_buf(),
        };
        spec.initial = Some(crate::spec::Instructions {
            instructions: Some("BE BRIEF".to_string()),
            prompt: None,
        });

        let launch = Claude::new()
            .provision_home(&spec, None, scratch.path())
            .unwrap();

        assert!(launch.env.iter().all(|(k, _)| k != "CLAUDE_CONFIG_DIR"));
        assert_eq!(
            flag_value(&launch.args, "--append-system-prompt"),
            Some("BE BRIEF")
        );
        assert!(!scratch.path().join(".claude.json").exists());
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
        assert_eq!(servers["postgres"]["args"].as_array().unwrap().len(), 1);
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
        assert!(launch.env.iter().any(
            |(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &config_dir.path().display().to_string()
        ));
        assert!(launch.env_remove.contains(&"CLAUDECODE".to_string()));

        // Invariant: nothing written under the stand-in home dir.
        let home_entries: Vec<_> = std::fs::read_dir(fake_home.path()).unwrap().collect();
        assert!(
            home_entries.is_empty(),
            "expected no writes under the fake home dir, found: {home_entries:?}"
        );
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
            mcp_json
                .get("mcpServers")
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
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

        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "ANTHROPIC_API_KEY" && v == &expected)
        );

        // No-secret-on-disk invariant: walk the whole ephemeral dir and
        // confirm the secret value never landed in any file `am` wrote.
        super::super::shared::assert_no_secret_on_disk(config_dir.path(), &expected);
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
        // The permission ask channel, not a bypass: without it Claude never asks.
        let prompt_tool = launch
            .args
            .windows(2)
            .any(|w| w[0] == "--permission-prompt-tool" && w[1] == "stdio");
        assert!(
            prompt_tool,
            "expected `--permission-prompt-tool stdio` in argv: {:?}",
            launch.args
        );
        assert!(!launch.args.contains(&"bypassPermissions".to_string()));
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
        // The stdio prompt channel is the structured bridge's; a tty run asks in the tty.
        assert!(
            !launch
                .args
                .contains(&"--permission-prompt-tool".to_string())
        );
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
    fn provision_injects_effort_flag_right_after_model_pair_when_thinking_set() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.model = Some("sonnet".to_string());
        spec.thinking = Some("high".to_string());

        let launch = Claude::new().provision(&spec, config_dir.path()).unwrap();

        let model_idx = launch
            .args
            .iter()
            .position(|a| a == "--model")
            .expect("--model present");
        assert_eq!(launch.args[model_idx + 1], "sonnet");
        assert_eq!(launch.args[model_idx + 2], "--effort");
        assert_eq!(launch.args[model_idx + 3], "high");
    }

    #[test]
    fn provision_without_thinking_has_no_effort_flag() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());

        let launch = Claude::new().provision(&spec, config_dir.path()).unwrap();
        assert!(
            !launch.args.iter().any(|a| a == "--effort"),
            "no --effort expected when spec.thinking is None: {:?}",
            launch.args
        );
    }

    #[test]
    fn provision_structured_io_honors_spec_policy_permission_mode() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;
        spec.policy = Some(crate::spec::Policy {
            permission_mode: Some("plan".to_string()),
            ..Default::default()
        });

        let launch = Claude::new().provision(&spec, config_dir.path()).unwrap();
        let pair = launch
            .args
            .windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "plan");
        assert!(
            pair,
            "expected `--permission-mode plan` in argv: {:?}",
            launch.args
        );
    }

    /// With a prompt channel present, an unattended structured run no longer silently bypasses:
    /// nothing chose a mode, so nothing is passed, and the caller answers the asks.
    #[test]
    fn provision_structured_io_does_not_bypass_permissions_with_no_policy() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;

        let launch = Claude::new().provision(&spec, config_dir.path()).unwrap();
        assert!(
            !launch.args.contains(&"--permission-mode".to_string()),
            "no --permission-mode expected when no policy chose one: {:?}",
            launch.args
        );
        assert!(
            !launch.args.contains(&"bypassPermissions".to_string()),
            "structured runs must not bypass by default: {:?}",
            launch.args
        );
        let prompt_tool = launch
            .args
            .windows(2)
            .any(|w| w[0] == "--permission-prompt-tool" && w[1] == "stdio");
        assert!(
            prompt_tool,
            "expected `--permission-prompt-tool stdio` in argv: {:?}",
            launch.args
        );
    }

    #[test]
    fn every_mode_id_survives_its_own_provision_path() {
        for mode in Claude::new().modes() {
            let config_dir = tempfile::TempDir::new().unwrap();
            let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
            spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
            spec.io = crate::spec::IoModes::Structured;
            spec.policy = Some(crate::spec::Policy {
                permission_mode: Some(mode.id.clone()),
                ..Default::default()
            });

            let launch = Claude::new().provision(&spec, config_dir.path()).unwrap();
            let pair = launch
                .args
                .windows(2)
                .any(|w| w[0] == "--permission-mode" && w[1] == mode.id);
            assert!(
                pair,
                "mode '{}' did not reach argv: {:?}",
                mode.id, launch.args
            );
        }
    }

    /// A spec carrying one of everything a shared-home run passes by flag.
    fn home_spec(home: &Path, scratch: &Path, skill_dir: &Path) -> RunSpec {
        use crate::spec::Instructions;

        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("/tmp/project"));
        spec.config = ConfigStrategy::Home {
            home: home.to_path_buf(),
            scratch: scratch.to_path_buf(),
        };
        spec.skills.push(SkillRef {
            id: "my-skill".to_string(),
            source: crate::source::Source::Dir(skill_dir.to_path_buf()),
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
        spec.policy = Some(Policy {
            permission_mode: Some("plan".to_string()),
            ..Default::default()
        });
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER ME".to_string()),
            prompt: None,
        });
        spec
    }

    fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        let at = args.iter().position(|a| a == flag)?;
        args.get(at + 1).map(String::as_str)
    }

    #[test]
    fn provision_home_puts_every_per_run_file_in_scratch_and_passes_it_by_flag() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");
        let spec = home_spec(home.path(), scratch.path(), &skill_path);

        let launch = Claude::new()
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();

        // Nothing per-run lands in the shared home.
        let home_entries: Vec<_> = std::fs::read_dir(home.path()).unwrap().collect();
        assert!(home_entries.is_empty(), "home got: {home_entries:?}");

        let mcp_path = scratch.path().join("mcp.json");
        let settings_path = scratch.path().join("settings.json");
        let plugin_dir = scratch.path().join("plugin");
        assert!(mcp_path.is_file());
        assert!(settings_path.is_file());
        assert!(plugin_dir.join("skills/my-skill/SKILL.md").is_file());
        let manifest: Value = serde_json::from_str(
            &std::fs::read_to_string(plugin_dir.join(".claude-plugin/plugin.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["name"].as_str(), Some(PLUGIN_NAME));
        assert!(!scratch.path().join("CLAUDE.md").exists());

        let args = &launch.args;
        assert_eq!(
            flag_value(args, "--mcp-config"),
            Some(mcp_path.display().to_string().as_str())
        );
        assert!(args.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(
            flag_value(args, "--settings"),
            Some(settings_path.display().to_string().as_str())
        );
        assert_eq!(
            flag_value(args, "--append-system-prompt"),
            Some("REMEMBER ME")
        );
        assert_eq!(
            flag_value(args, "--plugin-dir"),
            Some(plugin_dir.display().to_string().as_str())
        );
        // The home's own user settings — the templates — must keep applying.
        assert!(!args.contains(&"--setting-sources".to_string()));
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &home.path().display().to_string())
        );
    }

    #[test]
    fn provision_home_with_nothing_optional_passes_only_the_mcp_config() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Home {
            home: home.path().to_path_buf(),
            scratch: scratch.path().to_path_buf(),
        };

        let launch = Claude::new()
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();

        assert!(launch.args.contains(&"--mcp-config".to_string()));
        for absent in ["--settings", "--append-system-prompt", "--plugin-dir"] {
            assert!(!launch.args.contains(&absent.to_string()), "{absent}");
        }
        assert!(!scratch.path().join("plugin").exists());
    }

    #[test]
    fn both_variants_share_a_home() {
        assert!(Claude::new().shares_home());
        assert!(Claude::new_acp().shares_home());
    }

    /// `claude-code-acp` under a home: the adapter and `CLAUDE_CONFIG_DIR`, no flag, nothing in
    /// scratch, and the profile-owned pieces written into the home around what was there.
    #[test]
    fn claude_code_acp_writes_profile_owned_items_into_the_home_and_keeps_the_rest() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");
        let mut spec = home_spec(home.path(), scratch.path(), &skill_path);
        spec.io = crate::spec::IoModes::Structured;
        std::fs::write(
            home.path().join("settings.json"),
            r#"{"theme":"light","permissions":{"allow":["Old"]}}"#,
        )
        .unwrap();
        let user_md = "# Mine\n\nKeep this.\n";
        std::fs::write(home.path().join("CLAUDE.md"), user_md).unwrap();

        let acp = Claude::new_acp();
        let launch = acp
            .provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();
        assert_eq!(launch.program, ACP_COMMAND);
        assert!(launch.args.is_empty(), "{:?}", launch.args);
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &home.path().display().to_string())
        );
        assert!(std::fs::read_dir(scratch.path()).unwrap().next().is_none());
        assert!(!home.path().join("mcp.json").exists());

        assert!(home.path().join("skills/my-skill/SKILL.md").is_file());
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(home.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["theme"], "light");
        assert_eq!(settings["permissions"]["defaultMode"], "plan");

        // A second run replaces the managed block rather than stacking another.
        spec.initial.as_mut().unwrap().instructions = Some("NEW TEXT".to_string());
        acp.provision_home(&spec, Some(home.path()), scratch.path())
            .unwrap();
        let md = std::fs::read_to_string(home.path().join("CLAUDE.md")).unwrap();
        assert!(md.starts_with(user_md), "{md}");
        assert_eq!(md.matches(MANAGED_BEGIN).count(), 1, "{md}");
        assert!(
            md.contains("NEW TEXT") && !md.contains("REMEMBER ME"),
            "{md}"
        );
    }

    /// With no profile, `claude-code-acp` names no config dir and writes nothing anywhere.
    #[test]
    fn claude_code_acp_native_writes_nothing() {
        let scratch = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");
        let mut spec = home_spec(scratch.path(), scratch.path(), &skill_path);
        spec.io = crate::spec::IoModes::Structured;

        let launch = Claude::new_acp()
            .provision_home(&spec, None, scratch.path())
            .unwrap();
        assert_eq!(launch.program, ACP_COMMAND);
        assert!(launch.env.iter().all(|(k, _)| k != "CLAUDE_CONFIG_DIR"));
        assert!(std::fs::read_dir(scratch.path()).unwrap().next().is_none());
    }

    #[test]
    fn claude_code_acp_logs_in_as_claude_code_does() {
        let home = tempfile::TempDir::new().unwrap();
        let (acp, native) = (
            Claude::new_acp().login_home(home.path()).unwrap(),
            Claude::new().login_home(home.path()).unwrap(),
        );
        assert_eq!(
            (acp.program, acp.args, acp.env),
            (native.program, native.args, native.env)
        );
    }

    #[test]
    fn login_home_logs_in_through_the_config_dir_and_leaves_home_alone() {
        let home = tempfile::TempDir::new().unwrap();
        let launch = Claude::new().login_home(home.path()).unwrap();

        assert_eq!(launch.args, vec!["auth", "login"]);
        assert_eq!(
            launch.env,
            vec![(
                "CLAUDE_CONFIG_DIR".to_string(),
                home.path().display().to_string()
            )]
        );
    }

    /// The shared `.claude.json` holds the login's identity and every other run's projects:
    /// `post_seed` adds only what is missing and leaves the rest byte for byte.
    #[test]
    fn post_seed_under_a_home_adds_only_missing_keys_and_keeps_the_rest() {
        let home = tempfile::TempDir::new().unwrap();
        let scratch = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("/tmp/new"));
        spec.config = ConfigStrategy::Home {
            home: home.path().to_path_buf(),
            scratch: scratch.path().to_path_buf(),
        };
        let path = home.path().join(".claude.json");
        std::fs::write(
            &path,
            r#"{"oauthAccount":{"emailAddress":"a@b"},"hasCompletedOnboarding":false,"projects":{"/tmp/old":{"hasTrustDialogAccepted":false,"x":1}}}"#,
        )
        .unwrap();

        Claude::new().post_seed(&spec, home.path()).unwrap();

        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc["oauthAccount"]["emailAddress"].as_str(), Some("a@b"));
        // Present, so not forced — a value the harness wrote is its own.
        assert_eq!(doc["hasCompletedOnboarding"].as_bool(), Some(false));
        assert_eq!(
            doc["projects"]["/tmp/old"]["hasTrustDialogAccepted"].as_bool(),
            Some(false)
        );
        assert_eq!(doc["projects"]["/tmp/old"]["x"].as_i64(), Some(1));
        assert_eq!(
            doc["projects"]["/tmp/new"]["hasTrustDialogAccepted"].as_bool(),
            Some(true)
        );

        // A document holding both already is not rewritten at all.
        let before = std::fs::read(&path).unwrap();
        Claude::new().post_seed(&spec, home.path()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // Nothing but the file and its lock.
        let mut names: Vec<_> = std::fs::read_dir(home.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, vec![".claude.json", ".claude.json.am-lock"]);
    }

    #[test]
    fn parse_model_slash_output_extracts_aliases_and_default() {
        let text = "\
Current model: Opus 4.8 (effort: high)
Usage: /model <name>. Available: sonnet, opus, haiku, fable, best, sonnet[1m], opus[1m], fable[1m], opusplan, default, or a full model ID.";
        let models = parse_model_slash_output(text);
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "sonnet",
                "opus",
                "haiku",
                "fable",
                "best",
                "sonnet[1m]",
                "opus[1m]",
                "fable[1m]",
                "opusplan",
                "default",
            ]
        );
        let default = models.iter().find(|m| m.id == "default").unwrap();
        assert!(default.default);
        assert_eq!(
            default.description.as_deref(),
            Some("Current model: Opus 4.8 (effort: high)")
        );
        assert!(
            models
                .iter()
                .filter(|m| m.id != "default")
                .all(|m| !m.default)
        );
    }

    #[test]
    fn parse_model_slash_output_empty_without_available() {
        assert!(parse_model_slash_output("no models here").is_empty());
    }

    #[test]
    fn extract_slash_result_text_prefers_result_event() {
        let stdout = r#"
{"type":"system","subtype":"init","model":"claude-opus-4-8"}
{"type":"assistant","message":{"model":"<synthetic>","content":[{"type":"text","text":"assistant only"}]}}
{"type":"result","subtype":"success","result":"Current model: X\nUsage: /model <name>. Available: sonnet, default, or a full model ID.","is_error":false}
"#;
        let text = extract_slash_result_text(stdout).unwrap();
        assert!(text.contains("Available: sonnet"));
        assert!(!text.contains("assistant only"));
    }

    #[test]
    fn extract_slash_result_text_falls_back_to_assistant() {
        let stdout = r#"
{"type":"assistant","message":{"content":[{"type":"text","text":"Usage: /model <name>. Available: opus, sonnet."}]}}
"#;
        let text = extract_slash_result_text(stdout).unwrap();
        assert!(text.contains("Available: opus"));
    }

    /// Live check against a real `claude` on PATH. Skipped when the binary
    /// is missing so unit CI without Claude Code still passes.
    #[test]
    fn discover_models_live_jsonl_when_claude_available() {
        let has_claude = Command::new("claude")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !has_claude {
            eprintln!("skipping: `claude` not on PATH");
            return;
        }
        let models = Claude::new()
            .discover_models()
            .expect("live stream-json /model discovery");
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(
            ids.contains(&"opus") && ids.contains(&"sonnet") && ids.contains(&"haiku"),
            "expected core aliases in live list: {ids:?}"
        );
    }

    #[test]
    fn parse_effort_levels_from_wrapped_help() {
        let help = "  --effort <level>                      Effort level for the current session\n                                        (low, medium, high, xhigh, max)\n";
        assert_eq!(
            parse_effort_levels(help),
            vec!["low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn parse_effort_levels_absent_is_empty() {
        let help = "  --model <model>                       Model for the current session\n";
        assert!(parse_effort_levels(help).is_empty());
    }

    /// Live check against a real `claude` on PATH. Skipped when the binary
    /// is missing so unit CI without Claude Code still passes.
    #[test]
    fn version_live_when_claude_available() {
        let has_claude = Command::new("claude")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !has_claude {
            eprintln!("skipping: `claude` not on PATH");
            return;
        }
        let version = Claude::new().version().expect("live `claude --version`");
        assert!(!version.is_empty());
        assert!(!version.contains('\n'));
    }
}
