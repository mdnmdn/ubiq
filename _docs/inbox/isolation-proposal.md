---
id: inbox-isolation
title: Proposal — isolated runs, custom homes, and per-agent MCP
kind: proposal
status: proposal
summary: Ubiq already links isol8 through the library's isolate module, already confines every terminal pane, and already carries a profile field on the wire that no screen fills — this is the correction, a default isolated run that keeps the user's own home, named homes the user creates and picks, MCP and skill selection riding the profile that is already there, and the four surfaces that draw all of it out of components the kit already ships.
read_when: you are deciding what a pane's $HOME is, what a sandboxed run may reach, or how an agent gets its MCP servers
updated: 2026-09-06
depends_on: [tech-agent-manager, tech-architecture, tech-transport, tech-ui, feat-sessions, feat-workbench, wip-agent-setup, inbox-config]
---

# Proposal — isolated runs, custom homes, and per-agent MCP

**Most of this is already built and switched off.** `crates/agent-manager/src/isolate.rs` wraps isol8 — layer selection, path grants, a macOS Seatbelt policy, managed homes — and `crates/ubiq-host/src/agent.rs` calls it on every terminal spawn. What is missing is not a mechanism. It is three answers Ubiq currently hardcodes: which home a confined run gets, who may choose a different one, and what goes into the run's config directory.

> A confined run's home is a choice with three settings — the user's own, a scratch one, or a named one the user made. Ubiq picks the second and offers no dial.

Three proposals in order of what they cost, each shippable alone.

## 1. Where it stands

`compose_run` in `crates/ubiq-host/src/agent.rs` overrides three fields of the resolved spec: the config directory, the io mode, and isolation. Isolation reads

```rust
spec.isolation = if self.isolate && !structured {
    Isolation::Sandboxed(String::new())
} else {
    Isolation::None
};
```

so a terminal pane is confined when the host's toggle is on and a chat conversation never is. The policy is then planned with `IsolateOptions::new(root/isol8)`, whose constructor documents its own defaults: `HomeMode::Ephemeral`, no extra read-only grants. Nothing in Ubiq can say otherwise, and `Settings::default()` is passed to the resolver, so agent-manager's own `[isolate] home` setting never reaches this path either.

