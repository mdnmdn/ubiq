---
id: agents-accounts
title: Accounts
summary: A named identity a harness runs under — Ubiq stores a reference to credentials, never the material.
keywords: [account, login, credentials, identity]
order: 20
status: current
related: [agents-starting, agents-permissions]
---

## What an account is

An account is a named identity a harness runs under — the login a harness like Claude Code or Codex
uses when it talks to its own service. Ubiq lets you keep more than one, and pick which one a given
agent starts under.

## Credentials stay out of Ubiq

Accounts carry a **reference** to a credential, never the credential material itself. Ubiq never
holds a password or a raw token on your behalf; it points at where the harness's own login already
lives and lets that harness authenticate itself the same way it would outside Ubiq.

## Picking one

The new-agent menu lets you choose an account per run, so two agents of the same harness can run
under two different identities side by side — useful for comparing a personal and a work account, or
for keeping an experiment's usage separate from your own.
