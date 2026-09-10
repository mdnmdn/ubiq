//! Grok CLI provisioner.
//!
//! Transcribes `_docs/harness/grok.md` (esp. "On-disk layout", "MCP servers",
//! "Skills", "Authentication", "Orchestration / headless invocation") into a
//! [`Harness`] impl.
//!
//! The installed binary is xAI's official `grok` CLI (Rust), version 1.0.13 as of the
//! 2026-09-10 capture — **not** the npm `@vibe-kit/grok-cli` this module originally assumed.
//! See `_docs/wip/grok-acp-capture.md` for what was verified live and `_docs/harness/grok.md`
//! for the corrected reference. The identity swap does not change the isolation story below:
//! both programs anchor on `~/.grok/` and `~/.agents/skills/`.
//!
//! The isolation lever (Class C — no config lever): Grok CLI has **no
//! `GROK_CONFIG_DIR`-style override** — its global config dir (`~/.grok/`)
//! and user-tier skills (`~/.agents/skills/`) are derived from the OS home.
//! So provisioning always relocates `HOME` to the ephemeral `dir`:
//! `~/.grok/user-settings.json` becomes `<dir>/.grok/user-settings.json`
//! and `~/.agents/skills/` becomes `<dir>/.agents/skills/`. Injected MCP
//! servers and skills land there, the user's real `~/.grok` is never read or
//! written, and ambient user-tier MCP servers are suppressed (the relocated
//! HOME has none). The throwaway `dir` is grok's home, so its config AND its
//! sessions/logs land there rather than in any persistent home. The agent
//! still runs against the user's real project (`spec.cwd`); only its config
//! home moves.
//!
//! Because relocating `HOME` strips the user's real toolchain
//! (`nvm`/`mise`/`pyenv`, shell rc, PATH shims), this Class-C harness sets
//! `ConfigAnchor::requires_home_relocation = true` (the isol8-pairing
//! signal). When the run's account carries a private `home` holding a
//! captured login, provisioning does NOT point `HOME` at that home; instead
//! it **seeds** the captured `~/.grok/auth.json` into `<dir>/.grok/auth.json`
//! (via [`super::seed_login`] driven by [`Grok::config_anchor`]), so grok
//! finds its credentials under the relocated HOME at launch.
//!
//! **Known non-invasiveness gap:** relocating `HOME` has been observed to
//! isolate config/skill reads (`user-settings.json`, `.agents/skills/`) but
//! NOT session/log writes — a real launch was seen writing to the user's
//! real `~/.grok/sessions/…` and `~/.grok/logs/…` even with `HOME` set to the
//! ephemeral dir. No verified env-var lever closes this. That observation
//! predates the 2026-09-10 capture (which confirms the installed binary is
//! Rust, not Node — see above), so the original Node `os.homedir()` /
//! `os.userInfo().homedir` explanation for *why* no env var closes it no
//! longer holds; the gap itself has not been re-verified against the real
//! binary. See `_docs/harness/grok.md` § "Format quirks / gotchas" for
//! detail. Do not add speculative `XDG_*`/config-dir env vars to
//! `provision`'s `Launch.env` to try to fix this without first verifying,
//! against the actual installed binary, that they (a) redirect session/log
//! writes AND (b) still let Grok read the injected config this provisioner
//! writes under `<dir>/.grok/` — an unverified lever risks silently breaking
//! MCP/skill injection instead.
//!
//! Grok has no non-invasive always-on memory slot (its `AGENTS.md` is
//! merged from the git root down to cwd — the user's real project, which a
//! run must not write to), so `--instructions` is folded into the seeded
//! `--prompt` rather than written to a memory file.
//!
//! Structured I/O goes through ACP, not through Grok's own event stream:
//! `grok agent stdio` is an Agent Client Protocol endpoint, so the structured
//! seam is the generic [`crate::io::AcpBridge`] and this harness contributes
//! no wire code of its own. The NDJSON stream behind `grok --format json`
//! stays deliberately unused — its per-field event shapes are not documented
//! enough to build a faithful bridge (see `grok.md` §"Output stream
//! protocol"), which is precisely why the structured path went to ACP
//! instead.
//!
//! Verified 2026-09-10 against the installed `grok` 1.0.13 binary (see
//! `_docs/wip/grok-acp-capture.md`): `grok agent stdio` is a real ACP endpoint,
//! and the agent-mode flags (`-m/--model`, `--reasoning-effort`,
//! `--always-approve`) go *between* `agent` and `stdio`, not after it — see
//! `provision`'s `IoModes::Structured` arm.

