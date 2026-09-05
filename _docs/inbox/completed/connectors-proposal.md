---
id: inbox-connectors
title: Proposal — connectors
kind: proposal
status: proposal
summary: A Connectors section in settings that holds many named connections to GitHub, GitLab, Gitea, Azure DevOps, Atlassian and Google Workspace, cloud or self-hosted — several per provider — each authenticated by a personal access token, a device code or a PKCE authorization flow on a loopback callback, with the token material stored in the OS keychain through the library's existing `SecretStore`, never crossing the bus, and an untrusted certificate on a self-hosted instance resolved by pinning one confirmed fingerprint to that instance.
read_when: you are deciding how Ubiq authenticates against an external service, where those tokens live, or how a browser-based login reaches a desktop application
updated: 2026-09-05
depends_on: [tech-architecture, tech-transport, tech-agent-manager, feat-workbench, inbox-config]
---

# Proposal — connectors

Ubiq authenticates against exactly one class of thing today: harnesses, through the account family,
by driving the harness's own login in a pane and capturing what it writes. It has no way to
authenticate against a *service* — to hold a GitHub or GitLab token, a Google Workspace grant, an
Azure DevOps scope, a Jira API token — and so every capability that would need one (private clones,
issue and work-item lookups, documents, pipeline status) is blocked on the same missing piece. That
holds equally for the self-hosted half of that list: a GitLab, Gitea or Azure DevOps Server behind a
company's own domain is the environment where a private repository is most likely to live.

This proposes that piece and nothing more: **a connection is an authenticated identity at a
provider, and this document is about how one comes into being, where its token lives, and how the
interface learns which exist.** What reads a connection — a git remote, a task source, a document
picker — is a later document; §12 says why drawing that line now is the point.

The shape is deliberately the account family's, one step further out: many named identities, several
per provider, created by completing a flow rather than by an `Add` message, and described on the
wire by a reference that carries no material.

## 1. Where it stands

- **`crates/agent-manager/src/credentials/`** already is the secure store this needs. `SecretStore`
  is a five-method trait — `list`, `get`, `set`, `delete`, `rename` — keyed by
  `CredentialId { harness, name }` over `CredentialBlob`s, with four engines behind
  `build_secret_store`: `files`, `keychain`, `os` (macOS Keychain, via a dedicated
  `am.keychain-db`) and `keyring` (the `keyring` crate, feature-gated). `credential_validity`
  already reads an expiry out of stored JSON without calling anyone.
- **`Account` is the wrong home.** `crates/agent-manager/src/account.rs` states the invariant in its
  own doc comment: an account holds env-var *names*, a base URL, a helper command, a home path —
  "No field here can hold a secret value." A connector token is material. It belongs in
  `SecretStore`, beside a harness login, not in `Account`.
- **No OAuth code and no HTTP client in Ubiq.** The workspace's only HTTP client is `ureq` 2, used
  by agent-manager's MCP transport. Every OAuth flow in the tree today is delegated to a harness's
  own browser flow. Nothing here has ever exchanged a code for a token.
- **`ubiq://` is already taken.** It is the interface's internal navigation URI —
  `ubiq://<ulid>/ide/...`, `/tasks/...`, `/agents/...` — parsed by `Destination::from_str`. It is
  not an OS-registered scheme: `_tools/Info.plist` declares `CFBundleDocumentTypes` and no
  `CFBundleURLTypes`. §5 is what follows from that.
- **The settings dialog takes a new section cheaply.** `SettingsSection` in
  `crates/ubiq/src/state/settings.rs` is an enum plus `all()` and `label()`; a section is one
  variant, one nav icon, one `body()` arm and one drawing function in `crates/ubiq/src/ui/settings.rs`.
- **Host settings are already versioned and round-tripped.** `HostSettings` in
  `crates/ubiq-proto/src/settings.rs` carries `HOST_SETTINGS_SCHEMA`, persists to
  `<config_root>/host-settings.toml`, and moves over `GetSettings`/`SetSettings`/`Settings`.

## 2. What a connection is

One record, and the vocabulary the rest of this document uses:

