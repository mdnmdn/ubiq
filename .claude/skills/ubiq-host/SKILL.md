---
name: ubiq-host
description: Reference for Ubiq's headless host (crates/ubiq-host) — the coordinator run loop, pseudo-terminals, the project catalogue, the four stores, the worker threads and the filesystem watch. Use when changing how a pane is spawned, resized, killed or reaped, how a project is added, located or forgotten, how anything is written to the config root, or when adding a message arm the coordinator has to answer.
---

# The headless host

`crates/ubiq-host` is everything that is not drawing. It spawns each harness under a
pseudo-terminal, reads that terminal's output, writes keystrokes into it, propagates geometry,
tracks each pane's lifecycle and reports exits. It is a process supervisor plus an I/O router.

Everything it says leaves as a `ubiq_proto::messages::Message` addressed to a client. It has no
window, no palette and no layout, and `just host` checks that mechanically — no `gpui` crate may
appear in its dependency tree.

```
one process
  ubiq-app boot ──► coordinator::start(host, root, projects, work, settings, pending)
                       │  one thread, "ubiq-coordinator", started before the first window
                       ├─ HostEnd ◄──► bus ◄──► every window's Client
                       ├─ HashMap<PaneId, Pty>          the panes
                       ├─ Files / Git / Search / Index  four worker threads
                       └─ Projects / Work / Settings    the stores under the config root
```

## Read first

| You are | Read |
|---|---|
| Adding anything that crosses the UI/host line | `_docs/tech/architecture.md`, then `_docs/tech/transport-contract.md` |
| Changing pane spawn, focus, resize, close or reap | `_docs/features/panes-and-terminals.md` |
| Changing a workspace's composition or lifecycle | `_docs/features/sessions-and-workspaces.md` |
| Touching a store, the config root or a file's on-disk shape | `_docs/tech/project-structure.md` (`## The config root`) |
| Changing `stats()` or the usage meter | `_docs/features/stats.md` |
| Adding a log line, or asking why one went missing | `_docs/features/logs.md` |
| Arguing with a rule below | `_docs/tech/decisions.md` |

Your change updates the documents it touched, in the same commit. `just docs-touched` names them.

## The rules

1. **The host never reaches around the bus.** `crates/ubiq` is not in its dependency graph and it is
   not in `crates/ubiq`'s. Every fact crossing is a variant in `crates/ubiq-proto/src/messages.rs`.
   `just host` and `just ui` are the two halves of that check (`D3`, `D27`).
2. **The coordinator renders nothing.** No layout, no colour, no opinion about what the bytes mean.
   Terminal *emulation* is the UI's (`D2`).
3. **Terminal bytes stay opaque.** No VT parser, no terminal state engine here. The one read of a
   forwarded stream is `links::LinkScanner` on a login pane — a crude byte search for a URL, after
   the `TerminalOutput` carrying those bytes has already been queued, never in place of it (`D6`).
4. **Every message carries a pane ID**, including in the single-pane case (`D5`).
5. **The reader is never blocked by a slow UI.** `Pty::forward_output` runs on its own thread into
   an unbounded mailbox. A stalled reader stalls the harness (`D21`).
6. **A resize is incomplete until the harness knows.** `Pty::resize` sets the pseudo-terminal size
   so the kernel signals the process. Visual-only resize is the classic corruption bug.
7. **An exited harness closes its pane.** `pty::reap` waits on the child and sends `PaneExited`
   once; the UI answers with `CloseWorkspace`. Closing a tab is still what kills a harness that has
   not already ended (`D22`).
8. **One host per process**, started by the binary before the first window and outliving all of
   them — two hosts would race the catalogue file. Nothing drops when a window closes, so
   `client_gone` reaps that window's pseudo-terminals deliberately (`D28`).
9. **Ubiq writes nothing inside a project's folder** except the workarea it reserves and never
   reads (`D30`, architecture rule 6).
10. **The host names no harness config path and hard-codes no launch.** `agent-manager` answers
    all of it; `agent.rs` overrides exactly three fields of what `resolve` returns.

## The run loop

`Coordinator::run` in `coordinator.rs` is the whole of it, and it is single-threaded:

```
loop {
    wait  = projects.next_due(now), capped at CONVERSATION_POLL (500ms)
            whenever a conversation is live or a clone is running
    event = host.recv_timeout(wait)     // Disconnected ⇒ break
    match event { Connected → client_here, Said → dispatch, Gone → client_gone, None → () }
    reap_conversations();  register_clones();  projects.flush_due(now)
}
projects.flush()    // nobody left to tell, but what the user last did belongs on disk
```

The bounded wait exists because two things finish without saying anything: a conversation whose
harness ended sets a flag rather than sending, and a preference debounce (`projects::DEBOUNCE`,
400ms) has to be written even if nobody speaks again.

`dispatch` is one giant `match` over `Message`, grouped by family, and its last arm **names** the
message rather than dropping it — a response-direction variant arriving here is a wiring mistake,
and `tracing::warn!` is how you find it.

## Gotchas that actually bite

