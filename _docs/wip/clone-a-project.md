---
id: wip-clone
title: Cloning a project
kind: wip
status: current
summary: How a repository becomes a project — a connection's listing or a pasted URL, a branch, a destination, and the throwaway clone that is deleted when it closes. The clone half is built and covered by tests; the named OAuth registrations the connect flow picks from are built and never exercised against a live provider, which is the gap this document exists to record.
read_when: you are changing how a repository is cloned, what an ephemeral project is, or how a connection chooses the application it authenticates as
updated: 2026-09-10
verified: 2026-09-10
code_anchors: [crates/ubiq-proto/src/repos.rs, crates/ubiq-proto/src/connectors.rs, crates/ubiq-proto/src/projects.rs, crates/ubiq-host/src/repos/mod.rs, crates/ubiq-host/src/repos/list.rs, crates/ubiq-host/src/repos/clone.rs, crates/ubiq-host/src/projects.rs, crates/ubiq-host/src/connectors/app.rs, crates/ubiq/src/state/clone.rs, crates/ubiq/src/app/clone.rs, crates/ubiq/src/ui/clone.rs, crates/ubiq/src/ui/kit/menu.rs]
depends_on: [feat-workbench, feat-connectors, tech-transport, tech-version-control]
---

# Cloning a project

Before this, the only way a project entered the catalogue was to point the folder dialog at a folder
that existed. Connectors shipped an authenticated identity at GitHub, GitLab, Gitea, Azure DevOps,
Atlassian and Google, and nothing consumed one. This is what consumes one.

The settled parts live in [`features/workbench.md`](../features/workbench.md) and
[`tech/transport-contract.md`](../tech/transport-contract.md); this document is the working
record — what is built, what was decided along the way, and what has not been run against a real
provider.

## What is built

**The clone surface.** A "Clone a project…" row in the project selector opens a modal. With a
connection it lists that identity's repositories, filters them, picks a branch, chooses a
destination folder and a name, and clones. A public repository URL can be pasted instead, with no
connection at all. Pasting a repository URL into the omni search and pressing Enter opens the same
modal on that URL — a repository URL is an answer rather than a filter, so the list collapses to one
row, the same short-circuit a pasted `ubiq://` link takes.

**Two sources, one modal.** `RepoSource` is either a connection lending its token or a bare URL
cloned anonymously. The two sit side by side rather than behind a wizard step, because either is
answerable at any moment; the half not chosen is drawn dimmed rather than hidden. With no connection
configured, the listing and its filter are absent entirely and the URL field is the whole of the
modal — a filter over a list nothing can fill is a control that can only disappoint.

**Finding a repository.** The host pages the provider's own membership listing to a fixed ceiling
and reports whether it stopped there. The interface filters that list in memory. Only a filter that
comes up empty *against a truncated listing* asks the provider to search, so typing costs no
requests in the common case and still reaches a repository past the ceiling.

**Finding a branch.** For a connection, the provider's branch API. For a bare URL there is no API to
call, so the host reads the refs anonymously through libgit2 — `Remote::create_detached`, `connect`,
`list`. The same call yields the default branch, as a full `refs/heads/…` that has to be stripped.

**Ephemeral clones.** A checkbox puts the clone under Ubiq's ephemeral root, shallow by default, and
marks the record `temporary`: the case is a repository read once and thrown away — a pull request
looked at, a dependency checked before it is trusted. Closing one raises the inline confirmation a
project with unsaved work raises, saying the clone will be discarded. Confirming sends
`ForgetProject`, and the host deletes the folder. A window closed without going through that path
leaves the folder for the startup sweep.

**Named application registrations.** Settings gains `+ App registration…` beside `+ Connect…`: a
named record of an OAuth application, its provider, its base URL, its client id and — through the
secret store — its client secret. The form shows the callback URL read-only with a copy button,
which is the value a user cannot guess and the reason the surface exists. The connect flow picks a
registration instead of typing an instance URL, offering `Default` only where this build carries a
bundled client id for that provider, and `Another instance…` for a token flow that needs no
application at all.

**Two settings**, under File explorer: the default project folder and the ephemeral folder. Empty
means the host's own default, shown as placeholder rather than a path the interface invented.

## The decisions that shaped it

**`git2` gains `https`** (`D72`). It was compiled with no transport at all, because Ubiq only read
repositories. A clone is the one operation that creates a repository, and everything after it is
still read-only. Verified before it was designed in: an anonymous ref listing, a shallow clone of a
named branch, and `is_shallow` true on the result.