| Field | Meaning |
|---|---|
| `id` | A ULID minted by the host when the flow succeeds. Stable; what everything else references |
| `provider` | `github`, `gitlab`, `gitea`, `azure_devops`, `atlassian`, `google`. A closed set (§3) |
| `label` | The user's name for it — "work", "personal", "client-x". Freely renamable |
| `instance` | The base URL this identity lives at: a GitHub Enterprise Server host, a self-managed GitLab, a Gitea or Forgejo install, an Azure DevOps organization or on-premises collection, an Atlassian site or Data Center. `None` means the provider's public cloud, and is the only case where it may be omitted (§3) |
| `auth` | Which flow produced it: `Token`, `Device` or `Oauth` |
| `scopes` | What was asked for, as returned. Non-secret, and the honest answer to "what can this do" |
| `account` | The provider's own display name for the identity — login, email. Fetched once at connect, cached, never re-fetched |
A pinned certificate is deliberately **not** a field here: trust belongs to the instance, not to the
identity that first met it (§3), so it is stored beside the connection list rather than inside a
record (§9).

**Several connections per provider is the default case, not a feature.** Nothing in the record is
unique per provider: two GitHub connections differ only by `id`, and every consumer takes a
connection id rather than a provider name. The user asking for two GitHub accounts is what the model
is shaped around; one is the degenerate case.

**A connection is an identity, not a binding.** It does not name a project, a repository or a board.
Which connection a given project uses is a separate, later decision (§12) — this document ends at
"the token exists and is valid".

## 3. Providers and what each accepts

The provider table is **static, in the host, in code**. Six entries, each naming its API base path,
the flows it supports, its default scopes and an optional built-in client id. A user cannot add a
seventh; adding one is a change to that table.

| Provider | Hosting | Token | Device | PKCE + loopback | API base under `instance` |
|---|---|---|---|---|---|
| GitHub | cloud + Enterprise Server | PAT (classic or fine-grained) | yes — the recommended flow | no | `/api/v3` when self-hosted |
| GitLab | cloud + self-managed | PAT (and project/group tokens) | no | yes, public client | `/api/v4` |
| Gitea | self-hosted only | PAT | no | yes, public client | `/api/v1` |
| Azure DevOps | Services + Server | PAT | no | Services only (Entra ID, tenant app) | `/_apis`, under the collection when on-premises |
| Atlassian | Cloud + Data Center | Cloud: API token + email. DC: PAT | no | Cloud only (3LO) | `/rest/api/…` |
| Google Workspace | cloud only | no | no | yes | — |

Forgejo is a Gitea fork with the same API surface and the same `/api/v1` base; it connects as
`gitea` against its own `instance` rather than earning a seventh entry.

**A personal access token is a first-class flow, not a fallback.** It is the only flow that needs no
registered application, no client id, no callback and no network round trip beyond one validation
call. Every provider that accepts one gets it — and for the self-hosted half of this table it is not
merely the easy path but the *only* one that works without an administrator, which is why phase 2
alone makes every row above usable except Google.

**Self-hosted is what the table is shaped around, not an appendix.** Three consequences, each of
which the interface has to draw:

- **`instance` is required, and it is a base URL, not a host name.** An on-premises install can live
  under a path — a GitLab behind `example.com/gitlab`, an Azure DevOps Server collection at
  `server/tfs/DefaultCollection` — so the record stores what the user typed and the provider entry
  appends its API base to it. Nothing reconstructs a URL from a host name.
- **There is no built-in client id for a self-hosted instance.** An OAuth application on a GitLab,
  Gitea or Azure DevOps Server install is registered *on that install*, by someone who administers
  it. So a per-connection `client_id` is the normal case there, not the exception, and the
  interface says so: choosing a browser flow against a self-hosted instance asks for a client id
  before it opens anything.
- **Azure DevOps Server has no browser flow at all.** Entra ID OAuth covers Azure DevOps *Services*;
  the on-premises product authenticates with a PAT. The provider entry marks the flow as
  cloud-only and the interface offers only the token flow once an `instance` is present.

**A self-signed or private-CA certificate is the common case on-premises**, and refusing it outright
makes the connector useless in exactly the environments that most need it. The host uses the
platform trust store first, so a certificate the machine already trusts works with no setting and no
prompt. A certificate it does not trust does not fail the flow: it stops it, hands the interface the
certificate's details, and waits for the user to vouch for that exact certificate — §4.

**Trust is pinned per instance, to one certificate.** What the user confirms is stored against the
instance's **origin** — scheme, host and port, the same triple a browser scopes anything to — as a
SHA-256 fingerprint of the leaf certificate. It means exactly one thing: for requests to *that
origin*, a certificate whose fingerprint matches is accepted even though the chain does not
validate. Any other certificate at that origin is refused, and every other origin validates
normally. It is not a trusted CA, not a machine-wide exception, and not "verification off".

