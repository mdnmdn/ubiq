---
id: philosophy-your-machine
title: Your machine, your credentials
summary: Where your work and your sign-ins live, and why Ubiq keeps references to your credentials rather than the credentials.
keywords: [local, local-first, credentials, privacy, secrets, keychain, token, password, api key]
order: 40
status: current
related: [agents-accounts, remote-overview, philosophy-what-ubiq-is-not]
---

## What it is

Ubiq is for one person working on their own machine. Your projects, your agents and the window that
shows them run on your computer, not on a hosted service. When you want an agent on another machine
of yours, you [attach it as a remote host](remote-overview), and its projects appear in the same
list as your local ones.

An [account](agents-accounts) is a named identity a harness runs under. It points at where your
credentials are kept and never holds the credentials themselves. You can register several accounts
for one harness and choose which one each run uses.

Where Ubiq does need a secret, it hands it to a store made for secrets:

- **Connector tokens** are kept in your operating system's keychain.
- **SSH passwords and keys** for remote hosts go to the operating system's credential store.
- **Database passwords** are stored sealed and never shown again.

## Why it works this way

Ubiq is not a secrets manager, and it does not try to be one. Your keychain and your harnesses
already keep your sign-ins; a copy inside Ubiq would be one more place for them to leak from.
Holding only a reference means switching an agent to another account never moves a key around.

The same care covers your harness settings. Each run gets its own throwaway configuration, so the
choices you make for one agent in Ubiq never rewrite the settings you rely on outside it.

[Accounts and profiles for several clients](guides-accounts-and-profiles) shows how to keep the
identities of different clients apart.