use std::path::Path;

use anyhow::{Context, bail};
use serde_json::{Value, json};

use crate::Result;
use crate::config::{McpServer, McpTransport};
use crate::spec::{IoModes, McpRef, RunSpec};

use super::{ConfigAnchor, Harness, Launch, SeedFile};

/// The Grok CLI harness provisioner.
#[derive(Debug, Clone, Default)]
pub struct Grok;

impl Grok {
    /// Construct the Grok CLI harness descriptor.
    pub fn new() -> Self {
        Grok
    }
}

impl Harness for Grok {
    // Grok's structured seam is `grok agent stdio` — an ACP endpoint, driven
    // by the generic `crate::io::AcpBridge`, so nothing harness-specific is
    // parsed here. The `--format json` NDJSON stream is deliberately NOT
    // used: its per-field event shapes aren't documented enough to build a
    // faithful bridge (see `_docs/harness/grok.md`), which is why this went
    // to ACP instead.
    super::shared::harness_identity! {
        id: "grok",
        display_name: "Grok CLI",
        command: "grok",
        aliases: [],
        passthrough: true,
        structured: true,
        multi_turn: true,
        acp: true,
    }

    /// Class C: Grok has **no config-dir lever** — its only relocation seam is
    /// `HOME`, from which both `~/.grok/` and `~/.agents/skills/` derive. So
    /// `levers` is empty and `requires_home_relocation` is true (relocating
    /// HOME strips the user's toolchain — the isol8-pairing signal). A captured
    /// login is the single plaintext `~/.grok/auth.json`, seeded into the
    /// relocated HOME's `.grok/auth.json`. See `_docs/profiles.md` §5.
    fn config_anchor(&self) -> ConfigAnchor {
        ConfigAnchor {
            levers: vec![],
            login_seed: vec![SeedFile::credential(".grok/auth.json", ".grok/auth.json")],
            requires_home_relocation: true,
        }
    }