- **Anything slow on the coordinator's thread stalls every pane's keystrokes.** That is why
  `Files`, `Git`, `Search` and `Index` are `start()`ed worker threads, why a clone gets a thread of
  its own (`D73`), and why model discovery and `suggest_job` spawn one-off threads. The spawn path
  is the known exception — it probes a folder inline (`G38`), as do the three file-backed stores
  (`G46`).
- **A worker thread cannot borrow `&self`.** That is why `Arc<FileHarnessCache>`, `Arc<Usage>` and
  `Arc<Settings>` exist, and why `probe_catalogue` is a free function rather than a method.
- **`owns(client, pane_id)` gates every pane message.** A pane belongs to the window that spawned
  it; `owners` is the entire routing table. A message about somebody else's pane is warned about
  and dropped, and one about a pane already gone is dropped silently.
- **The owner is inserted before the pane is announced.** Nothing may arrive about a pane the
  routing table has not heard of — and if `forward_output` fails, the owner is removed, the child
  killed and waited on, and only a `PaneError` goes out.
- **A refused spawn names the project, not the pane.** `refuse_spawn` sends `ProjectError` because
  no pane exists yet; `refuse_pane` sends `PaneError` for a pane the UI was never told about, so
  there is nothing on screen to close either way.
- **Drop the slave.** `pty::spawn` drops `pair.slave` after starting the child; holding it keeps
  the pseudo-terminal open forever and the reader never sees end-of-stream.
- **A shell must be a login shell** — `command_for` uses `CommandBuilder::new_default_prog` plus
  `SHELL`, because `portable-pty` prefixes argv0 with `-` only for that builder. Without it a
  Finder-launched Ubiq gives every pane a thin `PATH` and installed tools read as `command not
  found` (`D49`, `D62`).
- **Panes start at `INITIAL_COLS`×`INITIAL_ROWS` (80×24)** and are told the truth a frame later by
  `TerminalResize`. The harness must exist before the emulator has bounds to measure.
- **`gc::collect` runs only after a load that succeeded.** Sweeping against the empty catalogue a
  *corrupt* file produces would delete every project's view state — the exact thing preserving the
  file exists to avoid. `Projects::loaded` and `sweep_ephemeral` carry the same guard.
- **A too-new file is not corruption.** `StoreError::UnknownVersion` leaves the file untouched;
  `Work` marks that project `sealed` and refuses to write it. Only a *parse* failure moves a file
  aside via `atomic::preserve_aside`.
- **Watches are keyed `(ClientId, ProjectId)`.** There is no `CloseProject` message, so opening the
  next project is the only signal the old watch is unwanted — `watch_project` retains by that rule,
  and dropping the `Watcher` is the whole of stopping one.
- **The watch feeds the index directly**, not through the coordinator: `watch::Job.index` carries
  the index sender, so a flush reaches both without the coordinator seeing a change it would have
  to relay.

## Recipes

**Add a message the coordinator answers.** Variant in `crates/ubiq-proto/src/messages.rs` → an arm
in `dispatch`, placed in its family's block → if it touches disk or a network, a `*_job` helper that
hands the work to a worker or a one-off thread with a `Mailbox`, never inline → update
`_docs/tech/transport-contract.md` in the same commit.

**Answer from a service.** A service returns `Reply::Asker` or `Reply::Everyone` and never holds the
bus; `Coordinator::answer` turns that into a `To`. Broadcast is for the catalogue, which every
window shows; a listing or a preference is the asker's. This is what keeps `Projects` and `Work`
testable with no bus.

**Add a store.** Four traits in `store/mod.rs` (`ProjectStore`, `TaskStore`, `PreferenceStore`,
`SettingsStore`), a file impl in `store/file.rs` and a memory impl in `store/memory.rs` — the memory
stores are not test-only, they are the fallback for an unwritable config root, and carry
`fail_writes`/`fail_load` so the failure paths are exercised. A store with exactly one
implementation is a concrete type instead (`store/harness.rs`, `store/usage.rs`).

**Write a file.** `atomic::write_atomic` (creates parents) or `write_atomic_with` (keeps the mode,
creates nothing — the rule inside a *user's* project). Never `fs::write` for anything the user
would miss.

**Add a worker thread.** Follow `watch/`: a `Job` struct carrying the addressed `Mailbox`, a
`start(job)` returning a handle, and the worker ending when that handle is dropped.

## Reference files

- [`reference/module-map.md`](reference/module-map.md) — every module in scope, its public surface,
  its constants and what it must never hold.
- [`reference/runtime.md`](reference/runtime.md) — threads, the pane and client lifecycles, the
  store durability rules, the config root's shape, and the teardown order.

## Verifying

`just verify` (= `check clippy test host ui docs-lint`) is what a change has to pass. `just host`
alone builds the crate and asserts no `gpui` crate is in its tree; `just fmt` before committing.
Host tests are integration tests in `crates/ubiq-host/tests/` — `coordinator.rs`, `projects.rs`,
`preferences.rs`, `settings.rs`, `work.rs`, `watch.rs`, `usage.rs`, `config_root.rs` are the ones in
this skill's scope, and they drive real types against temp directories rather than mocking the bus.
