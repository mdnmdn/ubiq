# Harness implementation checklist

This is a working checklist for turning a harness's runtime contract
(`_docs/harness/<id>.md`, structured per `_docs/harness/structure.md`) into a
real `Harness` impl the `am` CLI can launch. It's distilled from the five
harnesses actually wrapped so far — `claude.rs`, `codex.rs`, `opencode.rs`,
`grok.rs`, `copilot.rs` — and the trait itself in `src/harness/mod.rs`.

**Not a per-harness doc.** `structure.md` governs `_docs/harness/<id>.md` (the
harness's *native* runtime contract, harness-neutral). This file governs the
*implementation* step that turns that contract into Rust — read it alongside
whichever `<id>.md` you're transcribing. See `AGENTS.md`'s "Supported
harnesses" table for current status and `_docs/profiles.md` §5 for the
Class A/B/C taxonomy referenced throughout.

Per this repo's standing rule: **adding a harness is a pure extension** — a
new `src/harness/<id>.rs` (and, if structured I/O is in scope, a new
`src/io/<id>.rs`) plus a few registration lines. If a step below tempts you to
special-case the new harness's id inside `provision.rs`, `harness/mod.rs`'s
generic helpers, or `io/mod.rs`'s core model — stop; that almost always means
the behavior belongs in a trait method override instead.

**A doc is a claim about a point in time, not the binary.** Copilot's first
pass (transcribed carefully from `_docs/harness/copilot.md`) shipped with a
real, user-reported bug: `login()` ran `copilot auth login`, but the installed
CLI (1.0.69) has no `auth` subcommand namespace at all — the real command is
`copilot login`. Re-verifying against the actual binary that same session also
found the config-dir lever the doc said didn't exist (`COPILOT_HOME` — moving
the harness from Class C to Class A, dropping a HOME-relocation cost that was
never necessary), a wrong MCP config filename/schema/top-level key, a
fabricated env var (`COPILOT_TOKEN`, which doesn't exist), a hooks feature
the installed binary shows no evidence of supporting, and a model-discovery
path that had been given up on as "too fragile to scrape" when the real
`--help` output turned out to be a stable, machine-parseable block. See
`src/harness/copilot.rs`'s module doc for the full list, written the way any
future correction should be: specific, dated, and citing the exact `--help`
invocation that proved it. **Treat every fact in a harness doc — including
this checklist's own Copilot references below — as a hypothesis to confirm
against `<binary> --help` (and a scratch run/dir where safe) the first time
you actually implement against it, not settled truth.**

## 1. Read the contract doc first

- [ ] `_docs/harness/<id>.md` exists and follows `structure.md`'s section
      ordering. If it's still a "reference" doc (no `Harness` impl yet, see
      AGENTS.md's second table), check its facts are marked appropriately
      ("Not documented" / "not verified against the installed binary") rather
      than presented as fact — you may need to verify against a real install
      as you go, same as any reference-doc gap.
- [ ] Note which sections exist specifically for this: "Credential capture &
      reuse (agent-manager)" and "Model discovery & selection (agent-manager)"
      — these are already written for the `Harness` impl step, not just human
      reference. If either is missing, you're the one adding it (verify
      against the real binary where possible; say so in the doc when you
      can't).

## 2. Classify the config/credential relocation model (Class A/B/C)

Per `_docs/profiles.md` §5 — this decision drives `config_anchor()`
and the whole `provision()` shape, so make it first and get it right:

- [ ] **Class A — unified root**: one env var relocates config *and*
      credentials (Claude's `CLAUDE_CONFIG_DIR`, Codex's `CODEX_HOME`,
      Copilot CLI's `COPILOT_HOME` — the last one initially looked Class C
      until the lever was actually tested; see the Copilot lesson above).
      `HOME` is never touched.
- [ ] **Class B — split store**: a config-dir lever exists, but credentials
      live in a separate HOME-relative (or other-lever-relative) location
      (opencode: `OPENCODE_CONFIG_DIR` for config, `XDG_DATA_HOME` for the
      auth store — verify empirically that the data lever actually
      relocates credential reads before trusting it; opencode's `profiles.md`
      §12 B-1 note is the precedent for how to record that verification).
- [ ] **Class C — HOME-only**: no config-dir lever at all; the harness's
      whole tree derives from `$HOME` (Grok's `~/.grok`, with no
      `GROK_CONFIG_DIR`-equivalent found on inspection). `HOME` itself must
      relocate — to the account's persistent fake `HOME`, which holds the
      login (`D194`).
      Set `requires_home_relocation: true` — this is the isol8-pairing
      signal (see profiles.md §8): relocating `HOME` strips the user's real
      toolchain (`nvm`/`mise`/`pyenv`, shell rc, PATH shims), which isol8
      reconstructs deliberately.
- [ ] If genuinely unsure between B and C (a plausible-looking data-dir env
      var that might not actually be honored), **verify against the real
      binary** rather than guessing — a wrong lever silently breaks
      credential/MCP/skill injection instead of erroring. See grok.rs's
      module doc for a documented example of a lever that looked plausible
      but was verified *not* to work for session/log writes — don't repeat
      that mistake for a new harness without checking.

## 3. `config_anchor()`

> A new harness needs **no** store changes. Preference templates flow through
> the injected `TemplateStore`. You declare only `config_anchor()` and
> (optionally) `templates()`; the stores are harness-agnostic. No login is
> seeded (`D194`). See `_docs/am-as-library.md`.

- [ ] `levers`: the env var(s) + `Relocate` variant from step 2 (empty for
      Class C).
- [ ] `requires_home_relocation`: `true` only for Class C.

## 4. `provision(spec, dir)`

Work through each composable piece `RunSpec` can carry. Skip a piece only
when the harness genuinely has no slot for it — and say so in a comment
(e.g. "opencode has no documented native hook slot, so `spec.hooks` is a
no-op here — a fidelity gap, not a user mistake").

- [ ] **Skills** (`spec.skills`): copy each into the harness's *user-tier*
      skill location under the relocated/ephemeral dir (`copy_dir_recursive`
      + `write_mcp_as_skill_pointers` for `spec.mcp_as_skill`). Never the
      harness's *project*-tier skill location — that's under `spec.cwd`, the
      user's real project (see the cwd rule below).
- [ ] **MCP servers** (`spec.mcps`): render into the harness's native MCP
      config shape. Get the **exact top-level key** right per the harness's
      own doc — this varies (`mcpServers` vs `servers`, `mcp` vs
      `mcp_servers`) and copy-pasting another harness's key is an easy,
      silent bug. `McpRef::InProcess` is always an error at this layer
      (`bail!("in-process MCP not supported in passthrough mode")` — same
      message every harness uses, for consistency); only `provision.rs`'s
      loopback-HTTP rewrite (behind the `inproc-mcp` feature) turns it into
      something a harness-level renderer ever sees. Match existing
      precedent on whether to write the file unconditionally (codex/claude:
      always, even empty — "the run is fully controlled") or only when
      non-empty (grok/copilot: "unused runs stay minimal, no
      `--mcp-config`-equivalent flag exists") — pick based on whether the
      harness's own docs imply a stub file is expected.
- [ ] **Hooks** (`spec.hooks`): if the harness has a native hook-file slot,
      group `HookRef`s by `event` and render it. If the schema isn't fully
      pinned in the harness's doc (Codex's `hooks.json` is the precedent),
      say so explicitly in a doc comment — a best-effort, unverified render
      is fine, a silently-presented-as-certain one is not.
- [ ] **Instructions** (`spec.initial.instructions`): write to the harness's
      *global* always-on memory file under the relocated dir — never a
      project-root `AGENTS.md`/equivalent under `spec.cwd`.
- [ ] **Policy/permissions** (`spec.policy`): map only the values you can
      point at a real, documented harness setting. Unrecognized values
      (e.g. a Claude-specific `permission_mode` string reaching Codex) get
      a comment explaining the mode wasn't recognized and default keys are
      omitted — never invent a plausible-sounding mapping.
- [ ] **Model** (`spec.model`): the harness's `--model`-equivalent flag (or
      config key), added only when set, so a run without `--model` keeps
      byte-identical argv/config to before the field existed.
- [ ] **Resume** (`spec.resume`): only wire it to a flag/mechanism the
      harness's doc actually documents — and check whether that flag is
      genuinely mode-specific before assuming so. Codex's resume really is
      app-server-only (no interactive equivalent exists at all — a real
      no-op elsewhere is correct there). Copilot's `--resume[=id]` looked
      `-p`-only in an early doc pass but turned out, on checking
      `copilot --help` directly, to be a general top-level flag valid in
      both modes — don't assume a flag's scope from where an example
      happened to show it; check the option's own listing. Also check the
      **argv form**: an optional-value option (`[=value]` in the `--help`
      listing) usually needs `--flag=value` as one token, not `--flag value`
      as two — Copilot's `--resume` is exactly this shape.
- [ ] **Structured vs. passthrough argv**: branch on
      `spec.io == IoModes::Structured` when the two modes need materially
      different flags (headless one-shot vs. interactive). Keep the
      passthrough branch's argv shape stable across unrelated field
      additions — tests should assert exact positions for the
      headless/structured argv (it's a fixed contract) and looser
      "contains"-style assertions for passthrough (closer to "pass these
      extra flags to an otherwise-interactive process").
- [ ] **Account** (`spec.account`): map `api_key_env`/`auth_token_env` to the
      harness's real env var name(s) — when both are set, `api_key_env`
      wins (established convention, e.g. Codex/Copilot). Read the value with
      `std::env::var(name).map_err(...)` naming both the account id and the
      var in the error. **Never write a secret value to disk** — it only
      ever lands in the in-memory `Launch.env`, passed to the child process.
      If the harness has no env-var equivalent for a field (e.g. a
      provider-specific `base_url` scheme), leave a `TODO(P2+)` comment
      naming the config surface that *would* carry it rather than silently
      dropping it or faking an unsupported flag.
- [ ] **Never write into `spec.cwd`.** Every existing harness only ever
      writes into the `dir` it's given (the ephemeral/relocated config
      root) — even when the harness's own doc documents a project-tier file
      it *would* read (Copilot's `.github/copilot/mcp.json`,
      `.github/skills/`, `.github/hooks/*.json` are the concrete precedent
      for a documented seam that's deliberately *not* used). Note the
      decision in a comment so a future reader knows it's deliberate scope,
      not an oversight.

## 5. `login_home(home)`

- [ ] For a harness that `shares_home`: the launch that logs the harness in
      straight into the account's home — the config lever pointed at `home`, the
      harness's own login command. Nothing is captured or read back (`D194`).

- [ ] `login_files()`: the paths, **relative to that home**, in which this
      harness keeps its login (claude `.credentials.json`, codex `auth.json`,
      copilot `config.json`, opencode `opencode/auth.json`, grok
      `.grok/auth.json`). `HomeStore::signed_in` and `home::login_validity` read
      them; nothing copies them. On macOS the login may be in the OS keychain
      instead, so an absent file is not proof of no login.

## 6. `discover_models()`

- [ ] Prefer a real discovery command the harness ships (`codex debug models
      --bundled`, `opencode models`) — parse its actual documented output
      shape, don't guess at undocumented fields.
- [ ] If no dedicated machine-readable discovery command exists, check
      whether a `--help`/human-help block is nonetheless stable and
      structured enough to scrape before giving up on it. Copilot's first
      pass assumed `copilot help config`'s model list was "too fragile to
      parse" and shipped a curated static fallback instead — actually
      running the command showed a stable, consistently-quoted bullet list
      under one settings entry, trivial to parse reliably. Only fall back to
      a curated static list once you've actually looked at the real output
      and confirmed it's genuinely unstructured (free prose, inconsistent
      formatting) — say so honestly in a doc comment ("static curated
      fallback, not machine-discovered", plan/entitlement-dependent where
      relevant) so a future reader doesn't mistake it for live discovery.
- [ ] If neither a command nor a documented static list exists, don't
      implement the method — the trait default (`bail!` naming the harness)
      is the correct behavior until real data exists.

## 6b. `quota()` — how much of the plan is left

- [ ] Read the provider's own documentation (and, for an undocumented endpoint,
      what its own client actually sends) before writing anything. There is no
      shared shape here: the providers disagree on whether a limit is a rolling
      window, a monthly count or a balance, which is what `QuotaReading`'s three
      variants exist for.
- [ ] Declare `io_support().quota` truthfully. `QuotaSource::None` is the
      **correct and permanent** answer for a provider that publishes only a plan
      ceiling as prose (Copilot, opencode, Grok) — the caller draws "this
      provider states no limit" in place rather than hiding the control. Never
      declare `Probe` or `Bridge` for a method you have not implemented: the
      declaration is what a surface offers the question from, so a capability
      that bails is worse than one that was never claimed.
- [ ] Where only a static plan ceiling is known, it belongs in this harness's
      `_docs/` page as prose, **not** in a gauge with an invented numerator.
- [ ] Reach the credential through `AccountStore::login_source`, never by
      naming a path — that is what makes the method work for a store that keeps
      credential bytes as well as one that keeps a home dir.
- [ ] Read the token, spend it, drop it. Nothing token-shaped may reach a
      `QuotaSnapshot`, a log line, or the caller — assert it in a test where the
      credential is a recognisable sentinel.
- [ ] Treat an unofficial endpoint as best-effort: a 429, a 401 or a changed
      schema degrades to a sentence a user reads, never to a wrong number and
      never to a panic. A window that states no utilization produces no gauge
      rather than a zero.

## 7. Structured I/O bridge (only if in scope for this harness)

- [ ] Confirm the harness's output stream protocol is actually pinned
      well enough in its doc to build a faithful mapping — if the doc says
      per-field shapes "aren't documented enough" (Grok's current state),
      leave `io_support().structured = false` and don't override
      `structured_bridge()`; a passthrough-only harness is a legitimate,
      honest end state, not an unfinished one.
- [ ] If in scope, pick the right template based on the wire shape:
  - **Agent Client Protocol** (Grok's `agent stdio`, opencode's `acp`,
    Copilot's `--acp`) → contribute no harness-specific bridge at all: drive
    the launch through the harness-neutral `AcpBridge`
    (`src/io/acp_client.rs`), the same way `structured_bridge()` does in
    `src/harness/grok.rs`, `opencode.rs` and `copilot.rs`. The prompt is a
    `session/prompt`, resume is `session/load`, permissions are real
    `session/request_permission` round trips — no `src/io/<id>.rs` to write.
  - Bidirectional NDJSON with an approval handshake over stdin (Claude Code)
    → add `src/io/<id>.rs` implementing `IoBridge`, mirroring `src/io/jsonl.rs`.
  - JSON-RPC, harness-specific envelope (Codex's `app-server`) → add
    `src/io/<id>.rs`, mirroring `src/io/codex.rs`.
  - Map only event types/fields the doc actually documents. If a step in
    the protocol (e.g. a tool-call *start* event) isn't documented — only
    its *completion* is — don't synthesize one; emit what's real and say so
    in a one-line comment.
  - Register in `src/io/mod.rs` (`pub mod <id>; pub use <id>::<Id>Bridge;`) —
    only for a harness-specific bridge; an ACP harness needs no entry beyond
    what's already there for `AcpBridge`.
  - Wire `Harness::structured_bridge()` to `spawn_piped` +
    `<Id>Bridge::new(child)`.
  - Write `map_event` as a pure function tested directly against hand-built
    `serde_json::json!(...)` values — no process spawning in unit tests.

## 8. Templates / `post_seed` — only if the harness needs them

Most harnesses need neither; both default to no-ops. Reach for one only when
the harness's own onboarding wizard would otherwise block every ephemeral,
always-first-run config dir (see `_docs/profiles.md` §14 for the full
rationale):

- [ ] **`Harness::templates()`** — for genuine, user-tunable *preferences*
      (theme, TUI mode, an opt-in default) that should live in an editable
      file under `~/.config/agent-manager/templates/<harness-id>/`, not a
      Rust literal. Gap-filling merge only — never overrides a key the run
      itself already generated.
- [ ] **`Harness::post_seed()`** — for *correctness* requirements that look
      similar but aren't preferences: a flag a non-interactive login capture
      never sets (Claude's `hasCompletedOnboarding`), forced unconditionally
      on every run.
- [ ] If in doubt which one: would a user ever plausibly want this value to
      differ between runs or accounts? Preference → template. Would leaving
      it unset break the run (wizard/dialog blocking headless execution)
      regardless of user preference? Correctness → `post_seed`.

## 9. Tests

Match the existing suites' coverage (grok.rs/opencode.rs/codex.rs/copilot.rs
are the templates) — at minimum:

- [ ] `provision` writes the expected files (skills/MCP/hooks/instructions)
      with the right shape/keys, sets the right launch env/argv, **and
      leaves a stand-in fake `$HOME` untouched** (the core isolation
      invariant every existing test asserts).
- [ ] Empty/absent input (no MCPs, no hooks, no skills) stays minimal —
      matches whatever unconditional-vs-only-when-non-empty choice step 4
      made.
- [ ] A skill pointing at a nonexistent path is an error naming it.
- [ ] `McpRef::InProcess` is an error mentioning "in-process".
- [ ] Account `api_key_env`/`auth_token_env` map to the right env var(s);
      an unset var names itself in the error.
- [ ] A **no-secret-on-disk invariant**: `walkdir` the provisioned dir after
      an account-env test and assert the secret value never appears in any
      file's contents.
- [ ] `login_home()` points the relocation lever at the home.
- [ ] `resolve(<id>)` finds the harness.
- [ ] Structured-vs-passthrough argv shape, including any resume/model
      flag placement decided in step 4.
- [ ] `discover_models()`, if implemented.
- [ ] `quota()`, if implemented: the parse against a real captured answer, a
      window that states nothing producing no gauge, and the credential never
      appearing in the snapshot (step 6b).
- [ ] Bridge `map_event` tests for every documented event type (step 7).

## 10. Wiring (mechanical, small diffs)

- [ ] `src/harness/mod.rs`: `mod <id>;`, `pub use <id>::<Id>;`, add to
      `all()`, extend `structured_io_support_matches_landed_bridges`'s
      match arm if structured I/O landed.
- [ ] `src/io/mod.rs`: `pub mod <id>; pub use <id>::<Id>Bridge;` if a bridge
      landed.

## 11. Verification

```bash
cargo build -p agent-manager --all-features
cargo build -p agent-manager --no-default-features   # core must build without cli/pty
cargo clippy -p agent-manager --all-features -- -D warnings
cargo clippy -p agent-manager --no-default-features -- -D warnings
cargo test  -p agent-manager --all-features < /dev/null   # PTY tests need non-interactive stdin
```

All five must be clean before calling the harness done — `--no-default-features`
and the second `clippy` pass catch different things than the default build
(lib-mode/core-only compilation; lints the plain `build` doesn't run).

## 12. Documentation

- [ ] `AGENTS.md`: move the harness's row in "Supported harnesses" from
      "documented" to "**wrapped**" (with its class/io-support noted), add
      it to the "each have `Harness` implementations" sentence, add
      `<id>.rs` to the `src/harness/` (and `src/io/`, if applicable)
      repository-layout tree, add a `cargo run -- <id> ...` example line.
- [ ] `_docs/profiles.md` §5's Class A/B/C table: add the harness to
      its class's example list if it's a new instance of an existing class
      (or document a genuinely new class, if one shows up). Update §11/§13
      ("Where it lives") if the file list changed.
- [ ] This file: if the harness surfaced a genuinely new pattern (a fourth
      relocation class, a new bridge template shape, a new reason to reach
      for `post_seed`), fold the generalizable lesson back in here so the
      next harness benefits — but don't restate harness-specific facts that
      belong in `_docs/harness/<id>.md` instead.
