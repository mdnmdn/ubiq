---
id: inbox-assist
title: Proposal — on-device models for the small namings
kind: proposal
status: proposal
summary: A tiny text generation the host can perform without a harness, an account or a network — the OS's own on-device model behind a backend trait with a stub for every platform that lacks one, reached by a message family in which the interface names a subject and never a prompt, off by default and switched on in settings only where the OS says the model is there.
read_when: you are deciding whether Ubiq may call a model directly, where an AI-suggested name comes from, or how a macOS-only capability is made optional without leaking into the interface
updated: 2026-09-07
depends_on: [tech-architecture, tech-transport, tech-agent-manager, feat-chat, feat-workbench, feat-sessions, inbox-config, inbox-editions]
---

# Proposal — on-device models for the small namings

**Ubiq calls no model.** Every model interaction in the tree goes through a spawned harness —
`Agents::compose_run` in `crates/ubiq-host/src/agent.rs` composes either a passthrough pane or a
structured conversation, and there is no "run one prompt, give me a string" path anywhere. There is
no HTTP model client either: the three crates that depend on `ureq` use it for MCP, for connector
OAuth and for Ubiq's own loopback doc server.

That is the right default for anything a user reads as work. It is the wrong shape for the
half-sentence namings Ubiq does mechanically today:

> A conversation is called `claude 3`, a session is called after its project, a chat tab that is
> attached to nothing is called `New chat`, and the commit box has no consumer at all.

Spawning a harness to name a conversation is absurd — it costs a process, a credential and a
round-trip to a paid endpoint for eleven characters. But an operating system that ships a small
model on the machine makes the same naming free, private and accountless, which is exactly the
property that lets it sit inside Ubiq rather than beside it. `refs/app-ly` already proves the
integration end to end in a Tauri shell; its shape, its availability discipline and its build
gotchas are borrowed here wholesale.

Four proposals in order of what they cost, each shippable alone: the rule, the message family, the
backends, and the places a suggestion lands.

## 1. Where it stands

**Nothing summarizes anything.** Case-insensitive search across `_docs/` and `crates/` for
`apple intelligence`, `foundation model`, `on-device`, `phi silica` and `LLM` returns zero hits.
Four naming surfaces, all mechanical:

| Surface | Where the name comes from | Owner |
|---|---|---|
| Conversation | `unique_name()` — the harness label plus a per-project counter | `crates/ubiq-host/src/coordinator.rs` |
| Session | The project's name, or the literal `session` | `crates/ubiq-host/src/coordinator.rs` |
| Chat tab | The attached agent's name, or the literal `New chat` | `crates/ubiq/src/ui/dock/mod.rs` |
| Task | The user types it; `Work::create()` refuses an empty one | `crates/ubiq-host/src/work/mod.rs` |

A harness may name its own conversation and Ubiq plumbs it through —
`AgentEvent::SessionInfoUpdate` becomes a `ConvUpdate::Title` in
`crates/ubiq-host/src/conversation.rs`, and `refresh_agent_record()` in `crates/ubiq/src/app/mod.rs`
turns it into the name a reader sees. That function is the seam, and its own comment says so: a
conversation the harness has not named keeps the registration default. Ubiq never generates one.

The commit box is blanker still. `commit_box()` in `crates/ubiq/src/ui/git/changes.rs` is a bound
`Textarea`, an amend checkbox and a disabled button, and its module header states that nothing there
commits — the box keeps what is typed so the thought is not lost.

**Settings are built and two-layered.** `SettingsLayer` in `crates/ubiq-proto/src/settings.rs`
splits an opaque `Ui` blob the host stores verbatim from a `Host` record the host parses and acts
on; they land in `ui-settings.toml` and `host-settings.toml` via `FileSettingsStore`. The governing
rule is in that file's own comment — the half that mutates a field owns it. A toggle is five edits,
and `toggle_isolate_agents` in `crates/ubiq/src/app/settings.rs` is the host-layer precedent.

**`crates/agent-manager` cannot help here, and that is the point.** An `Account` in
`crates/agent-manager/src/account.rs` carries credential *references* only — env-var names, a
gateway URL, a helper command it never runs. There is no notion of a provider Ubiq could call. An
OS-provided model needs no account, no key, no keychain entry and no credential path, so it
sidesteps that boundary entirely instead of straining it.

## 2. The rule

**The interface names a subject. The host owns the prompt.**

A request says *what is being named* — this conversation, that diff — and never carries prompt text.
The host holds every prompt string, every instruction, every schema. Three things follow, and they
are the reason this rule is worth more than the convenience it costs:

- The interface cannot become a generic model console. A message family that accepted a prompt would
  be one, whatever it was called, and every later feature would reach for it.
