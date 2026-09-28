---
id: wip-agent-login-note
title: Agent login note
kind: wip
status: draft
summary: Closed by D193 and D194. Why a conversation failed silently when the run directory had no Claude Code login, and the two ways considered for wiring account selection into the RunSpec — both answered by removing the capture and keying the config home by account.
read_when: you are reading how account selection came to be keyed the way it is, and want the question D194 answers
updated: 2026-09-27
verified: 2026-09-27
code_anchors: [crates/ubiq-host/src/agent.rs]
depends_on: [wip-agent-setup]
review_cycle: monthly
---

> **Closed 2026-09-27 by `D193` and `D194`.** The question this note asks — how a captured login
> reaches a run — has no answer in the tree any more, because nothing captures a login. A harness
> logs in once into a fixed config home keyed by the account (`D194`), the harness owns it from
> then on, and `am account login` signs that home in rather than copying anything out of it;
> `am profile login`, `seed_login`, `harvest_login` and `Claude::ambient_login` are all gone.
> Read `D193` and `D194` in [`../tech/decisions.md`](../tech/decisions.md), and
> [`../tech/agent-manager.md`](../tech/agent-manager.md) for the boundary. Everything below is the
> record of how it stood on 2026-09-12 and is kept only for that.

One command tells you for sure:

ls -l ~/.claude/.credentials.json

Missing → that's it.

The fix

am account login exists for exactly this. It runs Claude's own OAuth flow with the home relocated so the Keychain is unreachable, which forces Claude to write the plaintext credential, and captures it:

just am account login claude-code

Then the honest part: Ubiq does not select an account yet. crates/ubiq-host/src/agent.rs builds a RunSpec naming a harness and a folder and nothing else — that is G89, and P5 in the plan is where it gets a real answer. So after that login you would still need the host to pass the account through. Two small pieces, and I'd rather you choose whether to do them now than have me guess.

Meanwhile I made the failure say what it is. Starting a conversation whose run directory received no login now logs a warning under Harness naming the cause, instead of leaving you to read "Not logged in" inside the transcript as though the agent had said it. That warning only started firing on 2026-09-12: `Agents::has_login` counted any seed file, and the identity companion `.claude.json` always lands, so it answered "logged in" on exactly the machines that were not. It now counts only a `SeedFile::credential` — as does the seeding itself, which is why the Keychain tier never ran. See `_docs/tech/agent-manager.md`.

Which way do you want to go?

- Wire account selection now — the host reads the library's configured account (settings default, or one named per agent type) and passes it on the RunSpec. This is the beginning of P5 and it is the correct fix.
- A quicker stopgap — the host also accepts a credential straight from the environment (ANTHROPIC_API_KEY / CLAUDE_CODE_OAUTH_TOKEN) when one is set, so you can export a key and keep going while P5 waits.