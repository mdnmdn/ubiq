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

## 2. Credential login-capture — dropped

Capture is gone: a login lives in the account's own config home (`D194`).

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

## 6. Account model

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

## 9. OAuth token refresh — dropped

Nothing is copied or written back: the harness refreshes its login in the account's own home (`D194`).

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