**Per instance, not per connection**, because the certificate is a property of the server and the
user's answer is about the server. A company's GitLab has one certificate whoever is logging in; two
identities on it are two connections and one machine, and asking the same question twice would teach
the user to click through it. The consequence is stated rather than hidden: **a pin outlives the
connection that created it.** Deleting the last GitLab connection leaves the pin behind for the
next one, and dropping it is the explicit `ForgetCertificate` on that instance (§8) — which the
settings section offers, and which says how many connections it affects before it acts.

**GitHub uses the device flow, not a browser redirect.** GitHub's OAuth apps do not support PKCE for
a public client, which would force Ubiq to ship a client *secret* — a secret in a binary is not a
secret. The device flow needs neither secret nor callback: the host asks for a code, the interface
shows the user code and opens the verification URL, the host polls until the user finishes. This is
also why §5's callback question does not arise for the provider most users will connect first. An
Enterprise Server install is the same OAuth implementation and keeps the flow; only the device and
token endpoints move under `instance`.

**A browser flow needs an application, and an application is not the user's.** The table's built-in
client id is what a public-cloud flow uses; three rows have no usable built-in at all, because Azure
DevOps Services needs a per-tenant Entra ID registration and GitLab, Gitea and every self-hosted
install register their applications on the instance itself. Where that id comes from, and what
happens when it needs a secret, is §7.

## 4. The flows

Four ways to authenticate, one refresh, and one interruption any of them can hit. All of them run
**in the host**. The interface starts one, watches it, answers the questions it asks, and cancels
it; it never sees a token, an authorization code or a client secret.

**Token.** The user pastes a PAT (with an email, for Atlassian Cloud). The host calls the provider
entry's "who am I" endpoint once, under `instance` where there is one — `/user` for GitHub, GitLab
and Gitea alike, `/_apis/profile/profiles/me`, `/rest/api/…/myself` — to learn the `account` and to
fail fast on a bad paste, a wrong instance URL or an untrusted certificate, then stores it. One HTTP
round trip, no state machine, and the only flow every row of §3's table supports.

**Device.** The host posts to the device-code endpoint, answers with the user code and verification
URL, opens the browser, and polls the token endpoint at the interval the provider gave until it gets
a token, a denial, or the expiry. The interface draws the code — a user has to type it — and a
cancel button.

**PKCE authorization code.** The host generates a `code_verifier` and a `state` nonce, binds the
loopback listener (§5), opens the browser at the authorization URL, and waits for exactly one
request. It rejects a callback whose `state` does not match, exchanges the code with the verifier,
and stores what comes back. `code_challenge_method` is `S256`; a plain challenge is not accepted.

**Certificate confirmation.** Not an authentication flow, but the one thing that can interrupt
any of them, and the only path by which a pin comes into being. When a request's TLS handshake fails chain
or hostname validation, the host does not retry and does not proceed: it emits
`ConfirmCertificate` carrying the origin and the leaf certificate's details, and waits.
`TrustCertificate` naming the same origin and the same fingerprint pins it and resumes exactly where
it stopped; a cancel ends the flow with nothing stored. The host never pins on its own, never pins a fingerprint the interface did not send
back, and never continues an unconfirmed handshake to "see if it works".