**A clone never runs on the git worker** (`D73`). `git/mod.rs` is one shared thread whose `answer()`
is synchronous, so a clone of minutes would head-of-line block branch names and badges for every
project in every window. A clone takes its own thread instead, copying the connect flow's shape: an
id in every message, the asker's mailbox for progress, a `flume` receiver as the sole cancel
mechanism. Progress is throttled to one message every 250ms, because the transfer callback fires per
object and would otherwise flood the bus.

**Deleting needs the flag *and* the path** (`D74`). `temporary` is set for a folder dragged onto the
window from anywhere on the disk, so the flag alone cannot authorise a removal. The ephemeral root
is a setting a user could point at their home directory, so the path alone cannot either. Both
conditions hold, the path is canonicalised before the prefix test, and the tests for this are the
most important tests in the change.

**Whether a project is ephemeral is told, not derived.** `ProjectSnapshot::ephemeral` carries it,
beside `health` and `open_panes`. An interface that worked it out from a path prefix would answer
`false` for everyone running the default settings — the unset ephemeral root resolves to a default
only the host knows — so the window would stay silent about the one close that cannot be undone
while the host deleted anyway. The host answers with the same conjunction it deletes by.

**There is no clone-success message.** The clone registers the project and the existing
`ProjectAdded` is the signal. A second message saying the same thing would be a second thing to keep
in step.

**A registration is addressed by id.** Naming registrations means provider-and-origin stops being
unique, so `OauthAppId` keys the record, the secret and the connection that authenticates as it. A
record from an older file has neither id nor name; both are derived deterministically from provider
and origin, because the settings file is re-parsed on every read and a minted id would answer
differently each time.

## What the interface kit needed

Three defects surfaced through this work and were fixed in the kit rather than worked around.

**`elided`'s fourth argument is a font size**, not a width. Three call sites passed a width, so a
path rendered at 300px — one letter filling the modal, the neighbouring text squeezed into a
one-character column. Width belongs on a wrapping `max_w`.

**A dropdown inside a modal was painted underneath it.** `kit::menu` defers its list at one layer
and `kit::modal` defers its overlay at a higher one, so any picker in any modal opened behind the
panel holding it: no list, no search, nothing clickable. `Picker::above_modal()` raises it. Opt-in
rather than global, so no existing dropdown changes.

**A column that overflows does not clip.** The 300px row collapsed the repository list to nothing
and its contents painted over the sections beneath. The modal's body scrolls and the listing keeps a
floor, so overflow cannot become overlap.

Escape closes the modal and peels one layer at a time: an open picker first, the modal on the next
press. The rule is the window's rather than the modal's: `AppState::cancel_dialog` peels the layers
for every dialog, and this modal binds nothing. Binding it here took two bindings — one for the
modal and one for the field inside it — because the component library binds `escape` for its inputs
at the deepest node in the tree, where a keymap's ties are broken.

## What is not built

| | |
|---|---|
| `G149` | https only. An ssh remote parses so the modal can name it unsupported, and the clone refuses it |
| `G150` | A failed clone is not resumed or retried |
| `G151` | Discovery is the membership listing plus the provider's search; there is no org-wide enumeration |
| `G152` | Azure DevOps, Atlassian and Google have no listing behind them and answer `Unsupported` |
| `G157` | A client secret stored before registrations were named must be entered again |
| `G158` | A secret typed for a registration that does not exist yet is matched back by name and client id |
| `G159` | Nothing draws *which* registration a connection authenticates as |
| `G32` | The destination chooser is the platform dialog, so a detached host is wrong here — touched, not closed |

## How far it is verified

The workspace builds clean and its tests pass, `clippy` included. What the tests cover is the part
worth covering: that `forget` deletes a clone inside the ephemeral root, that it never deletes a
dropped folder outside it, that it never deletes a settled project inside it, that a path climbing
out with `..` does not pass the gate, that the sweep spares what a record names, that an old
settings blob still decodes, and that `parse_repo_url` accepts and rejects what it should.

**Nothing has been run against a live provider.** No repository listing, no branch API call, no
OAuth flow through a real registration, and no clone of a private repository with a real token has
been exercised outside a standalone check of the libgit2 mechanics. The whole application-registration
surface — the form, the callback URL, the connect flow's picker — is built and unproven. The first
thing to do with this document open is to register an application against GitHub, connect through
it, and clone something private.

The modal itself has never been driven by hand either. Its layout was corrected against screenshots
rather than by watching it work.

## Related docs

- [`features/connectors.md`](../features/connectors.md) — the identity a clone borrows, and the
  registration it authenticates as
- [`tech/version-control.md`](../tech/version-control.md) — why creating a repository leaves the
  read-only rule intact
- [`backlog.md`](../backlog.md) — the rows above, in the register that owns them