The consequence is the bug users actually meet: an isolated pane gets `$TMPDIR/isol8-home-<pid>-<nanos>`, which has no nvm, no mise, no pyenv, no shell profile, and no `.gitconfig`. The harness itself survives because every harness relocates its config by an env lever — `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `COPILOT_HOME` — and the library seeds credentials into that directory by copy. Everything *around* the harness breaks.

The library already knows this. The comment at `crates/agent-manager/src/harness/claude.rs` records that a per-account `HOME` was tried and reverted for exactly this reason, and `crates/agent-manager/src/isolate.rs` carries a list of well-known runtime roots — mise, nvm, fnm, volta, asdf, bun, pnpm — that the login flow grants read-only from the real home. That list is private to the login path.

## 2. What isol8 does and does not give us

Worth stating plainly, because one of the three asks does not exist upstream.

| Mechanism | isol8 | Where |
|---|---|---|
| Replacement home | Plan of four ops — link, mkdir, seed-ro, copy — idempotent, skip-if-exists, never deleted | `refs/isol8/crates/isol8-core/src/home.rs` |
| Home default | **Unset means the real `$HOME`.** Isolation of paths, not identity | same |
| Policy | TOML layers, deny-first merge, auto-selected by executable basename; kernel-enforced by Seatbelt and Landlock | `refs/isol8/crates/isol8-core/src/profile.rs` |
| MCP injection | **Does not exist.** Zero code; "mcp" appears only as path-grant strings in agent layers | `refs/isol8/profiles/agents/claude-code.toml` |

Two properties matter downstream. First, a home is **never destroyed** — not even a scratch one, which leaks into `$TMPDIR` until the OS reaps it. Retiring a home is the embedder's job. Second, `confined_launch` errors on Linux; isol8's Landlock backend is real but the pty seam it needs is not wired, so everything here is macOS-first with a documented hole.

MCP injection is ours already: the library writes the run's `mcp.json` and passes `--mcp-config … --strict-mcp-config`, and `McpServer`, `McpRef` and the filesystem catalogue in `crates/agent-manager/src/config.rs` are the data model. Nothing needs to be invented for step 3 — only chosen and sent.

## 3. Step one — isolated by default, on the user's own home

The default mode is a confined run whose `$HOME` is the user's home. Paths are restricted by the agent's isol8 layer; identity, toolchain and shell are untouched. This is isol8's own default and the one shape where "on by default" is honest.

`HomeMode` has two variants and needs a third:

```rust
pub enum HomeMode {
    /// The user's own home, unreplaced: isolate paths, not identity.
    #[default]
    Inherit,
    Ephemeral,
    Managed(String),
}
```

`isolate::plan` maps `Inherit` by setting neither `ephemeral_home` nor `home` on the isol8 spec, which is precisely how isol8 spells "keep the real home". `HomeMode::parse` gains the `"inherit"` string. Changing the `Default` derive is the whole of the Ubiq-side change, since `IsolateOptions::new` builds from it — a one-word diff in `crates/agent-manager/src/isolate.rs` and none in `crates/ubiq-host/src/agent.rs`.

The toggle already exists and is already honest about itself: the Harnesses section of application settings draws **Confine agents to their project**, the one row that writes the host layer rather than the interface's own. Step one changes what that checkbox means — confinement without a stolen home — and changes its default to on. Nothing else moves.

**Chat stays unconfined.** `compose_run` excludes structured runs today, and this proposal does not change that: the structured bridge holds a pipe pair the sandbox seam has not been proven against. It is a backlog row, not a hedge — say "terminal panes are isolated" in the UI, not "agents are".

## 4. Step two — named homes

A named home is a directory under `<ubiq root>/isol8/homes/<id>` that survives runs. isol8 materializes it, and the library already addresses it: `HomeMode::Managed(id)` becomes `@managed/<id>` in the spec. What Ubiq lacks is the noun.

A **home** is a record with an id, a display name, and a seed list — the relative paths copied in from the real home on first creation. Where accounts live at `<root>/accounts/<id>`, homes live at `<root>/isol8/homes/<id>`, and the record beside them. Both are references, never material: the seed list names `.gitconfig`, not its contents.

| What | Where | Who writes it |
|---|---|---|
| Home record (id, name, seeds) | `<root>/homes/<id>.json` | Host, on create |
| Home contents | `<root>/isol8/homes/<id>` | isol8, on first run |
| Selection | Session settings | UI, through the message set |

Three messages, shaped like the account family in `crates/ubiq-proto/src/messages.rs`: list homes, create a home, delete a home. Deletion is the one that carries weight — isol8 never deletes, so this is Ubiq's own `remove_dir_all` against a path it composed, and it is the only destructive operation the feature introduces. It asks first.

The seeding question answers itself from isol8's op kinds. A toolchain is a **link** to the real directory, because copying nvm is absurd. A `.gitconfig` is a **seed-ro** copy, because the run should not edit the user's. Credentials are neither — they come from the account, by the library's own seed path into the harness config directory, and a home never holds them. The well-known runtime roots in `crates/agent-manager/src/isolate.rs` become the default seed list for a new home, promoted out of the login path into a public constant.

## 5. Step three — per-agent MCP and skills

**The wire is already built.** `Message::StartConversation` carries `profile: Option<String>`, `ProfileInfo` is a proto type, and `ListProfiles`, `Profiles` and `SaveProfile` are host-implemented against the library's profile store. The interface has never sent anything but `None`, and no screen mentions a profile.

`Profile` in `crates/agent-manager/src/profile.rs` binds account, harness, mcps, skills, model, hooks, instructions and isolation into one named record with an `extends` chain, and `resolve::RunFlags` already resolves against it. So the per-agent MCP question is not a new mechanism, and building a bespoke Ubiq record for it would duplicate one that exists — which is the architecture rule, not a convenience.

Three gaps, all small:

| Gap | Where |
|---|---|
| `ProfileInfo` names account, model and mode, but not mcps, skills or home | `crates/ubiq-proto/src/messages.rs` |
| `SpawnWorkspace` has no `profile` field, so a terminal pane cannot pick one | same |
| No screen sends or edits a profile | `crates/ubiq/src/ui` |

Three optional fields on `ProfileInfo` — `home`, `mcps`, `skills` — and one on `SpawnWorkspace`. Every field stays a reference, which is the rule `ProfileInfo` already documents about itself.

That leaves one open question, which is whether an ad-hoc selection with no saved profile deserves a second field on the message. The answer is no until someone asks twice: a user who ticks three servers for one pane has made a setup, and naming it is cheaper for them than for us.

## 6. The interface

Four surfaces, all drawn from components the kit already ships. Nothing here is a new idiom.

**The defaults do the work.** A user who never opens any of this gets a confined pane on their own home with the profile named `default`, and never learns the words "isol8", "layer" or "seed". Everything below is what customisation looks like *after* that has been true for a while.

### 6.1 An Isolation section in application settings

`SettingsSection` gains an `Isolation` variant beside Harnesses — an enum arm, a label, a nav icon and a `body` arm. **Confine agents to their project** moves here out of Harnesses, where it is the only non-harness row, and gains three companions:

- The default home, as a column of `kit::card` tiles: **Your home** (recommended, the note says paths are restricted and the toolchain is not), **A fresh one each run**, **A named home**. The tile column with a status dot and a note per row is the shape the kitchen sink already draws for default permission mode.
- The named homes list, when the third tile is chosen.
- A one-line readout of what is unavailable on this platform, because a Linux build has to say isolation is off rather than pretend.

### 6.2 The homes manager

The accounts manager is the same object with a different noun, and the proposal is to copy it rather than resemble it: a block per home with a header carrying **Rename** and **Delete** icon actions, a `primary_button` with a plus above the list, a two-line empty state, refusals as a dismissible banner rather than a dialog, and `confirm_modal` with `danger: true` on delete — the one destructive operation the feature introduces, against a directory isol8 will never reclaim on its own.

Inside a block, the seed list is a wrapping chip row with an `×` per chip and a `+` to add — the sink's env-chip idiom, unchanged. A new home starts pre-filled with the well-known runtime roots, so the plug-and-play case is a name and Enter.

### 6.3 The profile editor, which is the MCP picker

`ProfileInfo` becomes a form of the shape the kitchen sink already mocks: account, model, thinking and mode as `Picker` chips, home as a picker, and MCP servers and skills as chip rows over the catalogue. The heading is the one the sink already wrote — applied to every new agent, and any single agent can override it.

There is deliberately no delete on the profile family, and the editor keeps that: a stale setup costs a row in a list.

### 6.4 Choosing, and seeing what was chosen

The new-agent menu is a flat position-indexed list because the kit has no submenus, and it is already grouped `Default` / `Configured`. Profiles are a third group with its own label — the rows and the pick read one list, separators included, so this is a row kind and an arm, not a new menu.

Seeing it afterwards is one chip. The conversation footer already carries a read-only harness·account chip with the whole value in its tooltip; an **isolated** chip sits beside it, faint when confinement is off — the status bar's vim chip is the precedent for a chip that is present either way rather than appearing and shifting its neighbours. Colour comes from the status group, never wording alone, and the pane's two rows of chrome stay two.

The one thing the interface must not do is explain a denial. A confined run that cannot reach a path fails inside the harness, in prose Ubiq did not write, and no chip fixes that. What the chip buys is the user knowing which question to ask.

## 7. What a session has to remember

`SessionMeta` in `crates/agent-manager/src/session.rs` records argv and the config directory, and no environment. A run whose home was `@managed/work` is indistinguishable afterwards from one that inherited. Add the home mode and the profile id to the record — two optional strings — or accept that a resumed session silently changes its own identity. This is small and it is the difference between a reproducible session and a plausible one.

## Phases

1. **Inherit by default.** The third `HomeMode` variant, the `Default` move, and the existing confinement checkbox flipped on. No new nouns, no proto at all. Ships the correction on its own.
2. **The Isolation section.** The nav arm, the moved checkbox, the default-home tiles. Still no new messages — the default home is one field on the host settings the toggle already writes.
3. **Homes.** The record, the three messages, the manager built from the accounts manager, the delete confirmation, the promoted seed list.
4. **Profiles reach the interface.** The three fields on `ProfileInfo`, the `profile` field on `SpawnWorkspace`, the profile editor, and the third group in the new-agent menu. The conversation path needs no proto change at all.
5. **Provenance.** Home mode and profile id in the session record, and the isolated chip in the footer.

## What this costs

macOS only, and visibly so. isol8's Landlock backend exists but its confined launch errors out, and its `literal` and `prefix` grant kinds degrade to whole-subtree grants there — a layer like the ssh integration, which denies `~/.ssh` and re-opens four files, does not transfer. A Linux build must present isolation as unavailable rather than as working.

Homes never shrink. Every named home is a directory the user accumulates, and the delete path is ours to get right. Scratch homes already leak; step one makes them rare rather than fixing them, and a sweep belongs with the run-directory sweep that already exists.

And the failure mode of confinement is a denial that looks like a bug in the agent. A pane that cannot reach a path fails somewhere inside the harness, in prose the user did not write. Without the toggle from step one this feature generates more reports than it prevents.

## What this asks to be decided

- Whether isolation-on-by-default is per session or per project. Per session is proposed; per project is one field away.
- Whether chat conversations stay unconfined indefinitely, or the structured bridge gets proven against the sandbox seam.
- Whether a named home may be shared by two panes at once. isol8 has no lock; concurrent harnesses in one home is the same hazard as two agents in one working tree.
- Whether the ad-hoc MCP selection, without a saved profile, ever earns its field.

The Linux story, the scratch-home sweep, and the structured-run confinement are backlog rows this proposal opens rather than closes.

## Related docs

- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary this proposal pushes against; it owns the rule that Ubiq never names a harness config path.
- [`../tech/architecture.md`](../tech/architecture.md) — why a home choice has to arrive as a message rather than a shared handle.
- [`../tech/transport-contract.md`](../tech/transport-contract.md) — where the home and profile fields land.
- [`../features/sessions-and-workspaces.md`](../features/sessions-and-workspaces.md) — the surface that owns the isolation toggle.
- [`../tech/ui-and-design.md`](../tech/ui-and-design.md) — the token rule, the page-overlay shape, and the one-menu-at-a-time rule every surface here obeys.
- [`../features/workbench.md`](../features/workbench.md) — the settings page and the accounts manager this copies.
- [`../wip/agent-setup.md`](../wip/agent-setup.md) — what the library cannot yet do behind a composed harness.
- [`./config-persistence-proposal.md`](./config-persistence-proposal.md) — where a home record would be written and read.
