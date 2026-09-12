---
id: wip-web-panel-phase3
title: Web panels — the shared workarea and the bundle fetch
kind: wip
status: current
summary: Phase 3 of the web-panel proposal as built, Rust side only — a `shared_workarea` on `HostInfo` reserved at `<config root>/ui/`, a four-variant web asset message family, and `crates/ubiq-host/src/web_assets/` fetching a manifest's 554 files six at a time, verifying each against its SHA-256, and writing them into one compressed `.bundle` file whose rename over a `.part` sibling is the only proof of done. No interface wiring, which is a later unit.
read_when: you are fetching or serving a vendor bundle, changing the shared workarea, or wiring the interface to `EnsureWebBundle`
updated: 2026-09-12
verified: 2026-09-12
code_anchors: [crates/ubiq-host/src/web_assets/mod.rs, crates/ubiq-host/src/web_assets/archive.rs, crates/ubiq-host/src/web_assets/manifest.rs, crates/ubiq-host/src/projects.rs, crates/ubiq-host/src/coordinator.rs, crates/ubiq-proto/src/messages.rs]
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
<shared workarea>/web/<app>/<version>.bundle        one file: every entry, deflated, then an
                                                     index and a footer — see `archive.rs`
<shared workarea>/web/<app>/<version>.bundle.part   where a fetch in progress writes
```

A vendor bundle is one compressed file, `crates/ubiq-host/src/web_assets/archive.rs`'s container,
rather than a directory of hundreds of small files: each entry's body is raw deflate at
`Compression::fast()` — deliberately light, since these bytes are read back over loopback once and
never shipped anywhere — and the file ends with a JSON index of `["path", offset, compressed_len,
uncompressed_len]` tuples, a little-endian `u64` index offset, then the 8-byte magic `UBIQBND1`.
The import map lands inside the archive under a fixed name (`web_assets::IMPORT_MAP_FILE`), not as
a sibling file.

`<version>` is `manifest::VERSION`, pinned in generated source. A build that pins a new one looks
for a file that does not exist, fetches, and leaves the old one alone — versioned by file, with no
TTL and no staleness question to ask.

**The rename is the whole completeness test, and there is no resume.** Every entry lands in the
sibling `.part` file as it is verified; only once the import map is appended and the footer written
is `.part` renamed over the real name, so the only two states a reader sees are "no `.bundle` file"
and "a finished one". A process killed mid-fetch leaves a `.part` file and no `.bundle` — its
content is never trusted, and the next ask overwrites it and fetches every entry again. This is why
the `.complete` marker file is gone: the rename replaces it as the single proof of done, and a
`.part` file is what a marker file used to be needed to disprove.

## The fetch

`crates/ubiq-host/src/web_assets/mod.rs`, on the clone family's shape: a `Job` that owns everything
by value, a thread of its own, progress straight to the asker's `Mailbox`, and a channel back to the
coordinator for the one thing the thread cannot do.

- **Six connections, one queue.** `PARALLEL` workers pull from a `flume` channel of manifest
  entries. 554 serial round trips is minutes of latency; 554 threads is an attack on a CDN.
- **Verify, then write.** The bytes are hashed before anything touches disk, so a file whose SHA-256
  is not the manifest's is discarded rather than cached, and fails the whole fetch with an error
  naming it. The length is checked too, but the hash is the test — the interesting failure is right
  length, wrong bytes. There is no resume: a killed fetch's `.part` file is never read back, only
  overwritten.
- **A bounded channel, not `atomic::write_atomic`, does the writing.** The `PARALLEL` workers only
  fetch and verify; a verified body travels over a bounded `flume` channel to the single thread that
  owns the `archive::Writer` and the `.part` file, so 554 bodies never race a write. That thread
  renames `.part` onto the real name once the writer's `finish()` returns.
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
a fresh fetch that finds a finished `.bundle` rather than joining a thread that is about to end.

## Testing it

`Fetch = fn(&str) -> Result<Vec<u8>, String>` is the one seam, and `WebAssets::with` takes it along
with the bundles to offer. Everything else — the hashing, the rename, the join, the throttle — is
the real code in the tests as in a real run, and no test reaches a network. The six in
`web_assets::tests` cover a complete `.bundle` answered without touching the network, a hash
mismatch discarding the file and failing, progress carrying the count up front and then the file, a
leftover `.part` file from a killed fetch that is not complete and is overwritten rather than
resumed, two asks that become one fetch, and an app with no manifest.
`a_window_is_told_a_shared_workarea_the_host_reserves_and_leaves_alone` in
`tests/coordinator.rs` mirrors the per-project workarea test over the bus.

## Next steps

- Wire the interface: `EnsureWebBundle` on opening an Edit panel, and the `vendor/` route serving
  the answered `path` with the MIME rules §6 makes a hard failure.
- Decide whether the shared workarea is ever swept — `G66` asks the same of the per-project one.
- `_tools/webassets.py` should emit `#[rustfmt::skip]` over `FILES`: now that `manifest.rs` is in
  the module tree, `cargo fmt` rewrites the generated one-line entries into 3,300 lines.
