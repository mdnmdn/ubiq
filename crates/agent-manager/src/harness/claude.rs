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
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, bail};
use serde_json::{Value, json};

use crate::Result;
use crate::config::{McpServer, McpTransport};
use crate::spec::{HookRef, McpRef, RunSpec};

use super::{ConfigAnchor, Harness, Launch, ModelInfo, Relocate, SeedFile};

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
    super::shared::harness_identity! {
        id: "claude-code",
        display_name: "Claude Code",
        command: "claude",
        aliases: ["claude"],
        passthrough: true,
        structured: true,
    }

    /// Class A: `CLAUDE_CONFIG_DIR` relocates the entire config — credentials
    /// and `.claude.json` included (verified against Claude Code 2.1.206) — so
    /// a captured login is the two files below, seeded into the ephemeral dir
    /// while the real `HOME` stays intact. See `_docs/profiles.md` §5.
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
        let output = Command::new("claude")
            .arg("--help")
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

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        // 1. Skills: copy each skill folder into <dir>/skills/<id>/.
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
            // Highest precedence: a mode `resolve` wrote onto `spec.policy` (from
            // `--permission-mode` or a `--safe` preset) wins; unattended structured runs still
            // default to `bypassPermissions` when nothing chose a mode, so every existing run
            // stays byte-identical.
            let permission_mode = spec
                .policy
                .as_ref()
                .and_then(|p| p.permission_mode.as_deref())
                .unwrap_or("bypassPermissions");
            args.push("--permission-mode".to_string());
            args.push(permission_mode.to_string());
            args.push("--disallowedTools".to_string());
            args.push("AskUserQuestion".to_string());
        }
        args.extend(spec.passthrough_args.iter().cloned());

        // Append prompt as trailing positional argument, passthrough mode
        // only — structured mode's bridge sends it as NDJSON on stdin.
        if !structured && let Some(prompt) = spec.initial.as_ref().and_then(|i| i.prompt.as_ref()) {
            args.push(prompt.clone());
        }

        // 6. Account: inject credential *references* into the child's env.
        let mut env = vec![("CLAUDE_CONFIG_DIR".to_string(), dir.display().to_string())];
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
            if let Some(login) = spec
                .account_login
                .clone()
                .or_else(|| account.home.clone().map(crate::source::Source::Dir))
            {
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
                super::seed_login(dir, &login, &self.config_anchor().login_seed)?;
            }
        }

        Ok(Launch {
            program: "claude".to_string(),
            args,
            env,
            env_remove: ENV_HYGIENE.iter().map(|s| s.to_string()).collect(),
            env_clear: false,
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
                env_clear: false,
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

    /// Renew by re-reading the live Claude Code session from the OS Keychain.
    ///
    /// Claude Code has no headless token-refresh subcommand — the current
    /// OAuth session lives in the macOS Keychain (service
    /// [`crate::account::CLAUDE_KEYCHAIN_SERVICE`]), which `am` reads but never
    /// writes. "Renew" therefore re-reads that blob and returns it as the
    /// credential set (plus the `.claude.json` identity companion from the real
    /// `HOME`, when present), so a `SecretStore` gets whatever the user's live
    /// login currently holds. Errors off macOS / with no readable entry, same
    /// as `am account import`. The `creds` argument (the currently-stored set)
    /// is unused — the Keychain is the source of truth.
    fn renew_credentials(
        &self,
        _creds: &[crate::credentials::CredentialBlob],
    ) -> Result<Vec<crate::credentials::CredentialBlob>> {
        use crate::credentials::CredentialBlob;
        let creds = crate::account::read_claude_keychain_credentials()?;
        let mut blobs = vec![CredentialBlob {
            name: ".credentials.json".to_string(),
            rel_path: std::path::PathBuf::from(".claude/.credentials.json"),
            bytes: creds,
        }];
        if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
            let identity = home.join(".claude.json");
            if identity.is_file()
                && let Ok(bytes) = std::fs::read(&identity)
            {
                blobs.push(CredentialBlob {
                    name: ".claude.json".to_string(),
                    rel_path: std::path::PathBuf::from(".claude.json"),
                    bytes,
                });
            }
        }
        Ok(blobs)
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

    /// Structural fix-ups to the seeded `.claude.json`, always forced
    /// (never template-overridable, unlike [`Claude::templates`] — these
    /// aren't preferences, they're correctness requirements for `am`'s
    /// ephemeral-config model):
    ///
    /// 1. A login captured via `claude auth login` (the non-interactive path
    ///    `am account login` drives, under a HOME-relocated,
    ///    keychain-unreachable capture home — see [`Claude::login`]) never
    ///    runs the interactive onboarding wizard, so the file lacks
    ///    `hasCompletedOnboarding`. Seeded as-is, Claude Code is fully
    ///    authenticated but still opens its onboarding UI on launch.
    /// 2. Claude Code gates a per-project trust dialog on
    ///    `projects[cwd].hasTrustDialogAccepted`, keyed by the exact cwd
    ///    string. A fresh/ephemeral `CLAUDE_CONFIG_DIR` has no record of
    ///    `spec.cwd`, so every run would otherwise hit that dialog too.
    fn post_seed(&self, spec: &RunSpec, dir: &Path) -> Result<()> {
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
    fn provision_account_seeds_login_into_config_dir_without_touching_home() {
        use crate::account::Account;

        // A persistent per-account "home" holding a captured login, laid out
        // exactly as `login()` writes it: `<home>/.claude/.credentials.json`
        // and `<home>/.claude.json`.
        let account_home = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(account_home.path().join(".claude")).unwrap();
        std::fs::write(
            account_home
                .path()
                .join(".claude")
                .join(".credentials.json"),
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
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "ANTHROPIC_BASE_URL" && v == "https://gw/")
        );
        let settings_path = config_dir.path().join("settings.json");
        assert!(settings_path.exists());
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(
            settings.get("apiKeyHelper").unwrap().as_str(),
            Some("get-key")
        );

        // The captured login is seeded INTO the ephemeral config dir...
        let seeded_creds = config_dir.path().join(".credentials.json");
        let seeded_json = config_dir.path().join(".claude.json");
        assert!(
            seeded_creds.exists(),
            "credentials should be seeded into CLAUDE_CONFIG_DIR"
        );
        assert!(
            seeded_json.exists(),
            ".claude.json should be seeded into CLAUDE_CONFIG_DIR"
        );
        assert!(
            std::fs::read_to_string(&seeded_creds)
                .unwrap()
                .contains("claudeAiOauth")
        );

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

    #[test]
    fn provision_structured_io_defaults_to_bypass_permissions_with_no_policy() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("claude-code".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = crate::spec::IoModes::Structured;

        let launch = Claude::new().provision(&spec, config_dir.path()).unwrap();
        let pair = launch
            .args
            .windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "bypassPermissions");
        assert!(
            pair,
            "expected default bypassPermissions in argv: {:?}",
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

    #[test]
    fn login_points_home_at_capture_dir_and_names_credentials_file() {
        let home = tempfile::TempDir::new().unwrap();

        let plan = Claude::new().login(home.path()).unwrap();

        assert!(
            plan.launch
                .env
                .iter()
                .any(|(k, v)| k == "HOME" && v == &home.path().display().to_string())
        );
        assert!(!plan.credential_files.is_empty());
        assert!(
            plan.credential_files[0]
                .to_str()
                .unwrap()
                .ends_with(".credentials.json")
        );
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