    /// Corrected per the 2026-09-10 capture (`_docs/wip/grok-acp-capture.md`): the installed
    /// binary is xAI's official `grok` 1.0.13, not the npm `@vibe-kit/grok-cli` this module
    /// previously assumed, and it has an explicit `grok models` command (there is no
    /// `~/.grok/models_cache.json` to read instead). Shell out to it and parse the ids off
    /// stdout, the same shape `Copilot::discover_models` uses for `copilot help config`.
    ///
    /// The output layout was captured from 1.0.13 on 2026-09-10 and is written out in
    /// [`parse_model_ids`], which reads the bulleted `Available models:` list and falls back to
    /// a token scan if that section is not there to read.
    fn discover_models(&self) -> Result<Vec<super::ModelInfo>> {
        let output = std::process::Command::new(self.command())
            .arg("models")
            .output()
            .with_context(|| "running `grok models` (is the grok binary on PATH?)")?;
        if !output.status.success() {
            anyhow::bail!(
                "`grok models` failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let ids = parse_model_ids(&stdout);
        if ids.is_empty() {
            anyhow::bail!(
                "no model ids found in `grok models` output — its format may have changed; \
                 re-verify against the installed binary"
            );
        }
        let mut out: Vec<super::ModelInfo> = ids.into_iter().map(super::ModelInfo::new).collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(out)
    }

    /// The six values Grok's own top-level `--permission-mode <mode>` accepts (verified against
    /// the bundled user guide and `grok --help`, 1.0.13 — see
    /// `_docs/wip/grok-acp-capture.md` §4). Fixed CLI enum, not probed — see
    /// [`super::Harness::modes`]. Only meaningful on the passthrough path: `grok agent stdio`
    /// (the structured/ACP seam) has no permission-mode selector at all, just
    /// `--always-approve`/`_meta.yoloMode` (see `provision`'s `IoModes::Structured` arm).
    fn modes(&self) -> Vec<super::ModeInfo> {
        [
            ("default", "Default"),
            ("acceptEdits", "Accept edits"),
            ("auto", "Auto"),
            ("dontAsk", "Don't ask"),
            ("bypassPermissions", "Bypass permissions"),
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

    /// `bypassPermissions` is Grok's own name for asking nothing — see
    /// [`super::Harness::unattended_mode`].
    fn unattended_mode(&self) -> Option<&'static str> {
        Some("bypassPermissions")
    }

    fn provision(&self, spec: &RunSpec, dir: &Path) -> Result<Launch> {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        // Grok has no way to split a credential-store HOME from an injected-
        // config dir (unlike Claude Code's `CLAUDE_CONFIG_DIR`/`HOME` split or
        // opencode's `OPENCODE_CONFIG_DIR`/HOME-relative-auth split) — its
        // only lever is relocating `HOME` wholesale, and both `.grok/` and
        // `.agents/skills/` resolve from it. So the ephemeral `dir` is the
        // write target for injected config AND the `HOME` the child sees; a
        // captured account login is *seeded* into it below (rather than
        // pointing HOME at the account home) so the throwaway dir stays grok's
        // home and the account's persistent home is never used as a write
        // target.

        // 1. Skills: copy each skill folder into
        // <dir>/.agents/skills/<id>/. With HOME relocated to `dir`, this is
        // the user-tier `~/.agents/skills/` Grok discovers (the agent-neutral
        // path, not `.grok/`).
        let skills_dir = dir.join(".agents").join("skills");
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

        // 2. MCP: write <dir>/.grok/user-settings.json with `mcpServers`
        // when there are servers to inject. There is no `--mcp-config` flag,
        // so the user-settings file (under the relocated HOME) is the only
        // injection channel. Written only when non-empty so unused runs stay
        // minimal; ambient user-tier servers are already suppressed by the
        // relocated HOME, not by writing an empty file.
        let mcp_map = build_mcp_servers(&spec.mcps)?;
        if !mcp_map.is_empty() {
            let grok_dir = dir.join(".grok");
            std::fs::create_dir_all(&grok_dir)
                .with_context(|| format!("creating {}", grok_dir.display()))?;
            let settings = json!({ "mcpServers": Value::Object(mcp_map) });
            let settings_path = grok_dir.join("user-settings.json");
            std::fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
                .with_context(|| format!("writing {}", settings_path.display()))?;
        }

        // Hooks: Grok exposes no documented native hook slot that a run can
        // populate non-invasively, so `spec.hooks` is a no-op here — a
        // fidelity gap, not a user error (same stance as opencode).

        // 3. Build the launch. Grok's `--prompt <text>` is its non-interactive
        // seam and is used to seed a run. Grok has no non-invasive always-on
        // memory file (its `AGENTS.md` lives in the user's real project, which
        // a run must not write to), so `spec.initial.instructions` is folded
        // into the prompt text rather than written to disk.
        let args = match spec.io {
            IoModes::Structured => {
                // Structured mode: `grok agent [options] stdio [args...]` — an
                // ACP endpoint, driven by `crate::io::AcpBridge`. Everything a
                // passthrough run puts in argv moves onto the wire here: the
                // prompt is a `session/prompt`, and a resume is a
                // `session/load` (from `Provisioned::resume`), so neither
                // `--prompt` nor `--session` belongs on this argv.
                //
                // Per the 2026-09-10 capture (`_docs/wip/grok-acp-capture.md`
                // §3), `grok agent stdio` DOES take agent-mode flags, but only
                // *between* `agent` and `stdio` — `grok agent --model
                // grok-4.6 --reasoning-effort high --always-approve stdio`.
                // Putting `-m`/`--reasoning-effort` after `stdio` (as this
                // used to assume they'd arrive via `session/set_config_option`
                // instead) does not work. There is no `--permission-mode` for
                // this path at all: Grok's only ACP-side permission lever is
                // `--always-approve`/`_meta.yoloMode` (see §4), neither of
                // which `spec.policy.permission_mode` maps onto today, so no
                // permission flag is emitted here.
                let mut structured_args = vec!["agent".to_string()];
                if let Some(model) = &spec.model {
                    structured_args.push("--model".to_string());
                    structured_args.push(model.clone());
                }
                if let Some(effort) = &spec.thinking {
                    structured_args.push("--reasoning-effort".to_string());
                    structured_args.push(effort.clone());
                }
                structured_args.push("stdio".to_string());
                structured_args.extend(spec.passthrough_args.clone());
                if seeded_prompt(spec).is_some() {
                    tracing::debug!(
                        "grok structured mode ignores the seeded prompt/instructions in argv; \
                         it arrives over the wire as session/prompt instead"
                    );
                }
                structured_args
            }
            IoModes::Passthrough => {
                let mut args = spec.passthrough_args.clone();
                // Model selection: `-m <id>` (Grok also honors `GROK_MODEL`).
                // Only added when a model is set, so runs without `--model`
                // keep byte-identical argv.
                if let Some(model) = &spec.model {
                    args.push("-m".to_string());
                    args.push(model.clone());
                }
                // Reasoning effort: `--reasoning-effort <level>` (top-level
                // flag, same spelling as the agent-mode one — see
                // `_docs/wip/grok-acp-capture.md` §3-4). Only added when set.
                if let Some(effort) = &spec.thinking {
                    args.push("--reasoning-effort".to_string());
                    args.push(effort.clone());
                }
                // Permission mode: `--permission-mode <mode>`, one of the six
                // values `Grok::modes` lists. Only added when the spec names
                // one, so unpolicied runs keep byte-identical argv.
                if let Some(policy) = &spec.policy
                    && let Some(mode) = &policy.permission_mode
                {
                    args.push("--permission-mode".to_string());
                    args.push(mode.clone());
                }
                // Resume: `--session <id>` (Grok also accepts
                // `--session latest`). Only added when a resume id is set, so
                // resumeless runs keep byte-identical argv.
                if let Some(id) = &spec.resume {
                    args.push("--session".to_string());
                    args.push(id.clone());
                }
                if let Some(prompt_text) = seeded_prompt(spec) {
                    args.push("--prompt".to_string());
                    args.push(prompt_text);
                }
                args
            }
        };

        // 4. Account: inject credential *references* into the child's env.
        // Grok authenticates with a single xAI API key via `GROK_API_KEY`
        // (and an optional `GROK_BASE_URL` endpoint override) — no OAuth, no
        // multi-provider map. `am`'s account store holds only env-var NAMES
        // and a base URL; the secret value is read transiently below and
        // passed to the child in-memory, never written to disk.
        //
        // HOME is relocated to the ephemeral `dir` so Grok's `~/.grok` and
        // `~/.agents/skills` resolve inside it (the isolation lever; see the
        // module docs). This is the Class-C HOME relocation that strips the
        // user's real toolchain — hence `config_anchor().requires_home_relocation`.
        let mut env = vec![("HOME".to_string(), dir.display().to_string())];
        if let Some(account) = &spec.account {
            if let Some(base_url) = &account.base_url {
                env.push(("GROK_BASE_URL".to_string(), base_url.clone()));
            }
            if let Some(name) = account
                .api_key_env
                .as_ref()
                .or(account.auth_token_env.as_ref())
            {
                let value = super::shared::account_env(account, name)?;
                env.push(("GROK_API_KEY".to_string(), value));
            }
            // Reuse a prior `am account login` by *seeding* the captured
            // `~/.grok/auth.json` into `<dir>/.grok/auth.json` (the relocated
            // HOME), so grok finds its credentials at launch. No-op when the
            // account home holds no captured login yet. The seed list is
            // declared once in `config_anchor()`.
            if let Some(login) = spec
                .account_login
                .clone()
                .or_else(|| account.home.clone().map(crate::source::Source::Dir))
            {
                super::seed_login(dir, &login, &self.config_anchor().login_seed)?;
            }
        }

        Ok(Launch {
            program: "grok".to_string(),
            args,
            env,
            env_remove: Vec::new(),
            env_clear: false,
        })
    }

    /// Log Grok CLI into `home`, capturing the resulting OAuth `auth.json`.
    ///
    /// Per grok.md "Credential capture & reuse": `~/.grok/auth.json` is the
    /// sole OAuth credential file and is **always plaintext** (no keychain,
    /// so no force-file-storage knob is needed here, unlike Claude Code/
    /// Codex). `HOME` is the only relocation lever Grok exposes (no
    /// `GROK_CONFIG_DIR`-style override), so login relocates HOME to the
    /// capture `home` to write `<home>/.grok/auth.json`. `provision()`'s reuse
    /// path then *seeds* that file into the ephemeral dir's `.grok/auth.json`
    /// (via [`super::seed_login`] driven by [`Grok::config_anchor`]) rather
    /// than pointing the child's HOME at the account home.
    ///
    /// There is no documented `grok auth login` verb: the interactive TUI
    /// triggers the OAuth flow on first run under a fresh `HOME`, so the
    /// launch is bare (no subcommand args). Not verified against the
    /// installed binary in this environment (grok is not on `PATH` here) —
    /// this matches grok.md's documented behavior and should be re-verified
    /// against `grok --help` when the binary is available.
    fn login(&self, home: &Path) -> Result<super::LoginPlan> {
        let env = vec![("HOME".to_string(), home.display().to_string())];
        Ok(super::LoginPlan {
            launch: Launch {
                program: "grok".to_string(),
                args: Vec::new(),
                env,
                env_remove: Vec::new(),
                env_clear: false,
            },
            credential_files: vec![std::path::PathBuf::from(".grok/auth.json")],
        })
    }

    fn structured_bridge(
        &self,
        provisioned: &crate::provision::Provisioned,
        cwd: &Path,
    ) -> Result<Box<dyn crate::io::IoBridge>> {
        let child = crate::io::spawn_piped(&provisioned.launch, cwd)?;
        Ok(Box::new(crate::io::AcpBridge::new(
            child,
            cwd,
            provisioned.resume.as_deref(),
            provisioned.model.as_deref(),
        )?))
    }
}

/// Combine always-on instructions and the initial prompt into one seed string
/// for `--prompt`. Returns `None` when neither is present (a pure interactive
/// passthrough run). Instructions are prepended so they read as standing
/// guidance ahead of the concrete request.
fn seeded_prompt(spec: &RunSpec) -> Option<String> {
    let initial = spec.initial.as_ref()?;
    match (initial.instructions.as_deref(), initial.prompt.as_deref()) {
        (Some(instr), Some(prompt)) => Some(format!("{instr}\n\n{prompt}")),
        (Some(instr), None) => Some(instr.to_string()),
        (None, Some(prompt)) => Some(prompt.to_string()),
        (None, None) => None,
    }
}

/// Render one [`McpServer`] into the JSON shape Grok's `user-settings.json`
/// `mcpServers` map expects, keyed by transport. Grok uses an explicit `type`
/// discriminator for every transport (`stdio` / `http` / `sse`), unlike Claude
/// Code which omits it for stdio.
fn mcp_server_json(server: &McpServer) -> Value {
    match server.transport {
        McpTransport::Stdio => {
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), json!("stdio"));
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

/// Build the `mcpServers` map from `spec.mcps`, keyed by server id.
fn build_mcp_servers(mcps: &[McpRef]) -> Result<serde_json::Map<String, Value>> {
    let mut servers = serde_json::Map::new();
    for mcp in mcps {
        match mcp {
            McpRef::Catalog(server) | McpRef::Inline(server) => {
                servers.insert(server.id.clone(), mcp_server_json(server));
            }
            McpRef::InProcess(_) => {
                bail!("in-process MCP not supported in CLI/passthrough mode");
            }
        }
    }
    Ok(servers)
}

/// The model ids in `grok models` stdout, deduped in first-seen order.
///
/// The format, captured from 1.0.13 on 2026-09-10:
///
/// ```text
/// Default model: grok-4.6
///
/// Available models:
///   * grok-4.6 (default)
///   - grok-4.5
/// ```
///
/// The bulleted list under `Available models:` is what is read: the first
/// token of every line whose first non-space character is `*` or `-`. A
/// custom model (`11-custom-models.md`) carries whatever id its provider
/// entry names, so the id is taken as written rather than filtered on a
/// `grok-` prefix.
///
/// **Fallback**: if that section yields nothing — the wording changed, or the
/// binary printed only the default line — every token shaped like
/// `grok-<something>` anywhere in the output is taken instead, which is what
/// keeps a format change from emptying the picker outright.
fn parse_model_ids(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut listing = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("available models:") {
            listing = true;
            continue;
        }
        if !listing {
            continue;
        }
        // A blank line ends the list; so does anything that is not a bullet.
        let Some(rest) = trimmed.strip_prefix(['*', '-']) else {
            if !trimmed.is_empty() {
                listing = false;
            }
            continue;
        };
        if let Some(id) = rest.split_whitespace().next()
            && !ids.iter().any(|seen| seen == id)
        {
            ids.push(id.to_string());
        }
    }
    if !ids.is_empty() {
        return ids;
    }
    for line in text.lines() {
        for word in line.split_whitespace() {
            let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '.');
            if cleaned.len() > "grok-".len()
                && cleaned.starts_with("grok-")
                && !ids.iter().any(|id| id == cleaned)
            {
                ids.push(cleaned.to_string());
            }
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

    super::super::shared::harness_conformance_tests!(Grok, "grok");

    #[test]
    fn provision_writes_user_settings_skills_and_launch_without_touching_home() {
        // Stand-in for the user's real `$HOME`. `provision()` never reads or
        // writes an env-derived home directory (it only touches the `dir` it
        // is given), so this must stay empty — the core isolation invariant.
        let fake_home = tempfile::TempDir::new().unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let skills_src = tempfile::TempDir::new().unwrap();
        let skill_path = write_skill(skills_src.path(), "my-skill");

        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
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

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        // user-settings.json exists under .grok/ with the right mcpServers shape.
        let settings_path = config_dir.path().join(".grok/user-settings.json");
        assert!(settings_path.exists());
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        let servers = settings.get("mcpServers").unwrap().as_object().unwrap();
        assert_eq!(servers.len(), 2);
        // stdio: explicit type + command + args.
        assert_eq!(servers["postgres"]["type"].as_str(), Some("stdio"));
        assert_eq!(
            servers["postgres"]["command"].as_str(),
            Some("postgres-mcp")
        );
        assert_eq!(servers["postgres"]["args"].as_array().unwrap().len(), 1);
        // http remote: type + url.
        assert_eq!(servers["docs"]["type"].as_str(), Some("http"));
        assert_eq!(
            servers["docs"]["url"].as_str(),
            Some("https://example.com/mcp/")
        );

        // Skill copied under the agent-neutral .agents/skills path.
        let skill_md = config_dir.path().join(".agents/skills/my-skill/SKILL.md");
        assert!(skill_md.exists());

        // Launch relocates HOME to the ephemeral dir (the isolation lever).
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "HOME" && v == &config_dir.path().display().to_string())
        );
        assert_eq!(launch.program, "grok");

        // Invariant: nothing written under the stand-in home dir.
        let home_entries: Vec<_> = std::fs::read_dir(fake_home.path()).unwrap().collect();
        assert!(
            home_entries.is_empty(),
            "expected no writes under the fake home dir, found: {home_entries:?}"
        );
    }

    #[test]
    fn provision_empty_mcps_writes_no_grok_dir() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        let grok = Grok::new();
        grok.provision(&spec, config_dir.path()).unwrap();

        // Byte-identical-config invariant: no MCP servers => no .grok dir.
        assert!(!config_dir.path().join(".grok").exists());
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
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.mcps.push(McpRef::InProcess(InProcessMcpHandle {
            name: "in-proc".to_string(),
            service: Arc::new(NoopService),
        }));

        let grok = Grok::new();
        let err = grok.provision(&spec, config_dir.path()).unwrap_err();
        assert!(err.to_string().contains("in-process"));
    }

    #[test]
    fn provision_prompt_and_instructions_fold_into_prompt_flag() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.initial = Some(Instructions {
            instructions: Some("REMEMBER: be terse".to_string()),
            prompt: Some("summarize the repo".to_string()),
        });

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        let idx = launch
            .args
            .iter()
            .position(|a| a == "--prompt")
            .expect("--prompt present");
        let text = launch.args.get(idx + 1).unwrap();
        assert!(text.contains("REMEMBER: be terse"));
        assert!(text.contains("summarize the repo"));
    }

