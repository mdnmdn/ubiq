# Harness Multiplexer — Architecture (Simple Solution)

## Purpose

A tmux-like application for hosting and orchestrating multiple interactive agent **harnesses** (Claude Code, Gemini CLI, Codex, opencode, etc.) side by side. Each harness is a full-screen terminal UI in its own right, so the application must host real terminal panes — not plain text output.

This document describes the **simple, first-iteration solution**. It deliberately defers the heavier architectural commitments (true multi-process, distributed harnesses) while making sure today's design does not foreclose them.

---

## Scope of this iteration

- **UI:** Tauri shell + `xterm.js`, one terminal instance per pane.
- **Coordinator:** owns the harness processes and their PTYs, routes bytes to/from the UI.
- **Topology:** coordinator and UI conceptually separate, but communicating over **a single channel** that runs **in-process now** and is built so it can be **serialized and split later** with no change to either side's logic.

Explicitly out of scope for now: running the coordinator as a standalone daemon, and running harnesses on remote hosts/containers. Both are anticipated below under Next Steps so we don't make choices we'll regret.

---

## Key design fact

Each harness is a full-screen, interactive TUI: it uses the alternate screen, addresses the cursor absolutely, emits the full ANSI/VT escape vocabulary, queries terminal size, redraws on resize, and expects raw-mode keystrokes routed back byte-for-byte (arrows, Ctrl/Alt, bracketed paste, mouse).

Consequence: **a pane is a terminal, not a text buffer.** We need a real terminal emulator per pane and a real PTY per harness. The chosen stack picks components that already solve both halves:

- **`xterm.js`** is a complete terminal emulator designed to be fed a byte stream from elsewhere and to ship keystrokes back — exactly our pane's job.
- **`portable-pty`** gives the coordinator cross-platform PTY bindings to spawn harnesses in a real TTY.

This means we write **no terminal-state engine and no VT parser ourselves**; we shuttle bytes between the two.

---

## Components

### 1. Coordinator (Rust)

Owns everything process-related:

- Spawns each harness in its own PTY via `portable-pty`.
- Reads each PTY's output stream.
- Writes keystrokes to each PTY's input.
- Tracks per-pane state: pane ID, process handle, lifecycle status, window size.
- Handles resize by propagating new geometry to the PTY (`TIOCSWINSZ`).
- Exposes everything to the UI through **one transport contract** (below).

The coordinator never renders anything. It is a process supervisor plus an I/O router.

### 2. UI (Tauri + xterm.js)

A pure **stream-attach client**:

- One `xterm.js` instance per pane.
- Feeds incoming PTY-output bytes (for that pane) straight into its `xterm.js`.
- Sends keystrokes from the focused pane's `xterm.js` back upstream.
- Sends resize / spawn / focus as control messages.
- Handles layout, focus, and pane chrome.

The UI never touches a PTY or a process. It only knows about streams and pane IDs.

### 3. The transport contract (the load-bearing decision)

Because the UI is a stream-attach client, the boundary between UI and coordinator is **a stream of bytes plus a few control messages**, not a function call. This contract is the one piece that is expensive to change later, so we define it now and keep both sides strictly behind it.

**Downstream (coordinator → UI)**
- `output{ pane_id, bytes }` — framed PTY output, tagged by pane.
- `exited{ pane_id, code }` — a harness ended.

**Upstream (UI → coordinator)**
- `input{ pane_id, bytes }` — framed keystrokes, tagged by pane.

**Control (bidirectional)**
- `spawn{ pane_id, harness, args }`
- `resize{ pane_id, cols, rows }`
- `focus{ pane_id }`

Everything is tagged by `pane_id` and framed so message boundaries are unambiguous. Bytes for terminal I/O stay opaque (we do not parse them); only control messages are structured.

---

## Topology for this iteration: one process, one channel

To keep the first version simple, the coordinator and UI run **in the same process**, and the transport contract is implemented over **a single in-memory channel** (e.g. an async channel carrying the message enum above).

