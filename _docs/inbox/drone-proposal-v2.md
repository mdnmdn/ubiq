---
id: inbox-drone-runtime
title: Proposal — the drone, a portable runtime Ubiq places on another machine
kind: proposal
status: proposal
summary: One small executable, built twice, that Ubiq puts where it is not running — a relay build that gives a remote machine's terminal, files and machine facts over SSH with no agent knowledge at all, and a runtime build that adds an embedded agent-manager to compose and confine harness runs inside WSL or a container; one duplex byte stream, one contract with a capability handshake, a signed manifest per triple, no socket by default, no state, and no reconnect.
read_when: you are deciding how a terminal, a file or a harness reaches a machine Ubiq is not running on, what "drone" means in this tree, which carrier a remote link rides, how a foreign-triple binary gets fetched and verified, or why confinement in a pane is macOS-only
updated: 2026-09-11
depends_on: [tech-architecture, tech-transport, tech-agent-manager, tech-structure, tech-decisions, feat-panes, inbox-isolation]
---

# Proposal — the drone, a portable runtime Ubiq places on another machine

Ubiq reaches only the machine that draws the window. Two quite different things want fixing, and
conflating them is how this design goes wrong:

**Remote access.** A terminal, a file tree and a machine's facts on a host across a network. No
agents, no harnesses, no composition — the user opens a shell somewhere else and edits something
there. The far machine may belong to someone else, may have other users on it, and is reached over a
network that has to be assumed hostile.

**A local boundary.** A Windows user whose toolchains, repositories, shells and agent harnesses all
live inside a WSL distribution, one kernel boundary away — and, later, the same shape for a
container. Their Claude Code is a Linux binary with a Linux config directory and a Linux credential
store, and no amount of pseudo-terminal work on the Windows side composes a run for it. Here
`crates/agent-manager` must be on the far side, because composing a run *is* reading that machine's
files and writing that machine's run directory. There is no network and no third party: it is the
user's own machine, one `wsl.exe` away.

This proposes the **drone** as one executable that serves both — **one crate, one contract, two
builds**, differing by a feature flag that decides whether `agent-manager` is linked. §2 is that
split and the reason it is a flag rather than two projects. Everything else — the carrier, the
frame, the manifest, the no-reconnect rule — is shared, because both cases are the same problem:
one duplex byte stream to a process somewhere Ubiq is not.

There is a shelved proposal of the same name. §1 says what it settled, what this keeps, and what the
brief behind this document overturns.

## 1. Where this starts

[`backlog/remote-drone-proposal.md`](./backlog/remote-drone-proposal.md) designed a drone as *"a