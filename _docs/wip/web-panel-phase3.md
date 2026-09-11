---
id: wip-web-panel-phase3
title: Web panels — the shared workarea and the bundle fetch
kind: wip
status: current
summary: Phase 3 of the web-panel proposal as built, Rust side only — a `shared_workarea` on `HostInfo` reserved at `<config root>/ui/`, a four-variant web asset message family, and `crates/ubiq-host/src/web_assets/` fetching a manifest's 554 files six at a time, verifying each against its SHA-256 before writing it, and marking the directory complete only after the last one. No interface wiring, which is a later unit.
read_when: you are fetching or serving a vendor bundle, changing the shared workarea, or wiring the interface to `EnsureWebBundle`
updated: 2026-09-11
verified: 2026-09-11
code_anchors: [crates/ubiq-host/src/web_assets/mod.rs, crates/ubiq-host/src/web_assets/manifest.rs, crates/ubiq-host/src/projects.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-proto/src/messages.rs]
depends_on: [tech-architecture, tech-transport, wip-web-panel-phase2]
---

# Web panels — the shared workarea and the bundle fetch

Phase 3 of [`../inbox/web-panel-proposal.md`](../inbox/web-panel-proposal.md), and the half of it
that is Rust: where a downloaded vendor bundle lives, and who downloads it. §5 left which side
fetches as its one open question and then answered it — *"the interface asks; the host fetches,
verifies, unpacks into the shared workarea and answers when it is there"* — and that is what is
built. Nothing here draws, and nothing in `crates/ubiq` was touched: the interface wiring, the
`vendor/` route and the chrome that mounts the component are a later unit.

## The shared workarea

`Message::HostInfo` gains `shared_workarea: Option<String>`, reserved once when the coordinator
starts and told to every window as it attaches. It is `<config root>/ui/` — the exact mirror of a
project's `projects/<id>/ui/`, one level up — and carries the same four rules: told rather than
composed, made by the host before it is named, disposable, and not the project's folder.

What makes it *shared* is that it belongs to the host rather than to any project, which is the whole
reason it exists: the same 25 MiB bundle wanted by five projects is one copy.

`projects::shared_workarea` and `projects::reserve_shared_workarea` are free functions beside
`Projects::workarea` and `Projects::reserve_workarea`, and keep that method's posture exactly — a
`create_dir_all` whose failure is a warning and never an error, because a directory that will not be
made is still named and an interface that cannot cache is one that redraws.

This widens `architecture.md` rule 6 by one directory, and by one qualification: the host **does**
write inside the shared workarea, through `web_assets` and only there, filling a directory the
interface asked it to fill. It still reads nothing in either workarea.

## The message family

Four variants, in `crates/ubiq-proto/src/messages.rs` after the notification family;
[`../tech/transport-contract.md`](../tech/transport-contract.md) owns them.

| Message | Direction | Payload |
|---|---|---|
| `EnsureWebBundle` | UI → host | `app` |
| `WebBundlePending` | host → UI | `app`, `done`, `total`, `file` |
| `WebBundleReady` | host → UI | `app`, `version`, `path` |
| `WebBundleFailed` | host → UI | `app`, `error` |

The interface never names a version: the pinned one and the manifest are the host's, and
`WebBundleReady` says which it got. There is no cancel — the ask is idempotent and re-entrant, so a
second `EnsureWebBundle` for an app in flight joins the running fetch instead of starting a second,
and one for a finished bundle is answered from a single `stat`.

`total` is known before the first byte, so the fetch sends a `WebBundlePending` with `done` at zero
and `file` empty before touching the network, and every report after names the cache path of the
file that last landed — what turns the interface's bar into a real fraction from the start rather
than a sweep.

## On disk

```
<shared workarea>/web/<app>/<version>/<entry.path>   the mirror; cache path = URL path exactly
<shared workarea>/web/<app>/<version>/importmap.json the map, at a fixed name the chrome knows
<shared workarea>/web/<app>/<version>/.complete      written last, and the only proof of done
```

`<version>` is `manifest::VERSION`, pinned in generated source. A build that pins a new one looks
in a directory that does not exist, fetches, and leaves the old one alone — versioned by directory,
with no TTL and no staleness question to ask.

**The marker is the whole completeness test.** A process killed mid-fetch leaves a directory of
real, verified files and no marker, and that must not read as done. The marker is written after the
last entry is verified and never before, so the only two states a reader sees are "not finished" and
"finished".

`.complete` is dotted so the interface's own server, which refuses dotfiles, can never serve it.

## The fetch

`crates/ubiq-host/src/web_assets/mod.rs`, on the clone family's shape: a `Job` that owns everything
by value, a thread of its own, progress straight to the asker's `Mailbox`, and a channel back to the
coordinator for the one thing the thread cannot do.

- **Six connections, one queue.** `PARALLEL` workers pull from a `flume` channel of manifest
  entries. 554 serial round trips is minutes of latency; 554 threads is an attack on a CDN.
- **A file already on disk with the right length and hash is skipped**, so an interrupted fetch
  resumes rather than starting again.
- **Verify, then write.** The bytes are hashed before anything touches disk, so a file whose SHA-256
  is not the manifest's is discarded rather than cached, and fails the whole fetch with an error
  naming it. The length is checked too, but the hash is the test — the interesting failure is right
  length, wrong bytes.
- **`atomic::write_atomic`** does the writing: temp sibling, fsync, rename. Reused rather than
  reimplemented from `state/diagrams.rs`.
- **`WebBundlePending` is throttled** to 250ms, `clone.rs`'s constant for `clone.rs`'s reason. Each
  file landing is also a `tracing::debug!` line, for the run that needs more than the throttled
  reports.
- **Failure is a downgrade**, never a panic and never a half-cached bundle.
- **A thread that fails to start still answers.** If the fetch's own thread cannot be spawned, every
  window waiting on that bundle is sent `WebBundleFailed` rather than only logged — a loader that
  never hears back is a panel that loads forever.

The coordinator holds `WebAssets`: one row per fetch in flight, naming the windows waiting on it
under a `Mutex` the worker thread shares, which is how a late joiner is told and how a departed
window stops being told. A row is reaped where the next fetch is started, on `Repos::clone`'s rule,
and the worker posts its own end *before* it answers, so a window asking at that exact moment starts
a fresh fetch that finds a finished directory rather than joining a thread that is about to end.

## Testing it

`Fetch = fn(&str) -> Result<Vec<u8>, String>` is the one seam, and `WebAssets::with` takes it along
with the bundles to offer. Everything else — the hashing, the marker, the resume, the join, the
throttle — is the real code in the tests as in a real run, and no test reaches a network. The six
in `web_assets::tests` cover a complete directory answered without touching the network, a hash
mismatch discarding the file and failing, progress carrying the count up front and then the file, a
directory with files and no marker that is not complete and then resumes, two asks that become one
fetch, and an app with no manifest.
`a_window_is_told_a_shared_workarea_the_host_reserves_and_leaves_alone` in
`tests/coordinator.rs` mirrors the per-project workarea test over the bus.

## Next steps

- Wire the interface: `EnsureWebBundle` on opening an Edit panel, and the `vendor/` route serving
  the answered `path` with the MIME rules §6 makes a hard failure.
- Decide whether the shared workarea is ever swept — `G66` asks the same of the per-project one.
- `_tools/webassets.py` should emit `#[rustfmt::skip]` over `FILES`: now that `manifest.rs` is in
  the module tree, `cargo fmt` rewrites the generated one-line entries into 3,300 lines.
