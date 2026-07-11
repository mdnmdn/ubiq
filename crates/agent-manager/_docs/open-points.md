# Open points

Topics known to be unresolved or deferred, gathered from the test runs
(`_docs/test-runs/`), the harness docs, and the credential-login / config work.
Each item says **what**, **why it's open**, and **what to check / do next**.
Ordered roughly by how load-bearing it is. Resolved items are recorded in the
relevant test-run's "Resolution" addendum, not here.

---

## 1. MCP-as-skill — deep dive required ⚠️

**This is the item most in need of a deep investigation before it can be
trusted.** Design + status doc: [`target/mcp-as-skill.md`](./target/mcp-as-skill.md).

**What exists today (the honest state).** Only the *schema + SKILL.md-pointer
stepping stone* is built:
- Catalog `expose = "tools" | "skill"` + `summary`, the `--mcp-as-skill a,b`
  flag, and `RunSpec.mcp_as_skill` all work.
- Each provisioner writes one generated `SKILL.md` pointer per entry into its
  skills dir (`<config>/skills/<id>/SKILL.md`, or `<CODEX_HOME>/.agents/skills/…`
  for Codex).
- **Crucially: this saves NO context.** The named MCP is *still* injected as a
  normal, always-on tool set in `mcp.json`/`config.toml` *alongside* the pointer.
  The `SKILL.md` body says so explicitly. So today the feature **adds** a skill
  file on top of the full MCP tool cost — a net context *increase*, not the
  decrease the feature promises.

**Why it's open.** The "expand on demand" mechanism — the whole point — is
unbuilt, and the stepping stone is easy to mistake for the finished feature.

**What to check / decide (the deep dive):**
1. **Auto-invocation actually fires.** Verify each harness truly discovers and
   auto-invokes a description-only skill placed by `am`:
   - Claude Code: is `skills/<id>/SKILL.md` under `CLAUDE_CONFIG_DIR` auto-loaded
     by description? (vs needing a `/skill` call or a plugin manifest.)
   - Codex: does `.agents/skills/<id>/SKILL.md` under `CODEX_HOME` get surfaced?
   - opencode: `skills/<id>/` discovery semantics.
   - grok: `.agents/skills/` under the relocated `HOME` — does grok read it at all?
   Confirm the seeded `description:` (from `summary`, else a generic fallback) is
   good enough to trigger auto-invocation; measure false-negative rate.
2. **Prove the premise with numbers.** Measure real system-prompt token cost of
   an MCP injected as tools vs. as a (working) skill, to confirm the saving is
   worth the machinery.
3. **Pick the mechanism per harness:**
   - *Deferred load* — enable the real MCP server only after the skill fires.
     Needs a harness that can add an MCP server mid-session. **Check which
     harnesses allow this at all** (via the I/O bridge? a hook? restart?). Likely
     none do cleanly in passthrough — verify.
   - *Proxy tool* — `am` exposes one thin "call the `<id>` MCP" tool (one schema,
     not twelve) and proxies to the real MCP behind it. **`am` already has
     in-process MCP hosting (`inproc-mcp` feature, `src/mcp/server.rs`)** — check
     whether that can back the proxy so this works on *any* harness in CLI mode.
4. **Stop double-injecting.** For a real saving, the provisioner must **not**
   inject the real MCP into `mcp.json`/`config.toml` when `expose = "skill"`
   (inject a proxy, or nothing until the skill fires). Today it injects both.
   Decide and implement the target provisioning shape.
5. **Cross-harness fidelity.** grok/others with weaker skill support may not be
   able to honor this at all — decide the per-harness fallback (inject as tools +
   warn?).

---

## 2. Credential login-capture — follow-ups

The `am account login <id> --harness <h>` flow is implemented (contract +
command + all four harness `login()` impls + reuse). Remaining:

- **Metadata extraction (the documented "plus") is not built.** `Account.captured:
  BTreeMap<String,String>` exists but is never populated. The per-harness
  "Extractable metadata" tables in `_docs/harness/<h>.md` list the non-secret
  fields to parse (auth type, plan tier, redacted identity, expiry). Implement a
  per-harness parse of the *captured* credential file into `captured`, **redacting
  identifying fields** (email, account uuid) per those tables. Never store token
  values.
