# agent-manager — Feature Test Plan

> **Scope.** This is an interactive / agent-driven acceptance plan for **`am`'s
> own behaviour** — the wrapper features implemented to date (Phase 1–3). It
> does **not** test what the agents *do* (no "did Claude write good code");
> every check is about what `am` provisions, launches, prints, records, and
> exits with.
>
> **Under test:** the four wrapped harnesses — `claude-code`, `codex`, `grok`,
> `opencode`. `gemini` / `copilot` and the 9 reference harnesses have **no
> `Harness` impl** and are explicitly out of scope (see §0.3).
>
> **How to run.** Work top-to-bottom. Do the manual prerequisites (§1) first
> and once; the feature sections (§4–§16) are then mostly agent-runnable and
> non-interactive. Record every result in the log format from §2.

---

## 0. Orientation

### 0.1 The binary

Built as `agent-manager`; invoked here as `cargo run -- …` (or `am …` if you've
symlinked it). Every command below assumes CWD is the crate root
(`crates/agent-manager`) unless stated.

### 0.2 The golden inspection tool: `--print-config`

Most provisioning checks need **no real harness and no login**: `--print-config`
resolves + provisions, then prints the config dir, the launch `argv`, the `env`
set, `env_remove`, and `keep_config` — and exits without launching. Prefer it
for every "does `am` inject X correctly" check. It also keeps the ephemeral dir
around long enough to inspect the files `am` wrote (pair with `--keep-config`).

### 0.3 Harness applicability legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Feature applies and is implemented for this harness — must be tested |
| ⛔ | Deliberately not implemented for this harness (documented gap, assert the *stated* behaviour, not a bug) |
| — | Not applicable |

Wrapped harnesses only: **claude** (`claude-code`), **codex**, **grok**,
**opencode**. A run using `gemini`/`copilot`/any reference id must **error** with
"unknown harness" (covered in §4).

---

## 1. Prerequisites (MANUAL — do these first, once)

These require a human: installing binaries, logging in, and provisioning test
fixtures. Group A is mandatory for the whole plan; Groups B–E are only needed
for the sections that name them (so you can run a partial pass without, say,
logging into all four harnesses).

> Record the outcome of every prerequisite in the log too — a skipped
> prerequisite is why a later section is `BLOCKED`.

### 1.A — Toolchain & build (mandatory)

- [ ] **A1** Rust toolchain present: `cargo --version` succeeds.
- [ ] **A2** Full build: `cargo build` (from crate root) — succeeds.
- [ ] **A3** Core builds featureless: `cargo build --no-default-features` — succeeds
      (proves the lib core has no `cli`/`pty` leakage).
- [ ] **A4** Unit/integration suite green: `cargo test < /dev/null`
      (the `< /dev/null` is required — PTY passthrough tests need a
      non-interactive stdin). Record the `test result: ok` line counts.
- [ ] **A5** Lint clean: `cargo clippy --all-features -- -D warnings`.
- [ ] **A6** Decide the invocation form and note it: `cargo run --` vs an `am`
      symlink. The log should say which you used.

### 1.B — Harness binaries on PATH (per harness you intend to test)

Only needed for **launch** (§10), **structured I/O** (§11), and **live model
discovery** (§9). Pure `--print-config` sections (§5–§8) do **not** need them.

- [ ] **B-claude** `claude --version` works. Needed for §9-claude is N/A (static list).
- [ ] **B-codex** `codex --version` works and is **≥ 0.131.0** (required by
      `codex debug models --bundled` in §9).
- [ ] **B-grok** `grok --version` works.
- [ ] **B-opencode** `opencode --version` works.

### 1.C — Authentication / login (per harness, for LAUNCH + live discovery only)

`am` never logs you in — it wraps an already-authenticated harness. Log in with
each harness's native flow before §10/§11/§9-live:

- [ ] **C-claude** Claude Code authenticated (subscription or `ANTHROPIC_API_KEY`).
- [ ] **C-codex** Codex authenticated (`~/.codex/auth.json` present).
- [ ] **C-grok** Grok authenticated — **and run `grok` once interactively** so
      `~/.grok/models_cache.json` is populated (§9-grok reads that cache).
- [ ] **C-opencode** opencode authenticated (a provider login in
      `~/.local/share/opencode/`).

