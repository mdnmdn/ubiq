---
id: feat-connectors
title: Connectors
kind: feature
status: draft
summary: Named authenticated identities at GitHub, GitLab, Gitea, Azure DevOps, Atlassian and Google Workspace — cloud or self-hosted, several per provider — created by completing a flow, with the token in the OS keychain and an untrusted certificate resolved by pinning one confirmed fingerprint to the instance.
read_when: you are changing how Ubiq authenticates against an external service, where those tokens live, or how a browser-based login reaches the application
updated: 2026-09-05
verified: 2026-09-05
code_anchors: [crates/ubiq-proto/src/connectors.rs, crates/ubiq-proto/src/messages.rs, crates/ubiq-proto/src/settings.rs, crates/ubiq-host/src/connectors/mod.rs, crates/ubiq-host/src/connectors/flow.rs, crates/ubiq-host/src/connectors/tls.rs, crates/ubiq-host/src/connectors/providers.rs, crates/ubiq-host/src/connectors/store.rs, crates/ubiq-host/src/settings.rs, crates/ubiq/src/ui/settings.rs, crates/ubiq/src/state/settings.rs, crates/ubiq/src/app/settings.rs]
depends_on: [tech-transport, tech-agent-manager, feat-workbench]
review_cycle: quarterly
---

# Connectors

## Purpose

A **connection** is an authenticated identity at an external service. It is what lets Ubiq hold a
GitHub token, a GitLab identity on a company's own install, an Azure DevOps scope or a Google
Workspace grant — the missing piece every capability that reaches a service is blocked on.

It is deliberately only that piece. A connection is an identity, not a binding: it does not name a
project, a repository or a board, and nothing here fetches one. Cloning a project is the consumer
that reads one — it takes a connection id, lists that identity's repositories and clones the chosen
one — and it belongs to [`workbench.md`](./workbench.md) rather than to this document. GitHub,
GitLab and Gitea answer it; the other three providers have no repository listing behind them.
The deliverable is a valid token and
the identity it belongs to.

The shape is the account family's, one layer out. An account is one authentication a *harness* runs
as; a connection is one authentication a *service* runs as. Both are many named identities, both
come into being by completing a flow rather than by an `Add` message, and both are described on the
wire by a record that carries no material.

## Behaviour

**Several connections per provider is the ordinary case.** Nothing in the record is unique per
provider: two GitHub connections differ only by id, and every consumer takes a connection id rather
than a provider name. One is the degenerate case, not the design.

**Six providers, in code.** GitHub, GitLab, Gitea, Azure DevOps, Atlassian and Google Workspace. A
user cannot add a seventh. Forgejo connects as `gitea` against its own instance, because it is a
fork with the same API surface and the same base path.

**Self-hosted is what the provider table is shaped around.** Three consequences the interface draws:

- **An instance is a base URL, not a host name.** An on-premises install can live under a path — a
  GitLab behind `example.com/gitlab`, an Azure DevOps Server collection at
  `server/tfs/DefaultCollection` — so what the user typed is stored and the API base is appended to
  it. Nothing reconstructs a URL from a host name.
- **A browser flow against a self-hosted instance asks for a client id first**, because an OAuth
  application on that install is registered *on that install*, by whoever administers it. There is
  no built-in id to fall back to.
- **Azure DevOps Server is never offered a browser flow at all.** Entra ID covers Azure DevOps
  *Services*; the on-premises product authenticates with a token. The interface reads which flows
  are offered from the provider table rather than special-casing a provider.

**A personal access token is a first-class flow, not a fallback.** It is the only one that needs no
registered application, no client id, no callback and no round trip beyond one validation call — and
for the self-hosted half of the table it is the only flow that works without an administrator.

**Four flows, all of them in the host.** A pasted token, validated by one "who am I" call under the
instance. The device flow, where the interface shows a code to type and the host polls. A PKCE
authorization code returned to a loopback listener. And a probe — an existing connection checked
against the network — which is a flow for the same reason the others are: a handshake must never
happen on the thread that carries keystrokes. The interface starts one, watches it, answers the
questions it asks and cancels it; it never sees a token, an authorization code or a client secret.

**An untrusted certificate stops a flow rather than failing it.** The platform trust store is tried
first, so a certificate the machine trusts needs no setting and no prompt. One that does not
validate hands the interface the certificate's details and waits. Confirming it pins that exact
certificate to the instance's origin and resumes where the flow stopped; cancelling leaves nothing.