    #[test]
    fn provision_no_initial_omits_prompt_flag() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();
        assert!(!launch.args.contains(&"--prompt".to_string()));
    }

    #[test]
    fn provision_resume_appends_session_flag() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.resume = Some("latest".to_string());

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        let idx = launch
            .args
            .iter()
            .position(|a| a == "--session")
            .expect("--session present");
        assert_eq!(launch.args.get(idx + 1), Some(&"latest".to_string()));
    }

    #[test]
    fn provision_account_api_key_and_base_url_map_to_grok_env_without_touching_disk() {
        use crate::account::Account;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "xai".to_string(),
            api_key_env: Some("PATH".to_string()),
            base_url: Some("https://gw.example/v1".to_string()),
            ..Default::default()
        });

        let expected = std::env::var("PATH").expect("PATH should be set in the test environment");

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "GROK_API_KEY" && v == &expected)
        );
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "GROK_BASE_URL" && v == "https://gw.example/v1")
        );

        // No-secret-on-disk invariant.
        super::super::shared::assert_no_secret_on_disk(config_dir.path(), &expected);
    }

    #[test]
    fn provision_account_home_seeds_creds_and_keeps_ephemeral_dir_as_home() {
        use crate::account::Account;

        // A persistent per-account "home" holding a captured login, laid out
        // exactly as `login()` writes it: `<home>/.grok/auth.json`.
        let account_home = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(account_home.path().join(".grok")).unwrap();
        std::fs::write(
            account_home.path().join(".grok").join("auth.json"),
            r#"{"access_token":"tok"}"#,
        )
        .unwrap();

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "private-home".to_string(),
            home: Some(account_home.path().to_path_buf()),
            ..Default::default()
        });
        spec.mcps.push(McpRef::Catalog(McpServer {
            id: "postgres".to_string(),
            transport: McpTransport::Stdio,
            command: Some("postgres-mcp".to_string()),
            args: vec![],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
        }));

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        // HOME relocates to the ephemeral dir, NOT the account's private home —
        // the throwaway dir stays grok's home (config + sessions land there).
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "HOME" && v == &config_dir.path().display().to_string())
        );
        assert!(
            !launch
                .env
                .iter()
                .any(|(k, v)| k == "HOME" && v == &account_home.path().display().to_string())
        );

        // The captured login is SEEDED into the ephemeral dir's relocated HOME
        // so grok finds `~/.grok/auth.json` (= <dir>/.grok/auth.json) at launch.
        let seeded = config_dir.path().join(".grok/auth.json");
        assert!(
            seeded.exists(),
            "auth.json should be seeded into the ephemeral dir"
        );
        assert!(
            std::fs::read_to_string(&seeded)
                .unwrap()
                .contains("access_token")
        );

        // Injected config (.grok/user-settings.json) also lands in the
        // ephemeral dir, alongside the seeded creds.
        assert!(config_dir.path().join(".grok/user-settings.json").exists());
    }

    #[test]
    fn provision_account_home_without_captured_creds_still_launches() {
        use crate::account::Account;

        // A `home` that exists but has no captured login yet: seeding is a
        // no-op, provisioning still succeeds and HOME is still the ephemeral dir.
        let account_home = tempfile::TempDir::new().unwrap();
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.account = Some(Account {
            id: "empty-home".to_string(),
            home: Some(account_home.path().to_path_buf()),
            ..Default::default()
        });

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        // Seeding is a no-op — no auth.json seeded.
        assert!(!config_dir.path().join(".grok/auth.json").exists());
        // HOME still relocates to the ephemeral dir.
        assert!(
            launch
                .env
                .iter()
                .any(|(k, v)| k == "HOME" && v == &config_dir.path().display().to_string())
        );
    }

    #[test]
    fn provision_mcp_as_skill_writes_skill_md_under_agents_skills() {
        use crate::spec::McpAsSkill;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
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

        let grok = Grok::new();
        grok.provision(&spec, config_dir.path()).unwrap();

        let skill_md_path = config_dir.path().join(".agents/skills/postgres/SKILL.md");
        assert!(skill_md_path.exists());
        let content = std::fs::read_to_string(&skill_md_path).unwrap();
        assert!(content.contains("name: postgres"));
        assert!(content.contains("description: Query a DB."));

        // Invariant: the MCP stays injected as normal in user-settings.json.
        let settings: Value = serde_json::from_str(
            &std::fs::read_to_string(config_dir.path().join(".grok/user-settings.json")).unwrap(),
        )
        .unwrap();
        assert!(settings["mcpServers"]["postgres"].is_object());
    }

    #[test]
    fn resolve_grok_by_id_and_command() {
        assert_eq!(super::super::resolve("grok").unwrap().id(), "grok");
    }

    #[test]
    fn login_points_home_at_capture_dir_and_names_auth_json() {
        let home = tempfile::TempDir::new().unwrap();

        let plan = Grok::new().login(home.path()).unwrap();

        assert!(
            plan.launch
                .env
                .iter()
                .any(|(k, v)| k == "HOME" && v == &home.path().display().to_string())
        );
        assert_eq!(
            plan.credential_files[0],
            std::path::PathBuf::from(".grok/auth.json")
        );
    }

    #[test]
    fn grok_supports_passthrough_and_an_acp_structured_bridge() {
        let grok = Grok::new();
        let support = grok.io_support();
        assert!(support.passthrough);
        assert!(support.structured);
        assert!(support.multi_turn);
        assert!(support.acp);
    }

    #[test]
    fn provision_structured_is_agent_stdio_without_prompt_or_session_but_carries_model_and_effort()
    {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = IoModes::Structured;
        spec.model = Some("grok-4.6".to_string());
        spec.thinking = Some("high".to_string());
        spec.resume = Some("sess-1".to_string());
        spec.initial = Some(Instructions {
            instructions: None,
            prompt: Some("summarize the repo".to_string()),
        });

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        // The model and reasoning effort go BETWEEN `agent` and `stdio`, per
        // the captured `grok agent --model … --reasoning-effort … stdio` argv.
        assert_eq!(
            launch.args,
            vec![
                "agent",
                "--model",
                "grok-4.6",
                "--reasoning-effort",
                "high",
                "stdio",
            ]
        );
        // The prompt and the resume still travel over the wire, not argv.
        assert!(!launch.args.contains(&"--prompt".to_string()));
        assert!(!launch.args.contains(&"--session".to_string()));
    }

    #[test]
    fn provision_structured_without_model_or_thinking_is_bare_agent_stdio() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.io = IoModes::Structured;

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        assert_eq!(launch.args, vec!["agent", "stdio"]);
    }

    #[test]
    fn provision_passthrough_adds_reasoning_effort_and_permission_mode_when_set() {
        use crate::spec::Policy;

        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());
        spec.model = Some("grok-4.6".to_string());
        spec.thinking = Some("xhigh".to_string());
        spec.policy = Some(Policy {
            permission_mode: Some("bypassPermissions".to_string()),
            ..Default::default()
        });

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        let idx = |flag: &str| launch.args.iter().position(|a| a == flag).unwrap();
        assert_eq!(
            launch.args.get(idx("-m") + 1),
            Some(&"grok-4.6".to_string())
        );
        assert_eq!(
            launch.args.get(idx("--reasoning-effort") + 1),
            Some(&"xhigh".to_string())
        );
        assert_eq!(
            launch.args.get(idx("--permission-mode") + 1),
            Some(&"bypassPermissions".to_string())
        );
    }

    #[test]
    fn provision_passthrough_without_thinking_or_policy_omits_the_flags() {
        let config_dir = tempfile::TempDir::new().unwrap();
        let mut spec = RunSpec::new("grok".to_string(), PathBuf::from("."));
        spec.config = ConfigStrategy::Fixed(config_dir.path().to_path_buf());

        let grok = Grok::new();
        let launch = grok.provision(&spec, config_dir.path()).unwrap();

        assert!(!launch.args.contains(&"--reasoning-effort".to_string()));
        assert!(!launch.args.contains(&"--permission-mode".to_string()));
    }

    #[test]
    fn modes_lists_the_six_permission_modes_with_bypass_as_unattended() {
        let grok = Grok::new();
        let ids: Vec<String> = grok.modes().into_iter().map(|m| m.id).collect();
        assert_eq!(
            ids,
            vec![
                "default",
                "acceptEdits",
                "auto",
                "dontAsk",
                "bypassPermissions",
                "plan",
            ]
        );
        assert_eq!(grok.unattended_mode(), Some("bypassPermissions"));
        assert!(ids.contains(&grok.unattended_mode().unwrap().to_string()));
    }

    #[test]
    fn parse_model_ids_reads_the_captured_listing() {
        // Verbatim `grok models` stdout, 1.0.13, captured 2026-09-10. The
        // `Default model:` line names an id the list repeats, so the dedupe
        // and the bullet-only rule are both exercised here.
        let text = "Default model: grok-4.6\n\nAvailable models:\n  * grok-4.6 (default)\n  \
                    - grok-4.5\n";
        assert_eq!(
            super::parse_model_ids(text),
            vec!["grok-4.6".to_string(), "grok-4.5".to_string()]
        );
    }

    #[test]
    fn parse_model_ids_keeps_an_id_that_is_not_grok_prefixed() {
        let text = "Available models:\n  * grok-4.6 (default)\n  - my-proxy/llama-70b\n";
        assert_eq!(
            super::parse_model_ids(text),
            vec!["grok-4.6".to_string(), "my-proxy/llama-70b".to_string()]
        );
    }

    #[test]
    fn parse_model_ids_falls_back_to_a_token_scan_without_a_listing() {
        let text = "Available models:\n  grok-4.6  (default)\n  grok-4.5\n";
        assert_eq!(
            super::parse_model_ids(text),
            vec!["grok-4.6".to_string(), "grok-4.5".to_string()]
        );
    }

    #[test]
    fn parse_model_ids_no_matches_returns_empty() {
        assert_eq!(super::parse_model_ids("nothing here"), Vec::<String>::new());
    }
}