> ⚠️ **Non-invasiveness is itself under test.** Before any launch, snapshot your
> real config dirs' mtimes: `~/.claude`, `~/.codex`, `~/.grok`, `~/.agents`,
> `~/.local/share/opencode`. After the launch sections, re-check — a run must
> **not** have written to them (§10-X). Do this on a machine whose real config
> you're willing to have *read*.

### 1.D — Test catalog fixture (needed for §5 mcps/skills, §6 catalog)

Build a throwaway catalog so injection is deterministic and independent of your
real `~/.claude`:

- [ ] **D1** `export AM_CATALOG="$(mktemp -d)/catalog"` (note the value in the log).
- [ ] **D2** Create a catalog root with at least: one MCP entry (`stdio` and, if
      possible, one `http`), and one skill folder containing a `SKILL.md` with
      `name:`/`description:` frontmatter. Confirm with `cargo run -- catalog ls`.
- [ ] **D3** Note the ids you created — later steps refer to `<mcp-id>` and
      `<skill-id>`.

### 1.E — Accounts, isolation, settings fixtures (needed for §7, §12, §13)

- [ ] **E1** `export AM_ACCOUNTS="$(mktemp -d)"`; create an `accounts.toml` with a
      reference-only account (e.g. `id="test"`, `api_key_env="SOME_ENV"`). Needed
      for §7 and §5-account.
- [ ] **E2** For §12 isolation: `isol8` (or the configured sandbox command)
      installed, **or** plan to assert the pre-launch wrap only via
      `--print-config` (no sandbox binary needed to see the wrapped argv).