- Prompts are testable where the rest of the host is tested, against a fake backend, with no window.
- A prompt is a fact with one owning document. Changing how a conversation is named is a host change
  and a documentation change, not a change in two crates.

Second rule, narrower: **a suggestion arrives as a proposal in an editable field, never as a
committed name.** It fills the box the user was going to type in. Nothing is renamed behind anyone's
back, nothing is written to a repository, and a suggestion that never arrives leaves the mechanical
name exactly as it is today.

## 3. The message family

A new family, `assist`, in `crates/ubiq-proto/src/messages.rs`, following the search family's shape:
the interface mints the id and discards every reply naming a request it is not holding.

```rust
/// Ask whether on-device assistance can run here. Answered with [`Message::Assist`].
GetAssist,
/// Ask for a short suggested text for something Ubiq already holds. The interface names the
/// subject; the prompt is the host's. Answered with [`Message::Suggestion`].
Suggest { suggest_id: SuggestId, subject: SuggestSubject },
/// Abandon a suggestion. Best effort: the model may already be running.
CancelSuggest { suggest_id: SuggestId },
```

```rust
/// Whether assistance is available, and if not, why — in words with no vendor in them.
Assist { available: bool, reason: Option<AssistReason>, detail: Option<String> },
/// The suggested text for a subject. Advisory: nothing has been renamed.
Suggestion { suggest_id: SuggestId, text: String },
/// A suggestion could not be produced. The subject keeps its mechanical name.
SuggestError { suggest_id: SuggestId, error: String },
```

`SuggestSubject` is a closed enum, one variant per call site, each carrying only ids:
`ConversationName { agent_id }`, `SessionName { session_id }`, `TaskTitle { project_id, task_id }`,
`CommitMessage { project_id }`. The host reads the material it needs from what it already holds —
the first user turn of a conversation, the tasks in a session, the staged hunks — so no transcript
crosses the bus to come straight back.

`AssistReason` is the closed vendor-neutral set `refs/app-ly` arrived at and tests:
`unsupported-platform`, `disabled-by-setting`, `unsupported-os`, `device-not-eligible`,
`not-enabled`, `model-not-ready`, `unavailable`. The framework's own reason strings are re-written
in Rust rather than forwarded; app-ly asserts in a unit test that no detail string names Apple, and
that test is worth copying, because a settings panel that says *Apple Intelligence is not enabled*
on a Windows machine is the failure mode it prevents.

`Suggest` is not routed by pane, so it needs no row in `pane_id_of`; the project-addressed variants
need one in `Message::project_id()`. Per the transport contract's six-step procedure, the family
gets its two tables in `tech/transport-contract.md` in the same commit, and this is a structural
choice, so it gets a row in `tech/decisions.md`.

## 4. The backends

Host-side, in a new `crates/ubiq-host/src/assist/`. One trait, one gate, expressed once — the
structure `refs/app-ly` arrived at, where every caller is platform-free and both backends produce
the same boxed trait object from one constructor:

```rust
#[cfg(all(target_os = "macos", feature = "assist-apple"))]
mod apple;
#[cfg(not(all(target_os = "macos", feature = "assist-apple")))]
mod stub;
```

The FFI belongs here and not in `crates/ubiq`, even though `objc2` and `objc2-foundation` are
already linked there for the dock icon in `crates/ubiq/src/ui/ribbon.rs`. The coordinator renders
nothing and the interface reaches around nothing; a capability the host performs is a host
dependency, under `[target.'cfg(target_os = "macos")'.dependencies]`, beside the existing
`#[cfg(target_os = "macos")]` in `crates/ubiq-host/src/cli_shortcut.rs`.

**macOS.** Apple's FoundationModels is a Swift-only API — `objc2` has no binding for it and cannot,
since there is no Objective-C metadata to consume. Two crates wrap it through a Swift bridge they
compile: `foundation-models` (which app-ly pins at `0.11`, feature `macos_26_0`) and `fm-rs`, the
more used of the two. Either needs a Swift toolchain and the macOS 26 SDK **at build time**, which
is the real cost of this proposal and the reason the cargo feature exists.

Requirements are narrow and must be stated where a user reads them: macOS 26 or later, Apple
silicon — Intel Macs are excluded outright — Apple Intelligence switched on, model downloaded,
7 GB free, and a device language among the sixteen the model supports. Availability is not inferred
— it is asked, via `SystemLanguageModel::availability()`, whose three unavailable reasons
(`deviceNotEligible`, `appleIntelligenceNotEnabled`, `modelNotReady`) map one-to-one onto
`AssistReason`. There is no OS-version check in Ubiq's code; `OsTooOld` comes from the framework.

