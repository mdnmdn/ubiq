---
id: inbox-graph-harness-ideas
title: Notes — an agent graph, workspaces and a PM orchestrator
kind: proposal
status: proposal
summary: Raw brainstorm notes for a multi-agent graph — one-to-many running workspaces per project, a PM-like orchestrator agents talk to about starting tasks, three ways a task can run (direct agent, chained workflow, or a coordinator spawning sub-agents), a graphical view of the agent graph with per-agent status and read-only chat windows, and cross-harness agents identified and connected through injected MCP. Expanded and settled in `agent-graph-final.md`.
read_when: you want the original brainstorm behind the agent graph proposal, before reading the settled design
updated: 2026-09-03
depends_on: [inbox-agent-graph-final]
---

Expanded and settled in [`agent-graph-final.md`](../backlog/agent-graph-final.md).
The working area the notes call a "session" is a **workspace** there, so "session" can stay
the harness conversation. A workspace is for a task, or just to code. The PM starts as an
analyst that refines the brief.

brainstorm another part of this product:

for a project we could have 1-n running sessions each session could represent a working area with a goal (so the main repo or a worktree, maybe with a specific branch)

we could work having a project agent orchestrator (like a PM) with which we could talk about starting new tasks, each task could be executed in the same session from different parallel subagents, according to the task we could have 3 specific approaches:
- talk directlyto a generic agent that start working
- start a workflow with chain of agents that each one performs a task and then pass to a different one
- start an activitity coordinator the spawn different agents with different responsibility and coordinates them

A PM could work also with different sessions spawning different sessions

all the high level tasks could be tracked in a specific area, and also the agent tasks (like sub tasks) and also work with the documentation.

I want a graphical interface in order to represent and follow these scenario, in particular the agents graphs

for each agent I want to see who it is, which is role if has a session (PM could have none or always the main) the releation to other subagents,which harness (claude...),  which task and sessions the subagents are working with and what is doing (idle , doing something, thinking, tools, waiting for input, ended, error, ...)

And I want to have the possibility to select an agent and open the chat window, also only for read. I want to have the possibility to open also more chat together

Each agent could be an agent native subagents (eg claude or grok subagents) or an agent of a potential different harness that could communicate
via MCP injected, each agent will have a precise identity injected in the initial prompt and specific MCP for cross agent communication
 
 
