---
id: inbox-harness-quota
title: Problem — Ubiq never says how much of the plan is left
kind: proposal
status: proposal
summary: Ubiq records what the agents spent and never says what remains; the one quota fact it receives (Claude's `RateLimitUpdate`) arrives per conversation, is stored per conversation, and is drawn nowhere but an overage tooltip. This proposes an account-level quota fact shaped as a list of gauges rather than a union of every provider's fields, a per-harness prober in agent-manager reusing the credential path accounts already own, a five-minute poll over accounts with a live agent, one popup drawn once and opened from two places, and a threshold alert through the bell.
read_when: you are adding a usage-limit readout, probing a provider for quota, adding a periodic host job, or asking why the 5h percentage is not on screen
updated: 2026-09-09
depends_on: [tech-agent-manager, tech-transport, feat-stats, feat-notifications, feat-chat]
---

# Problem — Ubiq never says how much of the plan is left

Ubiq keeps a careful record of what has been spent. `usage.db` accumulates tokens per hour, per
project, per harness, per account (`crates/ubiq-host/src/store/usage.rs`), and the Control screen's
AI page draws it. Every one of those numbers is about the past.

What the user actually wants to know before starting a long agent run is the other direction: **how
much of this week's allowance is left, and when does the window reset.** No screen in Ubiq answers
it, and for four of the five harnesses no part of the tree even asks.

## 1. What already exists, and why it is not the feature

One quota fact does reach the interface today, from Claude only:

| Piece | Where | What it is |
|---|---|---|
| `RateLimitWindow { utilization_pct, resets_at }` | `crates/agent-manager/src/io/model.rs:622` | The provider's own window reading |
| `AgentEvent::RateLimitUpdate { five_hour, seven_day, status, overage_status, overage_reason }` | `io/model.rs:809` | Pushed by Claude's NDJSON bridge from its `rate_limit_event` (`io/jsonl.rs`) |
| `ConvUpdate::RateLimit(RateLimitRecord)` | `crates/ubiq-proto/src/conversation.rs:389` | Mapped by `ubiq-host/src/conversation.rs:688` |
| `Conversation::rate_limit` | `crates/ubiq/src/state/conversation.rs:219` | Held per conversation |

Three properties of that path are why it is not the feature, and each one is a design constraint on
what replaces it rather than a bug to fix:

1. **It is attached to the wrong thing.** A rate-limit window belongs to an *account*, not to a
   conversation. Two agents signed in as the same account read the same window and each holds its
   own copy of it; an account with no agent running holds none at all, which is exactly the moment
   the user wants to ask.
2. **It is push-only, and only Claude pushes.** `io/codex.rs:950` fills none of the accounting
   contract, and `io/opencode.rs:36` and `io/copilot.rs:39` say outright that they never will from
   the stream they read. Nothing anywhere can *ask*.
3. **The percentage was taken off the screen deliberately.** `crates/ubiq/src/ui/conversation/mod.rs:1796`
   says so in a comment: the `5h N%` readout was removed and only `overage_status` survives, in the
   account chip's tooltip. A proposal that puts a bare percentage back in the header is re-proposing
   something already rejected. **The readout comes back as a thing the user opens, not as a number
   that sits in the chrome.**

Two more facts about the ground: the host has **no ticker** — the run loop's 500 ms `recv_timeout`
exists to flush preference debounce (`coordinator.rs:56`), and the only poll loop in the tree is
`nap(job, interval)` inside one connector's own worker thread (`connectors/flow.rs:129`). And both
`agent-manager` and `ubiq-host` already depend on `ureq`, so an HTTPS probe needs no new dependency.

## 2. What each harness can actually be asked

This is the part that cannot be designed around — the mechanisms are not alike, and three of the
five have nothing to offer.

| Harness | Mechanism | Fields | Confidence | Needs a process |
|---|---|---|---|---|
| Claude Code | `GET https://api.anthropic.com/api/oauth/usage`, bearer = the OAuth access token, `anthropic-beta: oauth-2025-04-20` | five-hour and seven-day utilisation, reset times | **Likely** — unofficial, community tools use it, known to 429 | No |
| Claude Code | `RateLimitUpdate` already pushed by the running bridge | as above, plus `status`, `overage_*` | **Verified** — landed | Yes, a live turn |
| Claude Code | Keychain `Claude Code-credentials` | `subscriptionType` (`pro`), `rateLimitTier` | **Verified** — read on this machine | No |
| Codex | `codex app-server` JSON-RPC, method `account/rateLimits/read` | `primary` / `secondary` windows: `usedPercent`, `windowDurationMins`, `resetsAt` | **Likely** | Yes, an app-server |
| Copilot | Nothing queryable found. Docs give the static plan ceiling only (50 / 300 / 1500 premium requests a month, resetting on the 1st UTC) | plan ceiling | **Unknown** | — |
| opencode | Nothing. Upstream issues (#44189, #10448) ask for a Zen balance endpoint and say the dashboard is the only way | — | **Verified absent** | — |
| Gemini | Nothing live. Fixed daily caps only (250/day API-key free tier, 1000/day Code Assist) | static ceiling | **Verified** (docs) | — |

Two consequences worth stating before any code is written. **Claude is the only harness pollable in
the background** — token from the keychain, one HTTPS call, no process. **Codex is the only one
where an existing Ubiq mechanism already fits**: `CodexBridge::request` (`io/codex.rs:257`) is a
JSON-RPC request/response side channel with id correlation, so `account/rateLimits/read` is one more
`self.request(...)` and no new plumbing. And **three harnesses will answer "not said" forever**,
which the design has to treat as a first-class answer rather than as a failure.

## 3. The shape — an account fact, and a list of gauges

Two decisions carry the whole design.

**Quota is keyed by account, not by conversation.** `AccountId` (or the account name accounts are
already keyed by) is the key; the harness is a field on the snapshot, not part of the key.

**A snapshot is a list of gauges, not a struct of every provider's fields.** A union struct
(`five_hour_pct`, `seven_day_pct`, `premium_requests_used`, `credit_balance`, `daily_requests`…)
grows a field per provider and is mostly `None` on every read. A list is what the harnesses
actually differ in — Claude has two windows, Codex has two differently-sized windows, Copilot has a
monthly request count, opencode Zen would have a currency balance:

```rust
pub struct QuotaSnapshot {
    pub account: String,
    pub harness: String,
    /// What the provider calls the plan — "pro", "Copilot Business". None where it does not say.
    pub plan: Option<String>,
    pub gauges: Vec<QuotaGauge>,
    /// When this was read, so the interface can say how stale it is rather than implying "now".
    pub as_of: i64,
}

pub struct QuotaGauge {
    /// The provider's own name for the window. "5 hours", "Week", "Premium requests".
    pub label: String,
    pub reading: QuotaReading,
    /// Unix seconds. None where the provider states no reset.
    pub resets_at: Option<i64>,
    /// One line under the gauge, verbatim from the provider where it gives one.
    pub detail: Option<String>,
}

pub enum QuotaReading {
    /// A fraction of a window the provider itself computed. 0..=100.
    Window { used_pct: u8 },
    /// A count against a stated ceiling — Copilot's premium requests.
    Count { used: u64, limit: Option<u64> },
    /// Money left, minor units plus ISO 4217, matching `io::model::Cost`.
    Credit { remaining: i64, currency: String },
}
```

`QuotaReading` is the extension point: a new provider adds a variant only when it genuinely reads
differently, and every existing gauge keeps drawing.

## 4. agent-manager — the capability

The library already owns everything about how a harness is configured and authenticated, and this
is more of that. Two additions, both following conventions the trait already has:

- **`IoSupport` gains `quota: QuotaSource`** (`harness/mod.rs:197`), an enum of `None`, `Push`
  (arrives on the stream, as Claude's does today), `Probe` (can be asked with no process) or
  `Bridge` (can be asked over a running structured bridge). The host reads this to decide whether
  to offer the menu entry at all — the same way `AgentTypeInfo.chat` is derived from
  `io_support().structured` (`ubiq-host/src/agent.rs:284`).
- **`Harness::quota(&self, account: &Account) -> Result<QuotaSnapshot>`**, defaulting to an
  `Err("does not report usage limits")`, exactly as `structured_bridge` (`harness/mod.rs:801`) and
  `discover_models` do.

The implementations:

- **Claude** reads the OAuth token through the path `account.rs:481` already uses to read the
  keychain, then one `ureq` GET. **It never returns the token, never logs it and never puts it on
  the bus** — the account invariant (`account.rs:1`) is that accounts carry references, and the
  snapshot carries percentages.
- **Codex** goes through `CodexBridge::request("account/rateLimits/read", …)` when a bridge is
  live; `QuotaSource::Bridge` says the answer needs one, and the host does not spawn an app-server
  just to ask.
- **Copilot, opencode, Grok, Gemini** return the default error, and the interface draws the em dash
  the stats screen's rule already mandates: *a figure nobody has stated is an em dash, never a
  zero.* Where only a static plan ceiling is known, it belongs in the harness's `_docs/` page as
  prose, not in a gauge with an invented numerator.

Claude's existing push stays exactly as it is. It is free, it is live, and it is more current than
any poll — the host merges it into the same cached snapshot, so an account with a running agent has
a fresher reading than one without and neither needs a second code path in the interface.

## 5. Host — the prober and the five-minute poll

A `quota` worker thread on the `files`/`search` pattern (`files/mod.rs:511` — `start()` returns a
handle the coordinator holds, each job carries a `Mailbox` so the answer routes back to the client
that asked). It does two things:

**On demand.** A `QueryQuota` message becomes a job; the answer comes back as `QuotaRead`,
addressed to the asking client.

**On a five-minute tick.** The worker naps its own interval, `nap`-style, and on each wake probes
**only the accounts that have at least one live agent** — `conversation_owners`
(`coordinator.rs:189`) or `Work::live` (`work/mod.rs:40`) is the set. Three reasons the poll is
scoped that way rather than sweeping every configured account: an idle account's quota is not
changing, the Claude endpoint 429s under load, and probing an account the user is not using is a
network call they did not ask for.

Cached in memory on the coordinator, keyed by account. **Not persisted.** `usage.db`'s own module
doc draws the line — it is a record because nobody can ask a harness what it spent last Tuesday;
quota is the opposite, re-derivable by asking again, so it is a cache, and a cache this small and
this short-lived does not need a file. A restart re-probes.

## 6. Transport — three messages

Modelled on the file family's request/reply convention (verb-noun out, noun-participle back, every
reply echoing the request's identifying fields):

```rust
// ── quota family: UI → host ──
QueryQuota { account: String, harness: String },
// ── quota family: host → UI ──
QuotaRead { account: String, harness: String, snapshot: Option<QuotaSnapshot>, error: Option<String> },
QuotaChanged { account: String, harness: String, snapshot: QuotaSnapshot },
```

`QuotaChanged` is the unsolicited broadcast, `To::Everyone`, on the precedent of
`ProjectFilesChanged` (`messages.rs:833`) — sent when a poll or a pushed `RateLimitUpdate` changes
a cached snapshot, because every window showing that account is looking at the same fact.

`error` is a string the user reads, not a code. "Claude is rate-limiting the usage endpoint" is a
different thing on screen from "Copilot does not report usage limits", and both are different from
a stale reading, which is what `as_of` is for.

## 7. Interface — one popup, opened from two places

**One draw function, two entry points.** This follows the rule the lifecycle menu already obeys:
chat and the agents column both call `lifecycle_menu` in `ui/conversation/mod.rs:354` rather than
each drawing their own.

- **The agent's three-dots menu.** A `ContextItem` — *Usage & limits* — added to `LIFECYCLE_ROWS`,
  disabled (not hidden) when the harness reports `QuotaSource::None`, so the answer "this harness
  does not say" is discoverable instead of invisible. Opens the panel anchored to the button, the
  `context_panel` shape (`ui/kit/menu.rs:497`) the menu itself uses.
- **Settings → Accounts.** Each account row (`ui/settings.rs:1180`) gets the same panel behind a
  chevron, which is the surface that works when nothing is running.

Inside, one row per gauge: label, `kit::controls::meter(fraction, colour)` (`controls.rs:315`),
the reading as text, and the reset as a relative time. Colour by threshold from the status tokens —
`success` under 75%, `warning` from 75, `danger` from 90 — with the `_soft` variants for the track.
A `Count` gauge with no `limit` draws no bar, because a meter without a denominator is the exact
thing the Control screen refuses to draw. Foot of the panel: the plan, `as_of` as "read 2 min ago",
and a manual refresh.

## 8. The alert

The bell, not a toast — Ubiq has no toast and this does not introduce one.

`Family::Agents`, `actor` = the account, `category` = `"quota"`. That addressing matters more than
it looks: mute rules match on the origin (`features/notifications.md`), so the user can silence
quota warnings for one account without silencing the account's other notifications, which is the
narrowest-scope rule the notification design is built around.

Thresholds at **75% (`Warning`)** and **90% (`Error`)**, per gauge, **edge-triggered once per
window** — armed again only when `resets_at` passes or the reading falls. A five-minute poll that
re-warns every five minutes for an hour is a poll the user turns off, and then the one warning that
mattered is off too. The notification carries a link to the account's settings row.

## 9. Rules this must not break

1. **No credential material leaves agent-manager.** The probe reads the token inside the library and
   returns percentages. Nothing token-shaped is on the bus, in a log or in a snapshot.
2. **No probe without a live agent**, except the one the user asked for by opening the panel.
3. **A provider that says nothing produces an em dash, never a zero** (`features/stats.md`).
4. **No meter without a denominator the provider stated** (`features/stats.md`).
5. **The percentage does not return to the chat header.** It lives behind a menu entry
   (`ui/conversation/mod.rs:1796`).
6. **Ubiq names no provider endpoint.** The URL, the beta header and the keychain service are
   agent-manager's, like every other harness fact.
7. **An unofficial endpoint is treated as best-effort.** A 429 or a schema change degrades to "not
   read", never to a wrong number and never to a broken panel.

## 10. Staging

| Phase | What lands | Buys |
|---|---|---|
| 1 | `QuotaSnapshot`/`QuotaGauge` in the library; Claude's existing `RateLimitUpdate` re-expressed as gauges and cached per account in the host; the panel and its two entry points; `QueryQuota`/`QuotaRead` | The whole interface, on data already arriving. No network call, no new thread |
| 2 | `Harness::quota`, the Claude probe, the `quota` worker, on-demand only | An answer for an account with nothing running |
| 3 | The five-minute poll, `QuotaChanged`, the threshold alert | The feature as asked for |
| 4 | Codex via `account/rateLimits/read` on a live bridge | The second harness that can answer |

Phase 1 is worth landing alone: it is the only phase with no external dependency, and it turns a
fact Ubiq already receives and discards into something on screen.

## 11. To file

- A `Dnn` for **quota is an account fact held as a list of gauges, cached in memory, never
  persisted** — the counterpart to `D78`, which explains why spend *is* a database.
- A `Dnn` for **the poll is scoped to accounts with a live agent**, and why the interval is five
  minutes.
- `Gnn` rows for the three harnesses that report nothing (Copilot, opencode, Gemini), each naming
  the upstream gap rather than an Ubiq omission — the same shape as `G161`, which already records
  that Codex, Copilot and opencode report no usage at all.
- A `Gnn` for the Anthropic usage endpoint being unofficial and rate-limited.
- Open question: **whether the panel should show spend beside quota.** `usage.db` has the account's
  token history and the panel has the account's remaining window, and putting them on one surface
  is either the obvious synthesis or a second screen inside a popup. Deferred, not decided.