Three framework limits shape the host side. The context window is 4,096 tokens per session, read at
runtime rather than hard-coded, which means the material a subject gathers is **truncated to a
budget the host owns** — the first user turn, not the transcript; the diff stat, not every hunk. A
session refuses concurrent requests, so it is one session per request, as app-ly's backend comments
already conclude. And both the prompt and the output are screened, so a `guardrailViolation` on
terminal output Ubiq did not author is an ordinary outcome rather than a bug — which is the second
argument for the advisory rule, after the first.

**One build gotcha, load-bearing.** The Swift concurrency runtime is referenced as
`@rpath/libswift_Concurrency.dylib` and a Rust link emits no `LC_RPATH`, so the app dies at launch
with `dyld: Library not loaded`. The fix is one line — `cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift`
— and `cargo:rustc-link-arg` does not propagate to dependents, so it must be emitted by the *binary*
crate. In Ubiq that means a `build.rs` in `crates/ubiq-app`, gated on target and feature, and
`crates/ubiq-app` is already the only crate that names both halves.

**Windows: not yet, and the trait is the preparation.** `Microsoft.Windows.AI.Text.LanguageModel` is
the right API and `TextSummarizer` is literally this use case, but three gates disqualify it for a
Rust desktop tool today. There is no Rust projection at any level of officialness — `windows` covers
OS metadata only, `microsoft/windows-app-rs` was archived in 2022 with the note that the App SDK is
too tied to .NET to be usable from other toolchains, and every real version of the `windows-app`
crate is yanked. Phi Silica is a Limited Access Feature whose unlock token binds to a Package Family
Name, so it needs MSIX identity and a form. And it needs a Copilot+ NPU or a discrete GPU; CPU is
unsupported.

The re-evaluation trigger is dated rather than vague: Microsoft's own docs say Phi Silica is replaced
by Aion Instruct in retail in November 2026, keeping the `LanguageModel` API, dropping the token
requirement and running on CPU. If that holds, the remaining cost is MSIX identity plus generating
bindings with `windows-bindgen` against the App SDK winmd — which windows-rs's own Windows Reactor
does in-tree, so it is tractable. Until then the Windows backend is the stub, and the stub is not a
failure mode. It is the supported way to build Ubiq without an AI toolchain.

**Bundling a model is out of scope, and if it ever is not, it is `candle`.** A 0.5B model handles a
five-word title comfortably, and three routes exist. `candle`'s CPU backend is pure Rust — no cmake,
no libclang, no redistributable library — and loads GGUF directly, which makes it the only one whose
build cost is nothing; `llama-cpp-2` is faster and needs MSVC, cmake and libclang on every machine
and every CI runner; Microsoft's `foundry-local-sdk` is a real Rust crate that adds about 20 MB and
downloads NuGet packages at build time under non-OSS terms. All three then cost a 100–500 MB model
download and a model-management surface Ubiq has no other use for. Detecting a local Ollama or LM
Studio on loopback costs nothing and works everywhere; it is also a network client, a port, a trust
question and a second availability story. Each is a backlog row behind the same trait, not part of
this.

## 5. The setting

Off by default, and the toggle is host-layer, because the host is the half that acts on it.

| | |
|---|---|
| Field | `assist_on_device: bool`, `#[serde(default)]`, default `false` |
| Struct | `HostSettings` in `crates/ubiq-proto/src/settings.rs` |
| Schema | `HOST_SETTINGS_SCHEMA` moves 7 → 8, with the reason in its doc comment beside the others |
| Section | A new `SettingsSection::Assist`, labelled *Assistance* |
| Handler | `toggle_assist_on_device` beside `toggle_isolate_agents` in `crates/ubiq/src/app/settings.rs` |

The setting is checked before the backend, so a user who switches it off gets
`disabled-by-setting` and no FFI call is made at all — app-ly's `availability()` does exactly this,
and the ordering matters: a config answer needs no model and no device.

The section holds the master toggle and one checkbox per call site, because *suggest a commit
message* and *name my conversations* are genuinely different appetites. Where the model is not
available the rows draw disabled with the `detail` string beneath them, so the panel explains a
missing capability instead of hiding it — the same choice `harnesses` already makes for a harness
that is not installed. A build without the feature says *this build has no on-device model backend*
and nothing more.

## 6. Where a suggestion lands

Four call sites, in the order they are worth doing:

**A conversation's name.** `unique_name()` in the coordinator is the single best plug-in point: one
function, host-side, already the authority on a display name and already broadcast. The mechanical
name is assigned as it is today, the suggestion is requested after the first user turn, and it
arrives as a rename. This needs a rename message on the wire, which does not exist — closing `G119`
is a prerequisite, not a side effect, and it is worth having regardless.

