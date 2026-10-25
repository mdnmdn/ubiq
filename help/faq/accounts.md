---
id: faq-accounts
title: Account questions
summary: What people ask about accounts, agent definitions and sign-ins.
keywords: [account, agent definition, login, credentials, connector, github, token]
order: 30
related: [agents-accounts, settings-connectors]
---

## What is the difference between an account and a connector?

An account is the login a harness such as Claude Code or Codex runs under. A connector is you signed in at a code host such as GitHub or GitLab, which Ubiq uses to list and clone your repositories and to reach issues and pull requests. They are set up in different places: [Accounts](agents-accounts) under **Harnesses**, and [Connectors](settings-connectors) under their own section.

## Does Ubiq store my harness password or token?

No. An account points at where the harness keeps its own login, and Ubiq never holds a password or a token for it. An agent that names no account runs under your own home configuration. See [Accounts](agents-accounts).

## How do I run two agents as two different identities?

Add an account for each under **Settings**, then **Harnesses**, with **Add harness**. In the new agent dialog, pick the harness and the account on the **Harness** tab. The two agents then run side by side with their usage kept apart. See [Accounts and agent definitions for several clients](guides-accounts-and-definitions).

## What is an agent definition, and how is it different from an account?

An account says who an agent runs as; an agent definition says how it runs: harness, model, level, permission mode and what to add to the run. A definition can name an account, so one start picks both. See [Agent definitions](agents-definitions).

## My sign-in expired. What do I do?

For a harness, open its row under **Identities** in **Settings**, then **Harnesses**, and press **Re-authenticate**. For a connector whose chip reads `expired`, disconnect it and connect again. See [Sign in a connector](how-to-sign-in-a-connector).

## Can I use my company's own GitLab?

Yes. In the connect dialog, type its address under **Instance** and choose **Paste a token**. Or register an application there first and use **Open a browser**. See [Connectors](settings-connectors).

## Can I have two GitHub identities?

Yes. Several connectors for one provider are ordinary, such as a work identity and a personal one; give each its own **Name**. See [Sign in a connector](how-to-sign-in-a-connector).
