# Open points

Topics known to be unresolved or deferred, gathered from the test runs
(`_docs/test-runs/`), the harness docs, and the credential-login / config work.
Each item says **what**, **why it's open**, and **what to check / do next**.
Ordered roughly by how load-bearing it is. Resolved items are recorded in the
relevant test-run's "Resolution" addendum, not here.

---

## 1. MCP-as-skill — deep dive required ⚠️

**This is the item most in need of a deep investigation before it can be
trusted.** Design + status doc: [`mcp-as-skill.md`](./mcp-as-skill.md).

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

- **Two-tier gap (now documented, not unified).** This crate has two physically
  separate credential trees: `AccountStore`/`FsAccountStore` (`<root>/<id>/`, a
  per-account HOME each harness writes its own files into — what `am account
  login` and the Ubiq embedder's `capture_login` actually use) and
  `SecretStore`/`FileSecretStore` (`<root>/<name>/<harness>/<rel_path>` — what
  `am account rename|delete|check` operate on). An account created by `am
  account login` therefore couldn't be renamed, deleted, or checked by anything
  in this crate. `AccountStore` now has its own `rename_account`/
  `delete_account`/`sign_out`/`login_validity` (see `account.rs`,
  `_docs/am-as-library.md` §5) so an embedder can offer those operations on the
  tier the login flow actually writes — but the two tiers are still separate
  storage, and the CLI's `account rename|delete|check` still only see the
  `SecretStore` tier. Unifying them (one storage model, or a bridge that keeps
  both in sync) remains open.
- **`login_validity` / `effective_harnesses` / `has_capture` triplication.**
  Three call sites now compute essentially the same "is there a login here"
  fact, independently: `account::login_validity` (this change, `AccountStore`
  tier, full `Validity`), `cli::account::effective_harnesses`
  (`cli/account/mod.rs`, `AccountStore` tier, boolean-only, predates this
  change and wasn't refactored onto `login_validity` to keep this change's
  diff scoped to library API), and `ubiq-host`'s `Agents::has_capture`
  (`crates/ubiq-host/src/agent.rs`, boolean-only, outside this crate). All
  three read the same `ConfigAnchor::login_seed` files off an account home.
  Worth collapsing onto `login_validity` (== `Validity::Empty` for the boolean
  case) in a follow-up that touches both crates.
- **Metadata extraction (the documented "plus") is not built.** `Account.captured:
  BTreeMap<String,String>` exists but is never populated. The per-harness
  "Extractable metadata" tables in `_docs/harness/<h>.md` list the non-secret
  fields to parse (auth type, plan tier, redacted identity, expiry). Implement a
  per-harness parse of the *captured* credential file into `captured`, **redacting
  identifying fields** (email, account uuid) per those tables. Never store token
  values.
- **Login argv unverified for opencode & grok.** The dev sandbox has no
  `opencode` binary, so `opencode auth login` is transcribed-from-docs, not run.
  grok 1.0.13 is installed and its ACP path is captured, but it has no login verb
  at all — a captured `~/.grok/auth.json` is the whole account story — so its
  bare-run OAuth is equally unrun. Verify on a real machine.
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
grok is the one harness whose structured path *has* been run against the live
binary with a real account — `grok agent stdio` over the generic `AcpBridge`,
captured frame by frame in `_docs/wip/grok-acp-capture.md` — so §11's grok rows
(and the test plan's "grok passthrough-only" claims) need rewriting to match
rather than clearing.

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


---

## 8. harness alias: `claude-code` should be activable also with `claude`

---

## 9. OAuth token refresh in the copy-on-use approach ✅

**What exists today.** At launch a captured login is **copied** from the persistent account/profile base into the ephemeral run dir (`harness::seed_login` in `src/harness/mod.rs`; the non-credential profile config overlay is symlinked-else-copied by `overlay::materialize`, but credential files are always copied because the harness rewrites them in place). At teardown a credential the run **changed** is copied back: `harness::harvest_login` is the mirror of `seed_login`, called from `run::cleanup` before the run dir goes (and from `Agents::archive` in `crates/ubiq-host/src/agent.rs`, which is where Ubiq's per-pane run dirs go).

Only files marked `SeedFile::credential` are written back — the identity/onboarding companions a login also seeds (Claude's `.claude.json`) are not, because a run rewrites those with its own project history. The origin is whatever `Source` the login was seeded from, recorded on `provision::Provisioned::login_origin`: a `Source::Dir` is written in place (`0600` on unix), and anything else goes to `Harness::adopt_login`, whose only implementation today is Claude Code writing the macOS Keychain back through `account::write_claude_keychain_credentials`. A write-back that fails is a `tracing::warn!`, never an error — a run must not break at teardown.

**Why it mattered.** An OAuth refresh **rotates** the refresh token, so the copy the run left behind was the only live credential and the original it came from was already revoked: discarding the run dir logged the user out everywhere.

**What is still open.** The harvest happens at teardown, so a token rotated mid-run is lost if the process is killed. A watcher on the credential file, writing back as it changes, is the upgrade path (both call sites carry a `ponytail:` note saying so).

---

## 10. A resumed run carries no isolation

**What exists today.** `am session resume <id>` reconstructs a minimal `RunSpec` from the recorded
`SessionMeta` (`cli/session.rs::spec_for_resume`): the harness, the cwd, `config = Fixed(meta.config_dir)`
and the harness-native resume id. `RunSpec.isolation` is left at its default, so the resumed run is
**unconfined even when the original run was confined**, and `run_provisioned` is handed
`RunTail { confined: None, .. }`.

**Why it's open.** `SessionMeta` records the argv, the account and the config dir, not the policy. A
faithful resume needs the isolation the original run resolved — at least the layer name and the home
mode — persisted alongside the rest, and a decision about a *managed* home whose contents the resume
expects to still be there (an ephemeral one is gone by definition, so a resumed run gets a fresh
`$HOME` and the harness re-does its first-run work).

The default home is `inherit`, so the sharp edge here now belongs to an explicit opt-in rather than
to every run: a resumed run finds the real home exactly where it left it. Only a run that chose
`ephemeral` or `managed` resumes into a home the record cannot name.

**What to check / do next.** Add the run's isolation to `SessionMeta` (the layer, the home mode, and
the managed id when there is one), rebuild it in `spec_for_resume`, and decide whether resuming a run
whose home was ephemeral is confined with a new scratch home or refused as unresumable. Until then a
resume is a deliberate step down in confinement, which is the kind of thing that should be said out
loud rather than inferred from a `None`.

---

## 11. Claude tool approval — two loose ends after the real handshake landed

**What exists today.** `io/jsonl.rs` speaks the handshake verified against claude 2.1.258:
`harness/claude.rs` passes `--permission-prompt-tool stdio` on structured runs, a `can_use_tool`
`control_request` becomes an outstanding `AgentEvent::PermissionRequest`, nothing answers it but the
caller, and `AgentInput::Cancel` denies whatever is outstanding before it interrupts the turn (a
`{"subtype":"interrupt"}` `control_request`, which leaves the session open — closing stdin is
`AgentInput::Shutdown`'s separate job). See
[`harness/claude-code.md`](./harness/claude-code.md) §"Tool approval in headless mode" and
§"Process lifecycle".

**Why it's open.** Two things in it are not verified to the same standard as the rest:

1. **`updatedPermissions` is inferred, not captured.** An `allow_always:<n>` option echoes the
   request's own `permission_suggestions[n]` back inside the `control_response`'s `response`
   object, which is the Agent SDK's `PermissionResult` shape — but a live run confirming that the
   session really switches mode (and that a subsequent edit is *not* asked about again) has not been
   done. If it turns out to be ignored, the honest move is to stop offering the `AllowAlways`
   option rather than to offer one that only allows once.
2. **Only `can_use_tool` is understood.** Any other `control_request` subtype gets a `subtype:
   "error"` response and a `warn!` rather than an event, so it cannot stall the turn — but the
   error itself is untested against a real subtype. Nothing in `am`'s argv is known to produce one
   (`hook_callback` and `mcp_message` belong to SDK-hosted hooks/MCPs, which this crate does not
   use).

3. **The interrupt is verified from the binary, not from a live turn.** The request's shape, the
   `subtype:"success"` answer and the "Interrupts the currently running conversation turn"
   description all come out of the installed 2.1.258 bundle's own schema and dispatch (recorded in
   [`harness/claude-code.md`](./harness/claude-code.md) §"Process lifecycle"); the bridge test
   drives a fixture, not `claude`. What is unconfirmed live is *how promptly* a turn with a
   long-running tool in flight actually stops, and what the aborted turn's `result` carries. The
   codex side is the same: `turn/interrupt`'s params come from
   `codex app-server generate-json-schema`, not from an interrupted run.

**What to check / do next.** Run one live turn with an `allow_always` answer and read the second
edit's ask (or absence of one) plus the `result`'s `permission_denials`. If `updatedPermissions`
turns out to be ignored, stop offering `AllowAlways` rather than offer one that only allows once.
For the interrupt, run one live turn that starts a long tool call, write the interrupt, and record
the latency and the `result` line.