```
  ┌──────────────────────── Single process ────────────────────────┐
  │                                                                 │
  │   ┌──────────────┐      one channel       ┌─────────────────┐   │
  │   │ Coordinator  │  ◄── contract msgs ──►  │ UI (Tauri +     │   │
  │   │              │                         │ xterm.js panes) │   │
  │   │ portable-pty │                         │                 │   │
  │   └──┬───┬───┬───┘                         └─────────────────┘   │
  │      │   │   │                                                   │
  │   ┌──▼─┐┌─▼──┐┌─▼──┐                                             │
  │   │PTY ││PTY ││PTY │   (one harness each)                       │
  │   │ A  ││ B  ││ C  │                                             │
  │   └────┘└────┘└────┘                                             │
  └─────────────────────────────────────────────────────────────────┘
```

The critical discipline: **all communication flows through the contract, even though it's in-process.** No side reaches around the channel to call the other directly. That single rule is what makes the later split cheap.

---

## Why this is the right "simple" shape

- **xterm.js already is the terminal emulator** we'd otherwise have to integrate, and it's built for "remote stream in, keystrokes out" — our exact interface.
- **In-process + single channel** is the least code: no socket, no serialization, no daemon lifecycle to manage yet.
- **The contract is defined now**, so the simplicity is free of future regret — the same message types serialize later without touching coordinator or UI logic.

---

## Build order

1. **One PTY, one pane, one real harness.** Spawn a harness (e.g. Claude Code) in a PTY, bind it to a single `xterm.js` pane through the channel. Confirm full-screen redraw, color, resize (`SIGWINCH` → `TIOCSWINSZ`), and keystroke round-tripping. This single test concentrates essentially all the project's risk.
2. **Resize correctness.** Make pane geometry propagate accurately to the PTY window size; get this right early, as it's a common source of corrupted layouts.
3. **N panes.** Generalize to multiple harnesses with pane-ID tagging on every message.
4. **Focus + layout.** Route keystrokes to the focused pane only; add pane chrome and splitting.
5. **Lifecycle.** Handle harness exit/crash, surface `exited`, decide restart behavior.

---

## Next steps — anticipated, deliberately not decided now

These are flagged so today's simple design stays compatible with them. We are **not** choosing among them now; we are only making sure the contract above survives each transition unchanged.

### Next step 1 — Separate processes

Split coordinator and UI into two OS processes. The single in-memory channel becomes the **same contract serialized over a local transport** (e.g. a Unix domain socket / named pipe). Because both sides already speak only the contract, the change is confined to the channel implementation: add framing + (de)serialization, swap the in-memory channel for the socket. Coordinator and UI logic are untouched. This also unlocks tmux-style **detach/reattach** (UI can die and reconnect while the coordinator keeps harnesses alive) — which is why the in-process channel is built as a predisposition to detachment from day one.

### Next step 2 — Distributed harnesses

Run individual harnesses on **separate hosts or containers** for a ubiquitous, location-independent setup. This is structurally the same problem as a terminal stream crossing a machine boundary — the coordinator stops assuming the PTY is local, and the per-pane stream now arrives from a remote agent over a network transport (TCP/QUIC/WebSocket). Again the **contract is unchanged**: a pane is still a tagged bidirectional byte stream plus control messages; only its origin moves off-box.

### Design rule that protects both

> The UI must never assume the PTY is local, and neither side may bypass the contract. Keep every interaction expressed as contract messages tagged by `pane_id`.

Honoring this one rule is what lets us go **in-process → two processes → distributed** by only ever changing the transport beneath the contract, never the coordinator or the UI.

---

## Summary

| Concern | This iteration | Predisposed for later |
|---|---|---|
| UI | Tauri + xterm.js, one instance per pane | unchanged |
| Terminal emulation | handled entirely by xterm.js | unchanged |
| PTYs / processes | coordinator via `portable-pty` | move to remote hosts/containers |
| Coordinator ↔ UI | single in-memory channel | serialize over socket, then network |
| Topology | one process | two processes, then distributed |
| Contract | defined now, byte streams + control msgs, tagged by `pane_id` | **identical across all stages** |

The simple solution ships fast because xterm.js absorbs the terminal-emulation problem and a single in-process channel absorbs the transport problem — while the explicitly-defined contract ensures neither the process split nor the distributed step will force a rewrite.