**Trust is pinned per instance, to one certificate**, because the certificate is a property of the
server and the user's answer is about the server. Two identities on a company's GitLab are two
connections and one machine, and asking twice teaches the user to click through. The consequence is
stated rather than hidden: a pin outlives the connection that created it, is listed rather than
collected, and is dropped by its own explicit action.

**Nothing here calls a provider on a schedule.** Status is what the stored token says about its own
expiry, so the list costs no network call and a VPN that is down does not make a connection look
broken. `Valid` means "not expired", not "will work".

## Contract

The connector family in [`../tech/transport-contract.md`](../tech/transport-contract.md), which owns
the variants, their payloads and their direction. Two of them carry material, in a `Secret` whose
`Debug` redacts itself — see [D65](../tech/decisions.md).

The records — `connections`, `oauth_apps` and `trusted_certs` — ride the host settings blob, which
is persisted, versioned and round-tripped. The host owns those three fields: the interface
mirrors the whole record and writes the whole of it back, so its older copy must not be allowed to
carry away a connection a flow has just made.

## Implementation

`crates/ubiq-proto/src/connectors.rs` holds what both halves need: the provider enum, which flows
work where, and the records. Endpoints, embedded client ids and poll intervals are the host's alone,
in `crates/ubiq-host/src/connectors/providers.rs`.

`crates/ubiq-host/src/connectors/` is the engine. One thread per flow, and one bounded channel per
flow doing all three jobs a flow needs — asking the interface a question and blocking for the
answer, sleeping a poll interval while staying interruptible, and reporting whether anyone is still
listening. Cancelling is dropping the sender, which is why a window closing needs no code of its
own. `flow.rs` runs a request through one helper that turns a certificate refusal into a
confirmation and retries exactly once; `tls.rs` is the verifier behind it, which validates normally
first and only then consults the pin.

The token lives in the harness library's `SecretStore` under `connector:<provider>`, as one
`token.json` — JSON rather than a bare string, which is what lets `credential_validity` read its
expiry unmodified. The engine is chosen rather than inherited: the secure store explicitly, never
the library's plaintext default.

`crates/ubiq/src/ui/settings.rs` draws the section as a list, not a form: one row per connection with
its status, a "Connect…" control opening a modal that draws whatever the current stage says, and
below them the configured applications and the pinned certificates. The certificate dialog is the
one place the interface asks the user to take a risk, so it states what failed, shows the facts to
check against, names the instance the answer applies to, and its confirming action is named for what
it does.

## Failure

- **Browser will not open.** The stage carries the URL and the interface offers it as a link. The
  flow keeps waiting.
- **Port 47821 busy.** The flow fails by name before the browser opens. Nothing partial, and no
  fallback port — another port is a redirect URI no provider will accept.
- **The user closes the modal, or the window.** The flow is dropped. No record, no token; one that
  arrives after the cancel is discarded rather than stored, because a connection nobody asked for is
  worse than a flow to repeat.
- **No secure credential store.** The flow fails before any browser opens. The application never
  writes a bearer token to a plaintext file.
- **The instance URL is wrong, or is not the product it claims.** The one validation call fails
  before anything is stored, so a Gitea URL typed into a GitLab connection fails here rather than at
  first use.
- **An untrusted certificate.** The flow stops and waits. Cancelling leaves nothing; trusting pins
  that certificate to the instance and resumes.
- **A pinned certificate is replaced.** Validation fails again and the dialog is raised again with
  the new certificate's details — a renewal and an interception look identical to the machine, so
  the user is asked, and the old pin is replaced only by an answer. One confirmation unblocks every
  connection to that instance.
- **A handshake fails for something other than a certificate** — a reset, a protocol mismatch, no
  route: the flow fails with nothing to confirm and nothing to pin.
- **The instance is unreachable afterwards.** Nothing changes: status is read from the stored
  token's own expiry.
- **The keychain refuses at use time.** The connection reads as having nothing stored, and the row
  offers to reconnect.

## Related docs

- [`../tech/transport-contract.md`](../tech/transport-contract.md) — the connector family, and the
  account family it is modelled on
- [`../tech/agent-manager.md`](../tech/agent-manager.md) — the boundary the token storage leans on
- [`workbench.md`](./workbench.md) — the settings dialog this section sits in

## Next steps

- Refresh a stored `refresh_token` lazily, on use.
- Give the three providers with no repository listing — Azure DevOps, Atlassian and Google
  Workspace — something a connection to them is worth holding for. Cloning is the first consumer of
  a connection id, and it reaches only GitHub, GitLab and Gitea.
- A second consumer beyond cloning: a task source, or a document picker.
