---
id: agents-accounts
title: Accounts
summary: Named identities a harness runs under, how to add or sign one out, and how accounts differ from connectors.
keywords: [account, login, credentials, identity, sign in, sign out, re-authenticate, usage, plan]
order: 20
status: current
related: [agents-new-agent, agents-definitions, settings-harnesses, settings-connectors]
---

An account is a named identity a harness runs under: the login Claude Code or Codex uses when it talks to its
own service. You can keep several, and choose one per agent in the [new agent dialog](agents-new-agent), so two
agents of one harness can run side by side as, say, your personal and your work identity. An account points at
where the harness keeps its own login; Ubiq never holds a password or a token for it. An agent that names no
account runs under your own home configuration. [Account questions](faq-accounts) answers the common
questions.

Accounts live in **Settings**, then **Harnesses**, under **Identities**. **Installed**, above it, lists the
harnesses this machine has.

## The parts

| Part | What it does |
|---|---|
| **Add harness** | Opens the sign-in dialog: choose a harness, name the identity, and sign in |
| An account block | The account's name, with **Rename** and **Delete** |
| A harness row under it | The harness's name and its login status, with **Check**, **Re-authenticate** and **Sign out** |
| **Check** | Reads the login's own expiry and writes it in place: `valid`, `expires`, `expired`, `unknown` or `missing` |
| Plan readout | Where the provider states limits: one row per window with a meter, the reading, when it resets and how old it is, and **Refresh** |
| **Command** on an installed harness | Sets the command Ubiq starts that harness with, with a **Check** that tries it without saving |

**Delete** is the one action with nothing left behind, so its confirm says so.

## What you can do here

- Add an account: **Add harness**, pick the harness, name the identity. The harness's own sign-in runs in a
  terminal inside the dialog, the same screen it shows in a shell. A web address it prints appears as a button
  under the terminal, with a small button beside it that copies it. When the sign-in finishes cleanly, the
  account exists and agents that name it start signed in.
- Sign in again under the same account with **Re-authenticate**, which skips the harness choice.
- Remove one harness's login from an account with **Sign out**, after a confirm; the account and its other
  harnesses stay.
- Pick an account when starting an agent, or save it in an [agent definition](agents-definitions). For several clients, see
  [accounts and agent definitions for several clients](guides-accounts-and-definitions).
- Check how much of a plan is left without starting anything: the readout sits under the harness row and
  **Refresh** asks the provider again.

Accounts are not connectors. A connector is a signed-in identity at a code host such as GitHub or GitLab, used
for work like cloning. It is set up in [Settings](settings-connectors); the steps are in
[sign in a connector](how-to-sign-in-a-connector).

## When something goes wrong

- **Closing the dialog during sign-in.** The sign-in is abandoned and costs nothing; start again.
- **A rename or delete is refused.** A banner over the section says why, for example that the name is taken. It
  clears when you open another dialog or dismiss it.
- **Check says `missing` on a machine where you are signed in.** On macOS the login may be in the keychain,
  which this reading cannot see. The agent still works.
- **A readout says the harness did not answer.** The sentence is the provider's; press **Refresh** to ask again.
  A harness that states no limit shows no meter, rather than an empty one.
- **A harness is not in the list.** Only harnesses that run from a config home of their own can hold an account,
  and the ACP versions of Claude Code and Codex appear once their **Enable … ACP** switch is on in Settings.