**A commit message.** The cleanest target in the tree: the widget exists, has no consumer, and
filling it writes nothing to a repository, which keeps the standing rule in `tech/version-control.md`
intact. A button beside the box, a spinner, text in the `Textarea`, fully editable.

**A session's name and note.** `WorkSession` has no description on the wire at all, which is why the
agents sidebar borrows a task's title as the session note (`G81`). A suggested note needs that field
first.

**A task title.** Lowest value — the user is already typing it, and `Work::create()` refuses an
empty one. Worth it only as a suggestion offered while the field is empty.

## Failure

| When | What happens |
|---|---|
| Non-macOS, or built without the feature | `Assist { available: false, reason: unsupported-platform }`. The settings rows draw disabled; nothing else in the interface changes |
| Setting off | `disabled-by-setting`, before any FFI call |
| macOS 26 absent, Intel Mac, Apple Intelligence off, model downloading | The framework's reason, mapped and re-worded. `model-not-ready` is retried silently rather than shown as an error |
| A suggestion takes too long | The host drops it after a deadline and answers `SuggestError`. app-ly times only tool calls and has no generation deadline; a naming that hangs is worse than a mechanical name, so this one has one |
| Cancelled | The reply is discarded. Inference may still be running — the flag stops forwarding, it does not stop the model |
| The model refuses | A guardrail can fire on terminal output nobody wrote for it. `SuggestError`, mechanical name, no retry |
| Rate limited | Backoff. Apple limits a process that is in the background on battery, which Ubiq's foreground window is not, so this is a correctness case rather than a common one |
| The material is too long | The host truncates to its own budget before the model sees it. A subject that cannot be summarized within the context window is not asked about |
| The model returns something useless | It lands in an editable field the user was going to type in anyway |

## Rules this adds

- The interface names a subject and never a prompt. Every prompt string lives in the host.
- A suggestion is advisory. It fills an editable field; it renames nothing and writes nothing to a
  repository.
- An on-device model is not a harness. It gets no catalog entry, no profile, no account and no
  isolation policy, and `crates/agent-manager` does not learn about it.
- No vendor name reaches the interface. Availability is a reason code from a closed set and a
  sentence the host wrote.
- Availability is asked, never inferred. No OS-version comparison, no device allow-list.
- Assistance absent is the normal case. Every call site has to read correctly with the stub backend.

## Phases

1. **The stub.** The `assist` family, the trait, the stub backend, `GetAssist` answering
   `unsupported-platform`, the setting, the settings section drawing disabled. No FFI, every
   platform, fully testable. Nothing user-visible generates anything yet.
2. **The Apple backend.** The `assist-apple` feature, the crate, the `build.rs` rpath line in
   `crates/ubiq-app`, the availability mapping and its no-vendor-names test.
3. **The commit message.** One button, one call site, nothing on the wire that phase 1 did not add.
4. **Conversation naming.** The rename message (`G119`), then the suggestion behind it.
5. **Session note and task title.** Needs the `WorkSession` description field (`G81`) first.

Phase 1 stands alone and is worth landing alone: it is the whole of the optionality, and after it a
platform with no model is a platform Ubiq already handles. Phases 3 through 5 are independent of
each other.

## What this asks to be decided

- Ubiq may call a model directly, for tiny namings only, when the operating system provides one and
  it costs no account, no key and no network.
- The interface never sends prompt text. A generic prompt message is refused now rather than
  regretted later.
- The setting defaults to off, so the tree's current behaviour — Ubiq calls no model — is what an
  existing user still gets after upgrading.
- macOS-only with a documented Windows hole and a dated re-evaluation trigger is acceptable, as
  `G90` already established for confinement on Linux.
- The Swift toolchain is a build-time dependency of one optional cargo feature and never of
  `just verify`.
- Bundling a model, and detecting a local inference server, are both deferred behind the same trait.

## Related docs

- [`tech/architecture.md`](../../tech/architecture.md) — the crate boundary the FFI's placement answers
  to, and the rule that the coordinator renders nothing
- [`tech/transport-contract.md`](../../tech/transport-contract.md) — owns the message set; the `assist`
  family's tables and the six-step procedure for adding them
- [`tech/agent-manager.md`](../../tech/agent-manager.md) — owns every fact about harness configuration,
  and the boundary an on-device model must not be modelled across
- [`features/chat.md`](../../features/chat.md) — owns the conversation and what names its tab
- [`inbox/config-persistence-proposal.md`](../config-persistence-proposal.md) — the persistence
  philosophy the two settings layers sit on top of