- **Login argv unverified for opencode & grok.** The dev sandbox has no
  `opencode`/`grok` binary, so `opencode auth login` and grok's bare-run OAuth are
  transcribed-from-docs, not run. Verify on a real machine.
- **Codex headless flag doc drift.** `_docs/harness/codex.md` says `codex login
  --device-code`; the installed codex-cli 0.142.5 actually uses `--device-auth`
  (flagged in a `codex.rs` code comment). Fix the doc, and decide whether
  `cmd_login` should offer a `--device`/headless mode for sandboxed (no-browser)
  logins.
- **grok reuse tradeoff.** To make captured grok creds reusable, `Grok::provision`
  now honors `account.home` (HOME → the account home), which **co-locates per-run
  injected config with the persistent creds** (user-settings rewritten each run,
  skills accumulate in the home). Confirm this is acceptable or scope injected
  config into a subdir / clean it per run.
- **Codex config.toml clobber on reuse.** Reuse re-provisions `config.toml` into
  `CODEX_HOME = account.home`, overwriting the login-time
  `cli_auth_credentials_store="file"` (and any other keys). `auth.json` reuse is
  unaffected, but verify nothing else important is lost.

---

## 3. grok non-invasiveness (T-10X) — documented limitation

grok writes session/log files to the **real** `~/.grok/sessions` and `~/.grok/logs`
even under a relocated `HOME`, because it resolves those paths via
`os.userInfo().homedir` (getpwuid), which ignores `$HOME` and has **no env-var
override**. Config/skills isolation via `HOME` still holds. Documented in
`grok.rs` + `grok.md`. **Revisit if** grok adds a config/data-dir env var, or if a
heavier lever (a `getpwuid` shim, a fakeroot, or a per-user throwaway account) is
acceptable for full isolation.

---

## 4. Structured I/O acceptance pass — blocked on real logins

§11 of the test plan (`--io structured` for claude/codex/opencode) is `BLOCKED`
in automated runs: it needs a human-authenticated harness (claude "/login", codex
auth, opencode provider login). Run a manual pass once logged in to clear T-11.
(grok structured is intentionally unsupported — asserted, not a gap.)

---

## 5. Storage migration to `~/.config`

All stores now default under `~/.config/agent-manager/` (config, accounts,
catalog, sessions, runs), each with its own env override
(`AM_CONFIG_FILE`/`AM_CONFIG_FOLDER`, `AM_ACCOUNTS`, `AM_CATALOG`, `AM_SESSIONS`,
`AM_RUNS`). Open:

- **No migration from the old location.** Pre-existing data under the macOS
  `~/Library/Application Support/agent-manager/…` (accounts/catalog/sessions) is no
  longer read. Decide: leave as a clean break (alpha), add a one-time migration,
  or add the old path as a last-resort read fallback.
- **XDG deviation.** Putting `sessions/` and `runs/` (transient state) under
  `~/.config` follows the "everything in one place" directive over strict XDG
  (which would use state/cache). Revisit if it complicates packaging.

---

## 6. Account model / import

- **`import --write` merges by skip, not by field.** It's now idempotent (never
  appends a duplicate id — `partition_new`), but it only *adds new* ids; it does
  not update an existing account whose references changed. Consider a structured
  TOML merge if that's wanted.
- **opencode account is provider-agnostic-by-blast.** `Opencode::provision` sets
  **both** `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` from one env ref, and
  `base_url`→provider config is a `TODO`. Make it provider-aware.

---

## 7. Misc harness gaps (from the harness docs / provisioners)

- **Codex resume** has no CLI flag; real resume is an app-server `thread/resume`
  JSON-RPC call — deferred to the bridge step.
- **Hooks** are a no-op for grok/opencode (no native slot) — by design, but worth
  a neutral cross-harness hook mapping later.
- **gemini / copilot** are documented but unwrapped (no `Harness` impl); the
  9 reference harnesses likewise. Transcribing a doc into an impl is the on-ramp.


--

## 8. harness alias: `claude-code` should be activable also with `claude`