What crosses the bus is `CertInfo`: `subject`, the subject alternative names, `issuer`,
`not_before`, `not_after`, `sha256` (the leaf's DER fingerprint, formatted for reading),
`self_signed`, and `reason` — `UnknownIssuer`, `HostnameMismatch`, `Expired`, `NotYetValid` — which
is what the dialog needs to say *why* this is being asked. A certificate is public; nothing here is
material.

**Refresh.** A stored token that carries a `refresh_token` and an expiry is refreshed **lazily, by
the host, on use** — never on a timer, never in the background. `credential_validity` already reads
the expiry out of the stored JSON, so the status the settings section shows costs no network call.
A refresh that fails marks the connection `Expired` and the user reconnects; nothing is deleted.

## 5. The callback URL

**v1 is loopback: `http://127.0.0.1:47821/callback`, one fixed port.**

Every provider here accepts a loopback redirect, and it needs no OS registration, no bundle, no
running-instance handoff and no cooperation from a browser beyond opening a URL. The host binds the
port only for the duration of one flow, answers exactly one request with a small "you can close this
tab" page, and drops the listener — on success, on `state` mismatch, on cancel, or after 120
seconds.

The port is fixed rather than ephemeral because GitHub, Azure DevOps and Atlassian all match the
registered redirect URI exactly, port included; an ephemeral port would have to be pre-registered,
which is a contradiction. Google is the one provider that permits a varying loopback port, and it
accepts a fixed one too. If 47821 is busy the flow fails with a message naming the port — it does
not silently pick another, because another port is a redirect URI no provider will accept.

**`ubiq://` is deferred, and not only for cost.** The scheme is already the interface's internal
navigation URI, so registering it with the OS would mean one string meaning two things: a route
inside the window and an inbound OAuth callback. That collision has to be resolved before anything
is registered — a distinct scheme, or a reserved path prefix — and the flows all work without it.
When it lands it is `CFBundleURLTypes` in `_tools/Info.plist`, a Linux `.desktop` handler, a Windows
registry key, and a handoff from a second instance to the running one. A row in `backlog.md`.

<!-- ponytail: one fixed loopback port, no ubiq:// scheme, no second-instance handoff.
     Revisit if a provider appears that refuses loopback, or if a locked-down environment
     blocks the port. -->

## 6. Where the token lives

**In `SecretStore`, under the engine the user already chose for harness logins.** No new storage
layer, no second keychain, no encryption code in Ubiq.

The key is `CredentialId { harness: "connector:<provider>", name: <connection id> }`, which the `os`
and `keyring` engines render as the service string `am:connector:gitlab:<ulid>`. The `harness` field
is being used as a namespace it was not named for; that is deliberate and it is the whole change —
the alternative is a parallel trait with five identical methods.

The blob is a single `token.json`: `access_token`, and where the flow produced them `refresh_token`,
`expires_at`, `token_type`, `scope`. Storing JSON rather than a bare string is what makes
`credential_validity` work unmodified.

**Ubiq must select a secure engine for connectors.** agent-manager's default engine is `files` —
plaintext, appropriate for a captured harness login that is already a plaintext file in a home
directory, and not appropriate for a bearer token Ubiq itself obtained. The host builds the
connector store with `os` (or `keyring`), and refuses to complete a flow if no secure engine is
available rather than falling back to plaintext. That refusal is a message the user can act on:
"connectors need a secure credential store".

## 7. Where the application's own credentials come from

Two different secrets have been conflated so far, and the rest of this section depends on keeping
them apart. A **connection credential** is the token from §4: it identifies the *user* and it is
theirs. An **application credential** — a client id, sometimes a client secret — identifies *Ubiq*
to the provider, is the same for every user of a build, and exists only because a browser flow
demands one. Everything above is about the first. This is about the second.

**Ubiq ships no client secret. Ever.** A secret embedded in a binary that is distributed is not a
secret — it is a value anyone with the file can read, and every provider's terms treat it as
compromised the moment it ships. So the built-in applications are **public clients only**: PKCE for
Google, GitLab, Gitea, Atlassian Cloud and Azure DevOps Services, and the device flow for GitHub,
which is precisely why §3 chose that flow rather than an OAuth-app redirect. What gets embedded is
therefore only ever a **client id**, which is public by construction, travels in the query string of
every authorization URL, and needs no protection at all.

That rule is what makes the build-time half of this safe, and it is the one thing here that must not
be relaxed quietly.

### The three sources, in order

One resolution function, highest precedence first — the shape `resolve_engine` already uses in
agent-manager:

1. **The connection's own `client_id`**, set when it was created. The self-hosted case: an
   application registered on that GitLab or Gitea by whoever administers it.
2. **A provider entry in settings**, keyed by provider and optionally by instance origin. The
   organisation case: one Entra ID registration, one Atlassian app, configured once and used by
   every connection to that instance.
3. **The build's embedded id**, if the build has one. The default case: the public cloud, working on
   first run with nothing configured.

If all three are empty the provider simply has no browser flow, and the interface says so rather
than opening a URL that will be rejected — §3's table already gives every provider but Google a
token flow, so "no application configured" degrades to PAT rather than to nothing.

### Embedding at build time

`option_env!("UBIQ_OAUTH_<PROVIDER>_CLIENT_ID")` in the host's provider table, read at compile time.
An exported variable is baked into that build; an unset one leaves `None` and the row falls through
to sources 1 and 2. The bundling script in `_tools/` is where a release exports them, from whatever
its CI holds them in; nothing is committed, and nothing is read from the environment at *run* time —
a variable set in a user's shell must not be able to change which application a flow authenticates
as.

Three properties follow, and each is the reason for the choice:

- **A build from source is not broken, only unbranded.** No variables exported means no built-in
  applications, which means PAT and device flows everywhere and BYO client id for the rest. That is
  a working application, not a degraded one.
- **The official build needs no configuration for the common case.** Connecting a personal Google or
  Atlassian account is one click, because the id is already there.
- **A fork ships its own applications by exporting its own ids**, and cannot accidentally ship
  Ubiq's, because there is nothing in the repository to inherit.

### Configuring one in settings

The Connectors section grows an **Applications** list beside its connections: one row per configured
provider — provider, instance origin if it is not the cloud, the client id, and whether a secret is
stored. Each row also states where the *effective* id came from, in the resolution order above:
`built in`, `from settings`, or `not configured`. That line is the answer to "why is the GitLab
button greyed out", which is otherwise an unanswerable question.

Editing one is a client id field and, only where a provider genuinely demands a confidential client,
a secret field that is write-only: it shows `set` or `not set`, never the value, and clearing it is
its own action.

**A user-supplied client secret is material, so it lives where material lives** — `SecretStore`,
under `CredentialId { harness: "connector-app:<provider>", name: <origin, or "cloud"> }`, beside the
tokens and under the same secure-engine requirement (§6). It never enters `host-settings.toml`, and
the settings record holds only `has_secret: bool`, computed from the store.

This is the second variant that carries material across the bus — `SetAppSecret { provider, origin?,
secret }`, the same `Secret` newtype, the same redaction obligation as `SubmitConnectSecret`. §14
asks for both to be ratified together rather than one at a time, because the rule being written is
not "one exception" but "material crosses only in variants that say so in their type".

<!-- ponytail: no per-user override of a built-in id, no application management outside settings,
     no rotation. Add a rotation story if a shipped client id ever has to be replaced mid-release. -->

## 8. The connector family

The eleventh family. An eleventh entry in the transport contract's family list, following the
account family's rules because it is the same problem one layer out.

| Message | Direction | Payload | Responds with |
|---|---|---|---|
| `ListConnections` | UI → host | — | `Connections` |
| `Connections` | host → UI | `connections` | — |
| `BeginConnect` | UI → host | `provider`, `instance?`, `label`, `auth`, `client_id?` | `ConnectPending` or `ConnectFailed` |
| `ConnectPending` | host → UI | `connect_id`, `stage` | — |
| `ConnectCaptured` | host → UI | `connect_id`, `connection` | `Connections` follows |
| `ConnectFailed` | host → UI | `connect_id`, `error` | — |
| `CancelConnect` | UI → host | `connect_id` | — |
| `SubmitConnectSecret` | UI → host | `connect_id`, `secret` | `ConnectCaptured` or `ConnectFailed` |
| `RenameConnection` | UI → host | `connection`, `label` | `Connections` or `ConnectorError` |
| `DeleteConnection` | UI → host | `connection` | `Connections` or `ConnectorError` |
| `CheckConnection` | UI → host | `connection`, `probe` | `ConnectionStatus` |
| `ConnectionStatus` | host → UI | `connection`, `status` | — |
| `ConfirmCertificate` | host → UI | `scope`, `origin`, `cert` | `TrustCertificate` or a cancel |
| `TrustCertificate` | UI → host | `scope`, `origin`, `sha256` | resumes the flow, or `ConnectorError` |
| `ForgetCertificate` | UI → host | `origin` | `Settings` or `ConnectorError` |
| `SetAppSecret` | UI → host | `provider`, `origin?`, `secret` | `Settings` or `ConnectorError` |
| `ClearAppSecret` | UI → host | `provider`, `origin?` | `Settings` or `ConnectorError` |
| `ConnectorError` | host → UI | `error` | — |

**`ConnectionInfo` carries no material.** It is §2's record: id, provider, label, instance, auth,
scopes, account name, status. The log sink listens to the same bus, so the account family's rule
applies unchanged — a token here is a token in a log a user might paste into an issue.

**`ConnectPending` is how one flow reports itself, and `stage` is a typed enum, not a string.**
`Opening`, `DeviceCode { user_code, verification_url, expires_in }`, `AwaitingCallback { port }`,
`Exchanging`, `NeedSecret { prompt }`, `AwaitingCertificate`. One `connect_id` minted by the UI, the search family's
pattern: the host answers with a stream of stages terminated by exactly one of `ConnectCaptured` or
`ConnectFailed`, and the UI discards anything naming an id it no longer holds. `ConnectFailed`'s
error is likewise typed — `Denied`, `Expired`, `PortBusy`, `StateMismatch`, `NoSecureStore`,
`Tls(String)`, `BadInstance`, `Http(String)` — because "which of these happened" is exactly what the interface needs to draw.

**Two variants carry material, and they are the decision this document asks for.**
`SubmitConnectSecret` takes a pasted PAT; `SetAppSecret` takes a user-supplied client secret (§7).
Every alternative is worse: a temporary file is a plaintext file, and an env-var reference
(agent-manager's answer for accounts) asks a desktop user to restart the application with a variable
set. So material crosses the bus in exactly these two, and the contract takes on a second obligation
beside the first: their payload is a `Secret(String)` newtype whose `Debug` prints `Secret(***)`,
and the log sink records the variant's name and never its payload. The rule to ratify is not "one
exception" but "material crosses only in a `Secret`, and a `Secret` is never logged" — §14.

**No message ever carries a client *id*.** It is public, it lives in `host-settings.toml`, and it
rides `GetSettings`/`SetSettings` like any other setting. Only the secret half needs a variant of
its own, which is why there is no `SetOauthApp`.

**`CheckConnection` has two modes, and only one of them touches the network.** `probe: false` is
what the list draws: `credential_validity` over the stored blob, no request, no certificate, no
latency — the rule that this family calls nobody is kept for the path that runs on every render.
`probe: true` is the "Check" button and the one place a live handshake happens, which makes it the
only place an existing connection's pin can be established or replaced.

**`origin` is what gets pinned; `scope` is only what gets resumed.** The two are separate fields
because they answer different questions: `origin` is the server the user vouched for, and `scope`
is `Connect(connect_id)` or `Connection(id)` — whichever stalled waiting for the answer. That split
is what makes the pin instance-wide while keeping one message able to unblock either moment, and it
is why a pin made during a flow that is then abandoned still stands: the user's answer was about the
server, and the server has not changed.

**A pin is answered, never assumed.** `TrustCertificate` must carry the same `sha256` the host
offered; anything else is a `ConnectorError` and the flow stays stopped. This is what makes the
confirmation meaningful rather than a formality the interface can click through on the user's
behalf, and it is why the fingerprint — not a boolean — is the payload.

**Creating a connection is completing a flow.** There is no `AddConnection`. `BeginConnect` mints a
`connect_id`, not a connection; a connection exists only when a token is stored, so an abandoned
flow leaves nothing behind — the account family's rule, for the same reason.

**Deleting a connection deletes the token, and only the token.** `DeleteConnection` removes the
`SecretStore` entry and the record together. It does not touch the pin, which belongs to the
instance and may be the reason another connection still works — `ForgetCertificate` is the separate
operation for that, keyed by origin, and after it the next probe to that origin validates normally.
A pin nothing references any more is harmless and visible; the settings section lists it, which is
how the user finds one to drop. There is no "sign out but keep the name": unlike a harness account, which is a
home directory that survives its login, a connection with no token is nothing.

**Status is what the token says about itself.** `ConnectionStatus` is `credential_validity` over the
stored blob — `Valid`, `Expired`, `Unknown`, `Empty`. Nothing calls the provider, so `Valid` means
"not expired", not "will work"; a revoked token still reads `Valid` until something uses it.

## 9. Where the non-secret record lives

**In `HostSettings`, as `connections: Vec<Connection>`, with `HOST_SETTINGS_SCHEMA` bumped to 3.**

It persists to `<config_root>/host-settings.toml`, is read at boot by the `GetSettings` the interface
already sends, survives an older reader through the existing `#[serde(default)]` discipline, and
needs no new store, no new file, no new messages to read it. `ListConnections` exists for the
refresh-after-a-flow case rather than as the primary path.

<!-- ponytail: connections ride the host-settings blob, so a `SetSettings` is last-writer-wins over
     the whole thing. A dedicated ConnectorStore in ubiq-host/src/store/ if that bites. -->

Two consequences worth stating rather than discovering: the interface holds the full list in state
like any other host setting, and a connection record must stay small — an access log or a per-repo
cache does not belong in it.

**Configured applications are a list in the same blob**: `oauth_apps: Vec<OauthApp>`, each
`{ provider, origin?, client_id, has_secret }` (§7). The client id is public and the flag is
derived, so the whole record is safe in a file the user can open and hand to a colleague.

**Pinned certificates are a second list in the same blob**, not a field on a connection:
`trusted_certs: Vec<TrustedCert>`, each `{ origin, sha256, subject, issuer, not_after }`, keyed by
origin. That is what makes the pin instance-wide for free — a connection finds its pin by looking up
its own instance's origin, and two connections to one server find the same row. Everything in it is
public and small, and a fingerprint the user is meant to read back and compare against what their
administrator tells them belongs in a file they can open.

<!-- ponytail: the pin is the leaf certificate's fingerprint, so a renewal re-prompts. Pinning the
     SPKI instead would survive renewal and is the upgrade if re-confirming annoys anyone. -->

## 10. The settings section

`SettingsSection::Connectors`: one enum variant, one `all()` entry, one `label()`, one nav icon, one
`body()` arm, one drawing function beside `search` and `harnesses` in `crates/ubiq/src/ui/settings.rs`.

It draws a list, not a form. One row per connection — provider glyph, label, account name, instance
if any, and the status as a badge — with rename and disconnect on the row. Below it, one "Connect…"
control that opens a provider picker, and from there a modal that draws whatever `ConnectPending`'s
current stage says: a spinner, a device code to type with a copy button, a field for a PAT, an
error with a retry.

The picker asks two things before the flow starts, and only where the provider entry says they are
needed: the **instance URL** — required for every self-hosted row, absent for a public cloud — and,
once an instance rules out a built-in client id, the **client id** of an application registered on
that instance. Which flows the modal then offers is read from the provider entry against the
instance, so an Azure DevOps Server connection is simply never shown a browser button.

The modal is the whole flow surface. There is no wizard, no per-provider screen, and no scope
editor — scopes are what the provider table asks for, shown afterwards as fact.

**The certificate dialog is the one place the interface asks the user to take a risk, so it reads
like one.** It states what failed in a sentence — this certificate is signed by an issuer this
machine does not trust; this certificate is for a different host — and then shows the facts to check
against: subject and its alternative names, issuer, validity window, and the SHA-256 fingerprint in
grouped hex, selectable, beside a copy button. Its confirming action is named for what it does
("Trust this certificate"), not "OK"; its default action is cancel; and it names the instance the
trust applies to, saying that every connection to it shares the answer.

**Applications and trusted certificates sit below the connections**, in that order: §7's
Applications list, then the trusted certificates. Both are configuration a user visits rarely and
needs to find immediately when a flow refuses to start.

**Trusted certificates are their own short list at the foot of the section**, one row per origin
with the short fingerprint, the issuer, the expiry and how many connections currently use that
instance — because a trust that outlives its connection has to be findable to be revocable. "Forget"
sits on the row and says what it will affect before it acts. A connection whose instance is pinned
carries a small badge in its own row too, so the fact is visible where the user is looking.

## 11. Failure

- **Browser will not open.** The stage carries the URL; the interface shows it with a copy button.
  The flow keeps waiting.
- **Port 47821 busy.** `ConnectFailed { PortBusy }` before the browser opens. Nothing partial.
- **User closes the modal.** `CancelConnect` drops the listener or the poll. No record, no token.
- **Window closes mid-flow.** The host drops the flow with the client; a token that arrived after
  the cancel is discarded rather than stored, because a connection nobody asked for is worse than a
  flow to repeat.
- **No secure credential store.** `ConnectFailed { NoSecureStore }`, before any browser opens. The
  application never writes a bearer token to a plaintext file.
- **Instance URL wrong, or not the product it claims.** `ConnectFailed { BadInstance }` from the
  one validation call, before anything is stored — a Gitea URL typed into a GitLab connection fails
  here rather than at first use.
- **Untrusted certificate.** The flow stops at `ConfirmCertificate` and waits. Cancelling leaves
  nothing; trusting pins that certificate to the instance and resumes (§3, §4).
- **A pinned certificate is replaced.** The next probe to that origin fails validation again and
  raises the dialog again with the new certificate's details — a renewal and an interception look
  identical to the machine, so the user is asked, and the old pin is replaced only by an answer.
  Every connection to that instance is unblocked by the one confirmation.
- **The last connection to a pinned instance is deleted.** The pin stays, listed in §10's trusted
  certificates with a count of zero. It is not silently collected: a trust the user granted is a
  trust the user revokes.
- **A handshake fails for something other than a certificate** — reset, protocol, no route:
  `ConnectFailed { Tls }`, with nothing to confirm and nothing to pin.
- **Instance unreachable after connecting.** Nothing changes: status is read from the stored token's
  own expiry, so a VPN that is down does not make a connection look broken.
- **Keychain read denied at use time.** The connection reads as `Empty`; the row offers reconnect.
- **Refresh fails.** `Expired`, and the token is kept — a provider outage is not a reason to make
  the user re-authorize.

## 12. What this does not try to be

**It is not an API client.** Nothing here fetches a repository, an issue, a work item or a document.
The deliverable is a valid token and the identity it belongs to; every consumer is a later document
with its own family. That boundary is the reason this can land and be verified on its own.

**It is not a project binding.** "Which connection does this project use for its remote" is a real
question with a real answer, and it is not this document's — a binding names a connection id, so it
can be added without touching anything here.

**It is not a secrets manager.** Ubiq stores what its own flows obtained, under keys it minted.
Importing a token from elsewhere, sharing one between machines, or handing one to a harness is out
of scope, and the last of those is the account family's job.

**It is not sync.** No background refresh, no polling for revocation, no webhook.

## 13. Phases

1. **The record and the section.** `Connection` in `ubiq-proto`, `HostSettings` schema 3, the
   `Connectors` settings section drawing an empty list. Nothing authenticates yet.
2. **The token flow, with `instance`.** `SubmitConnectSecret`, the secure-engine selection,
   `SecretStore` under the `connector:` namespace, the per-provider validation call under a base
   URL. Five of §3's six providers become usable here, cloud and self-hosted alike — this is the
   phase that makes the feature real, and self-hosted is not deferred past it.
3. **The device flow.** GitHub and GitHub Enterprise Server without a PAT.
4. **PKCE and the loopback listener.** Google first — the only provider with no token flow — then
   GitLab and Gitea against a user-supplied client id, then Azure DevOps Services and Atlassian
   Cloud. Refresh lands with it, since these are the flows that produce refresh tokens.
5. **`ubiq://`**, if anything still needs it.

§7's application credentials land with phase 4, the first phase that needs one: the resolution
order, the `option_env!` reads and the Applications list are what make that phase's flows reach a
provider at all.

Certificate confirmation lands **with phase 2**, not after it: the token flow is what makes
self-hosted work, and a self-hosted instance behind a private CA is the case that flow exists for.

## 14. What this asks to be decided

1. **May a variant carry secret material if its type says so?** `SubmitConnectSecret` and
   `SetAppSecret` (§8) are the only ways a pasted PAT or a user-supplied client secret reaches the
   host. Ratifying them means the transport contract's "no material on the bus" rule gains a typed
   exception — a `Secret` newtype the log sink refuses — rather than a list of blessed variants;
   refusing it means dropping the token flow, which is phase 2 and most of the value.
2. **Is the provider set closed?** Six entries in code (§3), Forgejo folded into `gitea`. A
   user-extensible table is a generic-OAuth-client feature, and a different document — but note
   that a self-hosted instance already carries its own URL and client id, so the distance from six
   entries to "any GitLab-shaped thing" is smaller than it looks.
3. **Certificate pinning is decided: a confirmed leaf fingerprint, per instance origin** (§3, §4,
   §8). What it leaves open is lifecycle — a pin outlives every connection that justified it, and
   this document chooses to list it rather than collect it. Collecting it automatically would mean
   a user who deletes and re-adds a connection is asked again for a certificate they already
   approved.
4. **Fixed port 47821** (§5) — the number itself, and the choice to fail rather than to fall back.
5. **`connector:` as a `CredentialId.harness` namespace** (§6), versus a second trait in
   agent-manager. This one is a change to that crate's vocabulary, so it is theirs to accept.
6. **Connections in the host-settings blob** (§9), versus their own store.
7. **Ubiq ships no client secret** (§7) — built-in applications are public clients, which is what
   makes a build-embedded client id harmless. This is worth a decision-register row rather than a
   paragraph, because relaxing it later would look like a small convenience and would not be one.

## Related docs

- [`tech/transport-contract.md`](../../tech/transport-contract.md) — the ten existing families, the
  account family this one is modelled on, and the procedure for adding a variant
- [`tech/agent-manager.md`](../../tech/agent-manager.md) — the boundary this leans on for storage
- [`inbox/config-persistence-proposal.md`](../config-persistence-proposal.md) — the config root,
  the stores and the on-disk layout §9 writes into
- [`features/workbench.md`](../../features/workbench.md) — the settings dialog as a surface
