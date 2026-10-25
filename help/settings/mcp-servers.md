---
id: settings-mcp-servers
title: MCP servers
summary: The MCP servers section, in Settings and in Project settings — tool providers an agent can be given, and how to add one.
keywords: [mcp, mcp server, tools, registry, stdio, http, sse, command, url, headers, environment]
order: 46
status: current
related: [settings, agents-definitions, settings-skills, settings-connectors]
---

An MCP server is a tool provider a harness can call: a program the harness starts, or a URL it
connects to. This section is the list Ubiq knows about; an [agent definition](agents-definitions) or
the [new agent dialog](agents-new-agent) ticks the servers an agent should be given. It is not a
[connector](settings-connectors), which is you signed in at a code host.

**MCP servers** appears twice. In [Settings](settings) it is the application's list. In
[Project settings](settings-project) it is a list only agents started in that project can be given. A
server there with the same id as an application one takes its place.

## The parts

| Part | What it does |
|---|---|
| **Add MCP server** | Opens the form below |
| **Search registry…** | Opens **Search the MCP registry**: type and press **Search**, then **Use** for one result, or **Add all** for the ones you picked; **Load more** shows further results |
| A server row | Its id, a badge for its kind, a one-line summary, **Edit** and **Remove** |
| **From the application** | Project settings only. The application's servers, read-only; one this project replaces is marked as shadowed |
| **Paste a configuration** | In the form: paste a server's configuration and **Fill from paste** fills the fields; some entries ask for **Needs a value** |
| **Id** | What a definition or a start ticks the server by: letters, digits, dots, dashes and underscores |
| **Kind** | **Local** starts a command on the machine where the agent runs; **Remote** connects to a URL, **over** **HTTP** or **SSE** |
| **Command**, **Arguments**, **Environment** | Local: the program, one argument per line, and one `KEY=VALUE` per line |
| **URL**, **Headers** | Remote: the endpoint, and one `Name: value` per line, which is where a token goes |
| **Description** | Optional; shown beside the server in the pickers |

## What you can do here

- Give a server to an agent: tick it under **MCPs** in the [agent definition](agents-definitions) or the
  [new agent dialog](agents-new-agent).
- Keep a server to one project: add it in that project's **MCP servers** instead of in Settings.

## When something goes wrong

- **Save is dimmed.** The form is not complete: a local server needs an id and a command, a remote one
  an id and a URL.
- **A registry result says No way to run it is listed.** It cannot be added from there; add it by hand
  with **Add MCP server**.
- **The server is listed but an agent does not have it.** Ticking is separate from listing; tick it
  for that agent.
- **A red banner sits above the list.** Ubiq reports why the last change was refused; correct it and
  try again.