- [ ] **E3** For §13 settings: a scratch project dir with an `am.toml` you
      control (so discovery/merge tests don't depend on your real config).
- [ ] **E4** `export AM_SESSIONS="$(mktemp -d)"` so §14 session history writes to a
      throwaway state dir, not your real one.

---

## 2. How to record results (feedback format)

Create one results file per run: **`_docs/test-runs/YYYY-MM-DD-<tester>.md`**
(agent or human). Copy the fixture values from §1 into its header, then append
one block per test id. Keep it append-only during the run.

### 2.1 Status vocabulary

| Status | Meaning |
|--------|---------|
| `PASS` | Actual matched Expected. |
| `FAIL` | Actual contradicted Expected (a real defect — file it). |
| `BLOCKED` | Couldn't run: an unmet prerequisite (name which, e.g. `1.C-grok`). |
| `SKIP` | Deliberately not run this pass (say why). |
| `N/A` | Feature doesn't apply to this harness (per §0.3). |

### 2.2 Per-test block template

```
### <TEST-ID>  —  <one-line title>
- Harness:   claude | codex | grok | opencode | (n/a)
- Status:    PASS | FAIL | BLOCKED | SKIP | N/A
- Command:   <exact command line run>
- Expected:  <what §X says should happen>
- Actual:    <what happened — paste the salient argv/env/file line, trimmed>
- Evidence:  <config dir path / file snippet / exit code>
- Notes:     <deviations, follow-ups, defect link>
```

> **Redaction rule.** Never paste a secret **value** into the log. Env-var
> **names**, file **paths**, and argv are fine; a resolved `ANTHROPIC_API_KEY=…`
> value is not — write `ANTHROPIC_API_KEY=<redacted>`.

### 2.3 Writing a `FAIL` (error report)

A `PASS` block can stay terse. A `FAIL` is a **defect report** — someone has to
reproduce and fix it from your block alone, so it carries more. The bar: a
reader who has never seen the run can re-trigger the failure and knows what
"correct" would have looked like, **without asking you anything**.

**First, classify what kind of error it is** — this decides who fixes it:

| Class | Meaning | Goes to |
|-------|---------|---------|
| `code` | `am` did the wrong thing (wrong argv/env/file, wrong exit code, missing/extra output). | a code bug — file it |
| `doc` | `am` behaved fine but a doc (§16) says otherwise. | doc-drift ticket |
| `fixture` | The failure is your setup (missing login, bad `am.toml`, unset env). | fix setup → usually becomes `BLOCKED`, not `FAIL` |
| `flaky` | Fails only sometimes. | note the rate (e.g. 3/10) + record it |

Only `code` and `doc` are true `FAIL`s. If it's `fixture`, downgrade to
`BLOCKED` and name the prerequisite.

**Then capture, in this order:**

1. **The delta, in one sentence.** Not "it broke" — *what* differed:
   "argv contained `--model` but `spec.model` was unset" / "exit code was 0,
   harness exited 3" / "secret value found in `mcp.json`".
2. **Severity.** `blocker` if it trips a §17 load-bearing invariant
   (exit-code fidelity, non-invasiveness, no-secret-on-disk, cleanup/retention);
   else `major` / `minor`. State which invariant if `blocker`.
3. **Expected vs Actual as a literal diff** — paste both lines and mark the
   difference. Trim to the offending fragment; don't dump the whole config.
4. **A copy-paste reproducer** — the exact command **plus** the fixture state it
   needs (which `AM_*` vars, which `am.toml`, which flag). If it needs a file,
   inline the file's minimal content.
5. **Evidence** — the config-dir path (run with `--keep-config` so it survives),
   the specific file + line, full stderr, and `echo $?`. For a launch/exit bug,
   both `am`'s exit code and the harness's.
6. **Environment** — build sha, harness version, OS — anything that might make it
   host-specific (esp. for grok's cache and codex's version gate).
7. **Minimised?** — say whether you reduced it (dropped other flags and it still
   fails) or not. A minimised repro is worth far more than a maximal one.

**Redaction still applies** — a no-secret-on-disk failure (T-05g/T-07d) is
proven by the secret's *presence*, so report the **filename + byte offset /
matched env-var name**, never the secret value itself.

#### `FAIL` block template

```
### T-05h-codex  —  --model injected into wrong slot
- Harness:   codex
- Status:    FAIL
- Class:     code
- Severity:  major
- Delta:     model landed in argv (`--model gpt-5-codex`) instead of the
             `model = "…"` key in config.toml
- Command:   AM_CATALOG=/tmp/cat cargo run -- codex --model gpt-5-codex --print-config --keep-config
- Fixtures:  AM_CATALOG=/tmp/cat (no account/settings needed)
- Expected:  config.toml contains `model = "gpt-5-codex"`; argv has NO --model
- Actual:    argv = `codex --model gpt-5-codex`; config.toml has no model key
- Evidence:  config dir /tmp/xxxx/ ; config.toml pasted below ; exit 0
             ----
             [mcp_servers] (…)      # note: no top-level model key
- Env:       build 1a2b3c4 · codex 0.142.5 · macOS 15.5
- Minimised: yes — fails with only --model + --print-config
- Notes:     regression vs §5 T-05h expected behaviour; defect: <link>
```

For a non-repeating (`flaky`) failure, add `- Rate: N/M runs` and, if you have
them, the differing conditions between pass and fail.

### 2.4 Run summary (top of the results file)

```
# Test run — YYYY-MM-DD — <tester>
Build: <git sha>   Invocation: cargo run -- / am
Fixtures: AM_CATALOG=…  AM_ACCOUNTS=…  AM_SESSIONS=…
Harness binaries: claude <v> / codex <v> / grok <v> / opencode <v>
Totals: PASS __  FAIL __  BLOCKED __  SKIP __  N/A __
Open defects: <ids + links>
```

### 2.5 Coverage matrix (fill as you go)

Reproduce this grid and mark each cell with the section's aggregate status:

| Feature \ Harness | claude | codex | grok | opencode |
|-------------------|:------:|:-----:|:----:|:--------:|
| §5 MCP injection | | | | |
| §5 Skill injection | | | | |
| §5 Instructions/prompt | | | | |
| §5 Account injection | | | | |
| §5 Model select `--model` | | | | |
| §5 Hooks | | | | |
| §5 mcp-as-skill | | | | |
| §5 Resume flag | | | | |
| §8 Passthrough argv (`--`) | | | | |
| §9 `--list-models` | | | | |
| §10 Launch + exit code | | | | |
| §10 Non-invasiveness | | | | |
| §11 Structured I/O | | | ⛔ | |

---

## 3. Test id convention

`T-<section><nn>[-<harness>]`, e.g. `T-05a-codex`. Reuse the same numeric id
across harnesses so the coverage matrix lines up.

---

## 4. Meta / dispatch (no harness, no login)

| id | Command | Expected |
|----|---------|----------|
| T-04a | `cargo run -- --version` | prints `agent-manager <semver>`; exit 0 |
| T-04b | `cargo run -- help` (and bare `cargo run --`, and `-h`) | usage: reserved subcommands + **KNOWN HARNESSES** list contains claude-code, codex, grok, opencode |
| T-04c | `cargo run -- gemini --print-config` | **error** "unknown harness 'gemini'" listing known ids; non-zero exit |
| T-04d | `cargo run -- cursor` | same unknown-harness error (reference ids are not wrapped) |
| T-04e | `cargo run -- claude --frobnicate` | clap error naming the unknown flag; non-zero exit |
| T-04f | alias resolution: `cargo run -- claude --print-config` vs `cargo run -- claude-code --print-config` | both resolve to the same harness (`claude-code`) |

---

## 5. Provisioning via `--print-config` (the core matrix)

For each harness, run with `--keep-config --print-config` so you can inspect the
written files. **Baseline first** (T-05z), then one flag at a time. Assert both
the **argv/env** (printed) and the **files on disk** (in the config dir).

### Per-harness config surface (what to look for)

| | config-dir env var | MCP file | policy/settings file | memory file | skills dir |
|--|--|--|--|--|--|
| claude | `CLAUDE_CONFIG_DIR=<dir>` | `mcp.json` (always, even empty) | `settings.json` | `CLAUDE.md` | `skills/<id>/` |
| codex | `CODEX_HOME=<dir>` | `config.toml` (`[mcp_servers.*]`) | `config.toml` | `AGENTS.md` | skills copied |
| grok | `HOME=<dir>` (relocated) | `<dir>/.grok/…` | `<dir>/.grok/user-settings.json` | folded into `--prompt` ⛔ | `<dir>/.agents/skills/` |
| opencode | `OPENCODE_CONFIG_DIR` + `OPENCODE_CONFIG` | `opencode.json` | `opencode.json` | `AGENTS.md` | `skills/<id>/` |

### Tests (run for each of claude / codex / grok / opencode)

| id | Flag under test | Expected |
|----|-----------------|----------|
| T-05z | *(baseline)* `<h> --print-config --keep-config` | launches `program` = the harness command; config-dir env var points at the printed dir; MCP file exists even with zero servers (claude `mcp.json` empty `mcpServers`; opencode `opencode.json` written); `env_remove` lists the harness's hygiene vars (claude: `CLAUDECODE`, …) |
| T-05a | `--mcps <mcp-id>` | the injected server appears in the harness's MCP file with correct transport (stdio → command/args; http → `type:http`,url). claude adds `--mcp-config <path> --strict-mcp-config` to argv |
| T-05b | `--mcp-json ./inline.json` (a `{"mcpServers":{…}}` file) | inline server is **additive** to any `--mcps`; appears in the MCP file |
| T-05c | `--skills <skill-id>` | skill folder copied into the harness's skills dir; `SKILL.md` present at the destination |
| T-05d | `--instructions ./sys.md` | claude → `CLAUDE.md` with managed markers; codex/opencode → `AGENTS.md`; **grok ⛔** → no memory file, content folded into the seeded prompt (assert the prompt carries it) |
| T-05e | `--prompt "hello"` | passthrough mode: prompt appended as trailing positional argv (claude/…); assert it's the last arg |
| T-05f | `--safe` (with `[presets.safe]` in settings) | claude → `settings.json` `permissions` with `defaultMode`/allow/ask/deny; without a `[presets.safe]` defined → **error** "presets.safe" |
| T-05g | `--account test` (from §1.E fixture; env `SOME_ENV` set) | references injected into native slots: claude `ANTHROPIC_API_KEY`/`ANTHROPIC_BASE_URL`/`apiKeyHelper`; **secret value never written to any file in the dir** (grep the config dir — must not contain the value). Unset referenced env var → **error naming the var** |
| T-05h | `--model <id>` | claude/grok/opencode → model in argv (`--model <id>` / `-m <id>` / `--model <provider/id>`); **codex** → `model = "<id>"` key in `config.toml` (not argv). Omitting `--model` → **no** model flag/key (byte-identical baseline) |
| T-05i | `--hooks <hook-id>` (hook defined in settings) | claude/codex → native hook slot written (claude `settings.json` `hooks` grouped by event); **opencode ⛔** → no-op, not an error |
| T-05j | `--mcp-as-skill <mcp-id>` (id also in `--mcps`) | a `SKILL.md` pointer written under the skills dir **and** the MCP still injected normally in the MCP file; naming an id not in the effective mcp set → error |
| T-05k | `--resume <raw-id>` | native resume flag appended: claude `--resume <id>`; opencode `--session <id>` (structured form); **codex ⛔** (no CLI resume — assert documented behaviour); omitting → no resume flag |
| T-05l | `--keep-config` | printed `keep_config: true`; the dir survives after exit (it does, since `--print-config` doesn't delete) |

> **Merge/precedence** of these flags against the settings file is exercised
> separately in §13; here just confirm each flag *alone* provisions correctly.

---

## 6. `am catalog` (no harness, no login; needs §1.D)

| id | Command | Expected |
|----|---------|----------|
| T-06a | `catalog path` | prints the active catalog root (= `AM_CATALOG`); friendly message if unset |
| T-06b | `catalog ls` | lists Skills: and MCPs: sections; your fixture ids appear with description/transport |
| T-06c | `catalog ls --mcps` / `catalog ls --skills` | filters to only that kind |
| T-06d | `catalog show <mcp-id>` | prints the resolved MCP JSON def |
| T-06e | `catalog show <skill-id>` | prints skill path + name + description |
| T-06f | `catalog show nonexistent` | error "neither a skill nor an MCP" |
| T-06g | `catalog import --dry-run` (or `--from <dir>`) | prints an add/overwrite/skip plan + summary; ends with "(dry run — nothing written)"; **writes nothing** (verify catalog unchanged) |
| T-06h | `catalog import --from <dir> --force` then re-run without `--force` | 2nd run reports collisions as `skip` unless `--force` |
| T-06i | bare `catalog` | defaults to `ls` |

---

## 7. `am account` (§1.E for provisioning; §1.B + §1.C for login tests)

| id | Command | Expected |
|----|---------|----------|
| T-07a | `account ls` | lists fixture accounts with the reference kinds they carry, e.g. `(api_key_env, base_url)`; empty store → "no accounts configured" |
| T-07b | `account use test` | writes `[defaults].account = "test"` into the global config.toml; prints the path; unknown id → error with available list |
| T-07c | `account import` | read-only discovery: reports found env-var **names** and credential-file **paths** only (never contents); ends "(dry run …)" without `--write` |
| T-07d | `account import --write` | appends reference-only `[[account]]` snippets to `accounts.toml`; re-`ls` shows them. **Assert no secret values written** |
| T-07e | bare `account` | defaults to `ls` |
| T-07f | **MANUAL:** `account login <new-id> --harness <h>` (e.g. `account login personal --harness codex`) — authenticate with the harness's native login flow | launches the harness interactively in a persistent per-account home (`<accounts-root>/<new-id>/`); on successful login (harness exits 0), writes `<new-id>.toml` with `home = "<full-path-to-accounts-root>/<new-id>/"`; verify with `account ls` that `<new-id>` now appears with `(home)` reference. Subsequent `am codex --account personal` reuses that login without re-authenticating. **Status: typically `BLOCKED` unless login completed by human; record `PASS` if the credential file appeared and `home` was recorded.** |
| T-07g | **MANUAL:** unknown harness: `account login <id> --harness nonexistent` | errors with "unknown harness 'nonexistent'" listing known ids; non-zero exit; no account `<id>` written. Non-zero harness exit: `account login <id> --harness <h>` where the harness exits non-zero (login cancelled/failed) | no account recorded, `accounts.toml` unchanged. **Status: `BLOCKED` (needs live harness binary); mark as PASS if error message and non-invasiveness verified.** |

---

## 8. Passthrough argument forwarding (`--`) (no login)

| id | Command | Expected |
|----|---------|----------|
| T-08a | `<h> -- --version --foo` (each harness) | everything after `--` is appended verbatim to the harness argv (check with `--print-config`); `am` does not try to parse `--foo` |
| T-08b | `<h> --mcps <id> -- extra` | `am` flags before `--` still resolve; `extra` still forwarded |

---

## 9. Model discovery — `--list-models` (per harness)

| id | Harness | Command | Expected |
|----|---------|---------|----------|
| T-09-claude | claude | `claude --list-models` | curated static aliases: `opus`, `sonnet`, `haiku`, `fable` with descriptions; exits without launching. **No binary/login needed** |
| T-09-codex | codex | `codex --list-models` | execs `codex debug models --bundled`, lists real model slugs + display names; hidden ones filtered. Needs codex ≥ 0.131.0 (§1.B) → else clear error |
| T-09-grok | grok | `grok --list-models` | reads `~/.grok/models_cache.json`; lists ids + descriptions, id-sorted. **Un-populated cache → clear error** telling you to run `grok` once (§1.C-grok) |
| T-09-opencode | opencode | `opencode --list-models` | execs `opencode models`, one `provider/model-id` per line. Needs binary (§1.B) |
| T-09-empty | any | (harness reporting none) | prints "no models reported for '<id>'" rather than nothing |

> Round-trip: pick an id from `--list-models` and feed it to `--model` in a
> §5-05h `--print-config` check for that harness.

---

## 10. Launch & lifecycle (needs §1.B + §1.C)

Real launches. Use short, safe passthrough commands so the harness exits quickly
(e.g. `-- --version` where the harness supports it, or a one-shot prompt).

| id | Command | Expected |
|----|---------|----------|
| T-10a | `<h> -- --version` (each harness) | `am` launches the real binary in a PTY, forwards its output, and **exits with the harness's own exit code** (verify with `echo $?`) |
| T-10b | non-zero path: make the harness exit non-zero | `am`'s exit code equals the harness's (passthrough fidelity) |
| T-10c | Ctrl-C during an interactive run | signal reaches the child; `am` doesn't swallow it |
| T-10d | ephemeral cleanup: launch **without** `--keep-config` and no active recorder | config dir is deleted after exit (capture the path from a prior `--print-config`, confirm gone) |
| T-10e | `--keep-config` on a real launch | config dir retained after exit |
| **T-10X** | **Non-invasiveness** (all harnesses) | after all launches, the real config dirs from §1.C (`~/.claude`, `~/.codex`, `~/.grok`, `~/.agents`, `~/.local/share/opencode`) are **unmodified** (mtime/no new files). This is the headline invariant |

---

## 11. Structured I/O (`--io structured`) (needs §1.B + §1.C)

Applies to **claude ✅, codex ✅, opencode ✅**; **grok ⛔**.

| id | Command | Expected |
|----|---------|----------|
| T-11a-claude/codex/opencode | `<h> --io structured --prompt "say hi"` | emits normalized `AgentEvent` NDJSON on stdout (one JSON object per line, parseable); drains and exits cleanly |
| T-11b | `--io jsonl` (alias) | behaves identically to `--io structured` |
| T-11c-grok | `grok --io structured` | **error**: "harness 'grok' does not support --io structured (yet)" (assert the documented gap) |
| T-11d | `--io structured --output acp` | each line is the ACP projection (parseable JSON); non-projectable events dropped, not crashed |
| T-11e | `--io structured --output agui` (and `--output ag-ui`) | AG-UI projection; alias accepted |
| T-11f | `--io structured --output bogus` | error naming the value and the accepted set |
| T-11g | `--io bogus` | error naming accepted `--io` values |

---

## 12. Isolation (`--isolate`) (needs §1.E2)

Can be checked **without** a sandbox binary via `--print-config` (the wrap is
applied before launch):

| id | Command | Expected |
|----|---------|----------|
| T-12a | `<h> --isolate --print-config` | launch argv wrapped: `program = "isol8"`, argv = `["--", "<harness>", …]` (harness and its args moved after `--`) |
| T-12b | `<h> --isolate=dev --print-config` | named profile `dev` threaded in: `program = "isol8"`, argv = `["--profile", "dev", "--", "<harness>", …]` |
| T-12c | *(no flag)* | no wrapping (baseline argv), confirming isolation is off by default |
| T-12d | live (if isol8 installed) | `<h> --isolate -- --version` runs inside sandbox; harness output forwarded; exits with harness exit code |

---

## 13. Settings file & merge semantics (needs §1.E3; no login)

Use `--print-config` to read out the effective set. Fixtures: an `am.toml` with
`[defaults]`, `[harness.<id>]`, `[presets.safe]`.

| id | Scenario | Expected |
|----|----------|----------|
| T-13a | discovery: `am.toml` in a scratch project, run from a subdir | discovered by walking up to the git root; effective config reflects it |
| T-13b | precedence: CLI `--mcps x` over `[harness].mcps=y` over `[defaults].mcps=z` | `x` wins (replace, not union) |
| T-13c | per-harness over defaults (no CLI flag) | `[harness.<id>]` value wins over `[defaults]` |
| T-13d | explicit empty replaces lower layer (`[harness].mcps=[]`) | effective mcps empty |
| T-13e | `--config <path>` explicit file | overrides discovery |
| T-13f | `--catalog <path>` / `AM_CATALOG` | catalog root override honored (CLI over env) |
| T-13g | `.toml` vs `.yaml`/`.yml` fixture | both parse (format by extension) |
| T-13h | CLI empty string: `--mcps ''` (and similarly `--skills ''`, `--hooks ''`, `--mcp-as-skill ''`) | empty string yields empty list, not an error |

---

## 14. Session history (`am session`) (needs §1.E4 + a real launch)

| id | Command | Expected |
|----|---------|----------|
| T-14a | after a §10 launch: `session ls` | the run appears newest-first: id, harness, io, created time, exit code; empty store → "no sessions recorded" |
| T-14b | `session show <id>` | metadata block (harness, cwd, argv, account, io, config dir, timestamps, exit code, harness-session-id); passthrough run → "(no transcript recorded)" |
| T-14c | after a §11 structured run: `session show <id>` | transcript event count + first/last event shown |
| T-14d | `session resume <id>` for a **structured** session with retained dir | re-launches the harness against the retained config dir using the native resume flag; recorded as a new session |
| T-14e | `session resume <id>` for a **passthrough** session (no harness-session-id) | error "no recorded harness session id … cannot resume" |
| T-14f | `session resume <id>` whose config dir was deleted | error "was not retained … cannot resume" |
| T-14g | config-dir retention: a recorded run keeps its dir even without `--keep-config` | dir still present (needed by resume) |
| T-14h | bare `session` | defaults to `ls` |

---

## 15. Environment-variable overrides (no login)

| id | Var | Expected |
|----|-----|----------|
| T-15a | `AM_CATALOG` | sets catalog root (§6, §13-f) |
| T-15b | `AM_ACCOUNTS` | sets accounts root (§7) |
| T-15c | `AM_SESSIONS` | sets session state dir (§14) |
| T-15d | `AM_CONFIG_FILE` | full path to a settings file (highest environment precedence); verify against §13 precedence list |
| T-15e | `AM_CONFIG_FOLDER` | directory containing `config.{toml,yaml,yml}` (mid environment precedence); verify against §13 precedence list |

---

## 16. Reference docs & files — consistency checks

Not agent-behaviour; these assert `am`'s **documentation matches its
implementation**. Quick to do and they catch drift.

| id | Check | Expected |
|----|-------|----------|
| T-16a | `_docs/target/cli.md` flag table vs `RunArgs` | every implemented flag (§5 list) is documented, incl. `--model` / `--list-models`; no documented-but-absent flags |
| T-16b | `_docs/harness/{claude-code,codex,grok,opencode}.md` — each has the mandatory `### Credential capture & reuse (agent-manager)` (§10) and `### Model discovery & selection (agent-manager)` (§13) subsections, correctly placed | present, one each |
| T-16c | `_docs/harness/structure.md` registers both agent-manager subsections as mandatory | present in §10 and §13 templates |
| T-16d | `_docs/harness/gemini.md`, `copilot.md` also carry both subsections (documented-not-wrapped) | present |
| T-16e | `AGENTS.md` "Build & run" examples all execute as written (`--print-config`, `--list-models`, `--model sonnet`, `catalog ls`, `account ls`, …) | each runs without error (login-gated ones may `BLOCKED`) |
| T-16f | `AGENTS.md` supported-harness table matches `harness::all()` (claude/codex/grok/opencode wrapped; rest documented/reference) | matches |
| T-16g | `_docs/target/io-modes.md` structured-support claims match code | claude/codex/opencode structured; grok passthrough-only |
| T-16h | `_docs/reference/multica.md` + `refs/multica/` submodule present and cited | file exists; submodule checked out |

---

## 17. Exit criteria

A pass is **complete** when:

1. Every §4–§15 test has a non-empty status for every applicable harness cell in
   the §2.5 matrix (`N/A`/`BLOCKED`/`SKIP` allowed, but justified).
2. **Zero `FAIL`** on: passthrough exit-code fidelity (T-10a/b), non-invasiveness
   (T-10X), no-secret-on-disk (T-05g, T-07d), and the ephemeral-cleanup /
   retention pair (T-10d/e, T-14g). These are the load-bearing invariants.
3. §16 doc checks pass or have a filed drift ticket.
4. The results file (§2) is committed under `_docs/test-runs/` with the run
   summary totals filled in.
