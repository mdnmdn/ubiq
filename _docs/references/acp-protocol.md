---
id: ref-acp-protocol
title: Agent Client Protocol (ACP)
kind: reference
status: current
summary: The JSON-RPC wire reference for the Agent Client Protocol — initialisation, session setup and loading, the streaming update vocabulary, tool-call reporting, permission prompts, the client-side filesystem and terminal callbacks, and the Rust SDK Ubiq reads.
read_when: you are wiring a harness that speaks ACP, adding a session update variant, or checking what the protocol says before assuming it
updated: 2026-09-09
verified: 2026-09-03
---

# Agent Client Protocol (ACP) — Wire Reference

## What this is

The **Agent Client Protocol (ACP)** is a JSON-RPC 2.0 protocol that lets a *Client* (an editor, IDE,
or any UI — Zed, Ubiq, Neovim plugins, Emacs packages) drive an *Agent* (an AI coding harness —
Gemini CLI, Claude Code adapters, custom agents) running as a subprocess. It standardises session
setup, streaming model output, tool-call reporting, permission prompts, and the callbacks an agent
makes back into the editor for file reads/writes and command execution.

It was created by **Zed Industries** and has since moved to its own GitHub organisation:
**`github.com/agentclientprotocol/agent-client-protocol`**. The old `zed-industries/agent-client-protocol`
URLs still resolve by redirect. The language SDKs were split into separate repositories
(`agentclientprotocol/rust-sdk`, `agentclientprotocol/typescript-sdk`).

This is **not** IBM/BeeAI's "Agent Communication Protocol", and it is **not** MCP. ACP sits *above*
MCP: an ACP agent may itself connect to MCP servers, and `session/new` carries the MCP server list.

**Checked:** 2026-09-03, against `main`.
**Protocol version documented:** **v1** (the current stable version).
**Schema artifact version:** JSON-Schema marker `1.21.0` (`schema/v1/Cargo.toml`), 170 definitions.

### Canonical raw URLs

```
https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/main/schema/v1/meta.json
https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/main/schema/v1/schema.json
https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/main/schema/v1/schema.unstable.json
https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/main/schema/v1/meta.unstable.json
https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/main/schema/v2/meta.json
https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/main/schema/v2/schema.json
https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/main/agent-client-protocol-schema/src/version.rs
https://raw.githubusercontent.com/agentclientprotocol/agent-client-protocol/main/docs/protocol/v1/<page>.mdx
```

Note the path shape: there is **no** top-level `schema/schema.json` or `schema/meta.json`. The
schemas are versioned under `schema/v1/` and `schema/v2/`; requesting the unversioned paths returns
a 404. Rendered docs live at `https://agentclientprotocol.com/protocol/v1/...`.

`schema/v1/meta.json` is the single most useful file for a wire implementation: it is the complete
method-name registry, ~40 lines, split by side.

### Read this first — three ways earlier research in this repo is stale

1. **`session/set_mode` is deprecated and scheduled for removal**, superseded by
   `session/set_config_option`. This is now stated normatively in the spec (two separate pages), not
   inferred. It is already gone from the v2 draft's method set. See §10.
2. **The v1 method set is much larger than the commonly-cited one.** Beyond
   `initialize` / `authenticate` / `session/new` / `session/load` / `session/prompt` / `session/cancel`,
   stable v1 also has `session/list`, `session/delete`, `session/resume`, `session/close`, `logout`,
   `session/set_config_option`, `elicitation/create`, `elicitation/complete`, and the protocol-level
   `$/cancel_request`. `SessionUpdate` has **eleven** discriminants, not eight.
3. **A protocol v2 draft exists**, published 2026-07-20. Stable is still v1, but v2 removes
   `session/load` and `session/set_mode`, renames `authenticate`/`logout` to `auth/login`/`auth/logout`,
   drops `fs/*` and `terminal/*` from the client method set, overhauls diffs, and decouples
   `session/update` from the prompt turn. See §13.

### On `refs/multica`

**`refs/multica` contains no ACP code whatsoever.** A recursive grep across `*.ts`, `*.tsx`, `*.rs`,
`*.json` and `*.md` (excluding `node_modules`) for `agent-client-protocol`, `agentclientprotocol`,
`sessionUpdate` and `agent_thought_chunk` produced exactly one hit —
`packages/core/realtime/use-realtime-sync.ts` — which is an unrelated false positive on the word
`sessionUpdate` in that project's own realtime sync code. There is no ACP client, no type
definitions, and no vendored schema in that checkout. `~/.cargo/registry/src/*/agent-client-protocol*`
does not exist either.

**Therefore: any claim elsewhere in this repository that ACP message shapes were "confirmed against
multica's clients" is unsupported.** Every shape in this document comes from the upstream schema and
docs, cited per section. There is no local source to agree or disagree with.

---

## 1. Framing and transport

*Source: `docs/protocol/v1/transports.mdx`, `docs/protocol/v1/overview.mdx`*

- Encoding is **JSON-RPC 2.0**. Messages **MUST** be UTF-8.
- The spec currently defines two transports: **stdio**, and *Streamable HTTP* (a draft proposal, not
  yet specified). Agents and clients **SHOULD** support stdio whenever possible. Custom transports
  are explicitly permitted provided they preserve JSON-RPC framing and the lifecycle rules.

### stdio framing rules (normative)

- The **client launches the agent as a subprocess**.
- The **agent** reads JSON-RPC messages from its own `stdin` and writes them to its own `stdout`.
  From the client's side: the client writes to the child's stdin and reads the child's stdout.
- Each message is an individual JSON-RPC request, notification, or response.
- **Messages are delimited by newlines (`\n`) and MUST NOT contain embedded newlines.** There is no
  `Content-Length` header framing — this is *not* LSP-style framing. Serialise compactly, one line
  per message.
- The agent **MAY** write UTF-8 strings to `stderr` for logging. Clients **MAY** capture, forward, or
  ignore it.
- The agent **MUST NOT** write anything to `stdout` that is not a valid ACP message.
- The client **MUST NOT** write anything to the agent's `stdin` that is not a valid ACP message.
- Shutdown: the client closes stdin and terminates the subprocess.

### Bidirectionality

One connection; **both peers are simultaneously JSON-RPC servers and clients**. The agent issues
requests *back* to the client over the same byte stream — `session/request_permission`,
`fs/read_text_file`, `fs/write_text_file`, the `terminal/*` family, `elicitation/create` — using its
own independent `id` space. There is no second channel, no reverse connection, no separate socket.

A conforming implementation therefore needs, on each side: an outbound request table keyed by the
ids *it* issued, and a dispatcher for inbound requests, notifications, and responses, all
demultiplexed from the same line-oriented stream.

### Envelope

Standard JSON-RPC 2.0: `jsonrpc`, `id`, `method`, `params`, `result`, `error`.

`RequestId` (`schema/v1/schema.json` → `RequestId`) is an untagged union of:
- `null` — the JSON-RPC null request id
- `integer` — a numeric id
- `string` — a string id

The spec notes ids SHOULD normally not be null and numbers SHOULD NOT have fractional parts.

### Casing convention (normative)

From `docs/protocol/v1/overview.mdx` §Conventions:

> Unless explicitly defined otherwise in the schema, ACP-defined JSON object property keys use
> `camelCase`. String values carried by discriminator fields use `snake_case`. The JSON-RPC envelope
> fields (`jsonrpc`, `id`, `method`, `params`, `result`, and `error`) follow the JSON-RPC 2.0
> specification.

So: `sessionId`, `toolCallId`, `stopReason`, `outputByteLimit` — but
`"sessionUpdate": "agent_message_chunk"`, `"kind": "allow_once"`, `"stopReason": "end_turn"`,
`"status": "in_progress"`.

Method names themselves are `snake_case` after a `/`-separated namespace:
`fs/read_text_file`, `terminal/wait_for_exit`, `session/set_config_option`.

The reference Rust source uses snake_case field identifiers with `#[serde(rename_all = "camelCase")]`.
**Trust the JSON Schema, not the Rust identifiers** — reading `session_id` in `agent.rs` and
concluding the wire key is `session_id` is the classic mistake here.

### Global argument rules

From `docs/protocol/v1/overview.mdx`:

- **All file paths in the protocol MUST be absolute.**
- **Line numbers are 1-based.**

---

## 2. Multiplexing — one connection, N sessions

*Source: `docs/protocol/v1/session-setup.mdx`, `schema/v1/schema.json`*

**Yes: one ACP connection hosts an arbitrary number of concurrent sessions, and `sessionId` is
present on every session-scoped message.** This is the design, not an accident.

From `session-setup.mdx`:

> Sessions represent a specific conversation or thread between the Client and the Agent. Each
> session maintains its own context, conversation history, and state, **allowing multiple
> independent interactions with the same Agent.**

`SessionId` is a plain `string` newtype (`schema/v1/schema.json` → `SessionId`). There is no
per-session channel, no session-scoped sub-connection, no per-session handshake. **Routing is purely
by the `sessionId` field on the message.**

### Exhaustive list: which messages carry `sessionId`

**Client → Agent, `sessionId` required:**

| Message | Type |
|---|---|
| `session/prompt` | `PromptRequest.sessionId` |
| `session/cancel` (notification) | `CancelNotification.sessionId` |
| `session/load` | `LoadSessionRequest.sessionId` |
| `session/resume` | `ResumeSessionRequest.sessionId` |
| `session/close` | `CloseSessionRequest.sessionId` |
| `session/delete` | `DeleteSessionRequest.sessionId` |
| `session/set_mode` | `SetSessionModeRequest.sessionId` |
| `session/set_config_option` | `SetSessionConfigOptionRequest.sessionId` |

**Agent → Client, `sessionId` required:**

| Message | Type |
|---|---|
| `session/update` (notification) | `SessionNotification.sessionId` |
| `session/request_permission` | `RequestPermissionRequest.sessionId` |
| `fs/read_text_file` | `ReadTextFileRequest.sessionId` |
| `fs/write_text_file` | `WriteTextFileRequest.sessionId` |
| `terminal/create` | `CreateTerminalRequest.sessionId` |
| `terminal/output` | `TerminalOutputRequest.sessionId` |
| `terminal/wait_for_exit` | `WaitForTerminalExitRequest.sessionId` |
| `terminal/kill` | `KillTerminalRequest.sessionId` |
| `terminal/release` | `ReleaseTerminalRequest.sessionId` |
| `elicitation/create` (session scope) | `ElicitationSessionScope.sessionId` |

**Session-scoped messages that do NOT carry it, and why:**

| Message | Reason |
|---|---|
| `session/new` | Creates the session; returns `sessionId` in the response |
| `session/list` | Connection-scoped; filters by `cwd`, not by session |
| `elicitation/create` (request scope) | Uses `ElicitationRequestScope.requestId` instead — for elicitations during auth/config *before* any session exists |

**Not session-scoped at all:** `initialize`, `authenticate`, `logout`, `elicitation/complete`
(keyed by `elicitationId`), `$/cancel_request` (keyed by `requestId`).

### v1 concurrency caveat

The v1 spec is written around a **prompt turn**: `session/prompt` is a long-lived request whose
response carries the `stopReason`, and the `prompt-turn.mdx` narrative places `session/update`
notifications inside that window.

- **Concurrent prompts on *different* sessions over one connection: supported, intended.** Nothing
  in the spec serialises across sessions; the isolation statement above is explicit.
- **Concurrent prompts on the *same* session: not described.** Assume unsupported in v1. The doc's
  closing line is *"Once a prompt turn completes, the Client may send another `session/prompt`"* —
  i.e. one turn at a time per session.
- `session/update` outside a turn was never *prohibited* in v1, but the v2 announcement records that
  it was "a common point of confusion for implementers" and that many agents behaved as if it were
  forbidden. Do not rely on out-of-turn updates from a v1 agent.

### What v2 relaxes

*Source: `docs/announcements/acp-v2-draft.mdx`*

v2 explicitly decouples work from the turn:

> Now `session/update` notifications can proceed freely at any point in the session, and a prompt
> response is the indication that the message was acknowledged by the agent, not the end of the
> turn. The agent will replay the user message where it inserted it, which also makes it easier for
> both replay and the potential for multiple clients observing the same session. The agent can
> indicate when it is "idle", or ready to receive new inputs.

This enables queueing, steering mid-turn, background work reporting, and **multiple clients
observing one session**. If Ubiq ever wants two panes viewing the same agent conversation, that is a
v2 affordance, not a v1 one.

---

## 3. Full method list

*Source: `schema/v1/meta.json` (stable), `schema/v1/meta.unstable.json` (draft), and the
`x-side` / `x-method` schema annotations in `schema/v1/schema.json`*

### 3a. Client → Agent

| Method | Kind | Params type | Required params | Optional params | Gate |
|---|---|---|---|---|---|
| `initialize` | request | `InitializeRequest` | `protocolVersion: integer` | `clientCapabilities: ClientCapabilities`, `clientInfo: Implementation`, `_meta` | baseline |
| `authenticate` | request | `AuthenticateRequest` | `methodId: string` | `_meta` | when `authMethods` non-empty |
| `logout` | request | `LogoutRequest` | — | `_meta` | `agentCapabilities.auth.logout` |
| `session/new` | request | `NewSessionRequest` | `cwd: string` (absolute), `mcpServers: McpServer[]` | `additionalDirectories: string[]`, `_meta` | baseline |
| `session/load` | request | `LoadSessionRequest` | `sessionId`, `cwd`, `mcpServers` | `additionalDirectories`, `_meta` | `agentCapabilities.loadSession` |
| `session/resume` | request | `ResumeSessionRequest` | `sessionId`, `cwd` | `mcpServers`, `additionalDirectories`, `_meta` | `sessionCapabilities.resume` |
| `session/close` | request | `CloseSessionRequest` | `sessionId` | `_meta` | `sessionCapabilities.close` |
| `session/list` | request | `ListSessionsRequest` | — | `cwd: string\|null`, `cursor: string\|null`, `_meta` | `sessionCapabilities.list` |
| `session/delete` | request | `DeleteSessionRequest` | `sessionId` | `_meta` | `sessionCapabilities.delete` |
| `session/prompt` | request | `PromptRequest` | `sessionId`, `prompt: ContentBlock[]` | `_meta` | baseline |
| `session/set_mode` | request | `SetSessionModeRequest` | `sessionId`, `modeId: string` | `_meta` | **deprecated** — §10 |
| `session/set_config_option` | request | `SetSessionConfigOptionRequest` | `sessionId`, `configId: string`, `value: string\|boolean` | `type: "boolean"`, `_meta` | §10 |
| `session/cancel` | **notification** | `CancelNotification` | `sessionId` | `_meta` | baseline |
| `$/cancel_request` | **notification** | `CancelRequestNotification` | `requestId: RequestId` | `_meta` | protocol-level; either direction; MAY be ignored |

Baseline an agent **MUST** implement: `session/new`, `session/prompt`, `session/cancel`, and
emitting `session/update`. Everything else is capability-gated.

**Unstable / draft-only agent methods** — present in `schema/v1/meta.unstable.json`, absent from
stable `meta.json`:

`providers/list`, `providers/set`, `providers/disable`, `session/fork`, `mcp/message`,
`nes/start`, `nes/suggest`, `nes/accept` (notification), `nes/reject` (notification), `nes/close`,
`document/didOpen`, `document/didChange`, `document/didClose`, `document/didSave`,
`document/didFocus`.

(`nes/*` is "next edit suggestions"; `document/did*` mirrors LSP text-document sync; `providers/*`
is about **auth providers**, not model selection.)

### 3b. Agent → Client

| Method | Kind | Params type | Required params | Optional params | Gate |
|---|---|---|---|---|---|
| `session/update` | **notification** | `SessionNotification` | `sessionId`, `update: SessionUpdate` | `_meta` | baseline |
| `session/request_permission` | request | `RequestPermissionRequest` | `sessionId`, `toolCall: ToolCallUpdate`, `options: PermissionOption[]` | `_meta` | baseline |
| `fs/read_text_file` | request | `ReadTextFileRequest` | `sessionId`, `path: string` | `line: integer\|null`, `limit: integer\|null`, `_meta` | `clientCapabilities.fs.readTextFile` |
| `fs/write_text_file` | request | `WriteTextFileRequest` | `sessionId`, `path`, `content: string` | `_meta` | `clientCapabilities.fs.writeTextFile` |
| `terminal/create` | request | `CreateTerminalRequest` | `sessionId`, `command: string` | `args: string[]`, `env: EnvVariable[]`, `cwd: string\|null`, `outputByteLimit: integer\|null`, `_meta` | `clientCapabilities.terminal` |
| `terminal/output` | request | `TerminalOutputRequest` | `sessionId`, `terminalId` | `_meta` | `clientCapabilities.terminal` |
| `terminal/wait_for_exit` | request | `WaitForTerminalExitRequest` | `sessionId`, `terminalId` | `_meta` | `clientCapabilities.terminal` |
| `terminal/kill` | request | `KillTerminalRequest` | `sessionId`, `terminalId` | `_meta` | `clientCapabilities.terminal` |
| `terminal/release` | request | `ReleaseTerminalRequest` | `sessionId`, `terminalId` | `_meta` | `clientCapabilities.terminal` |
| `elicitation/create` | request | `CreateElicitationRequest` | `mode`, `message`, + mode-specific | see §11 | `clientCapabilities.elicitation.{form,url}` |
| `elicitation/complete` | **notification** | `CompleteElicitationNotification` | `elicitationId: string` | `_meta` | url mode |
| `$/cancel_request` | **notification** | `CancelRequestNotification` | `requestId` | `_meta` | protocol-level |

Baseline a client **MUST** implement: `session/update` handling and `session/request_permission`.

**Unstable / draft-only client methods:** `mcp/connect`, `mcp/message`, `mcp/disconnect`.

**Extension methods:** any method name beginning with `_` — see §12.

---

## 4. `initialize`

*Source: `docs/protocol/v1/initialization.mdx`; `schema/v1/schema.json` → `InitializeRequest`,
`InitializeResponse`, `ClientCapabilities`, `AgentCapabilities`*

The client **MUST** call `initialize` before creating any session.

### Wire example

Request:

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "method": "initialize",
  "params": {
    "protocolVersion": 1,
    "clientCapabilities": {
      "fs": { "readTextFile": true, "writeTextFile": true },
      "terminal": true
    },
    "clientInfo": { "name": "ubiq", "title": "Ubiq", "version": "0.1.0" }
  }
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "result": {
    "protocolVersion": 1,
    "agentCapabilities": {
      "loadSession": true,
      "promptCapabilities": { "image": true, "audio": true, "embeddedContext": true },
      "mcpCapabilities": { "http": true, "sse": true }
    },
    "agentInfo": { "name": "my-agent", "title": "My Agent", "version": "1.0.0" },
    "authMethods": []
  }
}
```

### `protocolVersion` semantics

*Source: `agent-client-protocol-schema/src/version.rs`, `docs/protocol/v1/initialization.mdx`*

- A **single integer** (`u16` in the reference implementation), representing a **MAJOR** version
  only. A semver string such as `"1.0.0"` is **rejected** by the reference deserializer — there is
  an explicit test for this.
- Incremented **only** for breaking changes. Non-breaking features are introduced via capabilities;
  adding a capability is never a breaking change.
- Known values:
  - `0` — pre-release, "should likely be treated as unsupported"
  - `1` — **current stable**
  - `2` — draft; in the reference crate it exists only behind the `unstable_protocol_v2` cargo
    feature and must be selected explicitly (`LATEST` is deliberately made *unavailable* when that
    feature is on, so code opting into v2 must name `V1` or `V2`).

**Negotiation:**

1. The `initialize` request **MUST** include the latest protocol version the client supports.
2. If the agent supports that version, it **MUST** respond with the same number.
3. Otherwise the agent **MUST** respond with the latest version *it* supports.
4. If the client does not support the version the agent returned, the client **SHOULD** close the
   connection and inform the user.

### `InitializeRequest`

| Field | Type | Required |
|---|---|---|
| `protocolVersion` | `integer` | ✅ |
| `clientCapabilities` | `ClientCapabilities` | optional (absent ⇒ everything unsupported) |
| `clientInfo` | `Implementation \| null` | optional; SHOULD be sent; will become required |
| `_meta` | `object \| null` | optional |

### `InitializeResponse`

| Field | Type | Required |
|---|---|---|
| `protocolVersion` | `integer` | ✅ |
| `agentCapabilities` | `AgentCapabilities` | optional |
| `authMethods` | `AuthMethod[]` | optional |
| `agentInfo` | `Implementation \| null` | optional; SHOULD be sent; will become required |
| `_meta` | `object \| null` | optional |

### `Implementation`

| Field | Type | Required | Notes |
|---|---|---|---|
| `name` | `string` | ✅ | programmatic/logical id; display fallback |
| `title` | `string \| null` | optional | human-readable, preferred for UI |
| `version` | `string` | ✅ | e.g. `"1.0.0"` |
| `_meta` | `object \| null` | optional | |

### The `{}` vs `null` vs omitted idiom

The **older** capabilities are plain booleans (`terminal`, `loadSession`, `fs.readTextFile`,
`promptCapabilities.image`, …): absent or `false` ⇒ unsupported.

The **newer nested** capabilities are **presence objects**. For all of them the schema doc-comment
is identical in form:

> Omitted or `null` both mean the client/agent does not advertise support. Supplying `{}` means …

So the tri-state is really a bi-state with a redundant encoding:

| Value | Meaning |
|---|---|
| field omitted | unsupported |
| `null` | unsupported (identical to omitted) |
| `{}` | **supported** |
| `{ ...future fields... }` | supported, with refinements |

The point of the object form is forward-compatibility: a capability can later grow sub-fields
without a breaking change. Model these in Rust as `Option<SomeCapStruct>`, **not** as `bool`.

One trap called out explicitly in `initialization.mdx`: **unlike MCP, ACP does not treat
`elicitation: {}` as form support.** An empty or all-null `ElicitationCapabilities` object is valid
but advertises *nothing*; you must supply `elicitation.form: {}` and/or `elicitation.url: {}`.

Capabilities omitted in the `initialize` request **MUST** be treated as **UNSUPPORTED**. Both sides
**SHOULD** support all possible combinations of their peer's capabilities.

### `ClientCapabilities` — complete tree

```
ClientCapabilities {
  fs?: FileSystemCapabilities {
    readTextFile?:  boolean                 // default false — enables fs/read_text_file
    writeTextFile?: boolean                 // default false — enables fs/write_text_file
    _meta?: object|null
  }

  terminal?: boolean                        // single flag enabling ALL terminal/* methods

  session?: ClientSessionCapabilities|null {
    configOptions?: SessionConfigOptionsCapabilities|null {
      boolean?: BooleanConfigOptionCapabilities|null {   // {} == client accepts type:"boolean" options
        _meta?: object|null
      }
      _meta?: object|null
    }
    _meta?: object|null
  }

  auth?: AuthCapabilities {
    terminal?: boolean                      // client can re-launch the agent binary interactively
                                            // for TUI-based auth; enables AuthMethod type:"terminal"
    _meta?: object|null
  }

  elicitation?: ElicitationCapabilities|null {
    form?: ElicitationFormCapabilities|null {  // {} == supports form elicitation
      _meta?: object|null
    }
    url?:  ElicitationUrlCapabilities|null  {  // {} == supports url elicitation
      _meta?: object|null
    }
    _meta?: object|null
  }

  _meta?: object|null                       // also the place to advertise custom capabilities
}
```

### `AgentCapabilities` — complete tree

```
AgentCapabilities {
  loadSession?: boolean                     // session/load. Still a top-level bool; the spec notes
                                            // "This will be unified in future versions."

  promptCapabilities?: PromptCapabilities {
    image?:           boolean               // default false — ContentBlock type:"image" in prompts
    audio?:           boolean               // default false — ContentBlock type:"audio" in prompts
    embeddedContext?: boolean               // default false — ContentBlock type:"resource" in prompts
    _meta?: object|null
  }

  mcpCapabilities?: McpCapabilities {
    http?: boolean                          // default false — McpServer type:"http"
    sse?:  boolean                          // default false — McpServer type:"sse"
                                            //   (this transport is deprecated by MCP itself)
    _meta?: object|null
  }

  sessionCapabilities?: SessionCapabilities {
    list?:   SessionListCapabilities|null                    // {} == session/list
    delete?: SessionDeleteCapabilities|null                  // {} == session/delete
    resume?: SessionResumeCapabilities|null                  // {} == session/resume
    close?:  SessionCloseCapabilities|null                   // {} == session/close
    additionalDirectories?: SessionAdditionalDirectoriesCapabilities|null
                                                             // {} == accepts additionalDirectories
                                                             //       on session lifecycle requests
    _meta?: object|null
  }

  auth?: AgentAuthCapabilities {
    logout?: LogoutCapabilities|null        // {} == logout method
    _meta?: object|null
  }

  _meta?: object|null
}
```

**Agent baseline (MUST, no capability required):** `session/new`, `session/prompt`,
`session/cancel`, `session/update`; `ContentBlock` `text` and `resource_link` in prompts; MCP
**stdio** transport.

### `AuthMethod`

Internally tagged on `type`, with the payload flattened. **When `type` is absent the method is
treated as `agent`** — that is the default variant.

`AuthMethodAgent` (`type` absent, or `type: "agent"`) — agent handles auth itself via `authenticate`:

| Field | Type | Required |
|---|---|---|
| `id` | `string` (`AuthMethodId`) | ✅ |
| `name` | `string` | ✅ |
| `description` | `string \| null` | optional |
| `_meta` | `object \| null` | optional |

`AuthMethodTerminal` (`type: "terminal"`) — the client re-runs the configured agent program as a
separate interactive process so the user can authenticate in a TUI:

| Field | Type | Required |
|---|---|---|
| `id` | `string` | ✅ |
| `name` | `string` | ✅ |
| `description` | `string \| null` | optional |
| `args` | `string[]` | optional — appended to the configured agent invocation |
| `env` | `map<string,string>` | optional — overrides same-named vars in the base launch config |
| `_meta` | `object \| null` | optional |

Rules: the agent **MUST** advertise `terminal` methods only when the client set
`clientCapabilities.auth.terminal: true`. A zero exit status signals success; any other termination
signals failure. The client **MUST NOT** pass a terminal method id to `authenticate`.

### `authenticate` / `logout`

- `authenticate` params `AuthenticateRequest { methodId: string ✅, _meta? }` — `methodId` must be
  one of the ids advertised in `authMethods`. Response `AuthenticateResponse { _meta? }` (empty).
- `logout` params `LogoutRequest { _meta? }`, response `LogoutResponse { _meta? }`. Gated on
  `agentCapabilities.auth.logout`.
- Error code `-32000` ("Authentication required") is what an agent returns when an operation needs
  auth that has not happened.

---

## 5. Session lifecycle: `new`, `load`, `resume`, `close`, `list`, `delete`

*Source: `schema/v1/schema.json`; `docs/protocol/v1/session-setup.mdx`, `prompt-turn.mdx`*

### `session/new`

Params — `NewSessionRequest`:

| Field | Type | Required | Notes |
|---|---|---|---|
| `cwd` | `string` | ✅ | **must be absolute** |
| `mcpServers` | `McpServer[]` | ✅ | may be `[]` |
| `additionalDirectories` | `string[]` | optional | each absolute; extra workspace roots that widen the session's filesystem scope **without** changing `cwd`, which remains the base for relative paths. Omitted and empty are equivalent. Gated on `sessionCapabilities.additionalDirectories` |
| `_meta` | `object \| null` | optional | |

Result — `NewSessionResponse`:

| Field | Type | Required | Notes |
|---|---|---|---|
| `sessionId` | `string` | ✅ | use in all subsequent requests |
| `modes` | `SessionModeState \| null` | optional | legacy mode state — §10 |
| `configOptions` | `SessionConfigOption[] \| null` | optional | preferred config mechanism — §10 |
| `_meta` | `object \| null` | optional | |

### `McpServer`

Internally tagged on `type` with the payload flattened. **The default variant when `type` is absent
is stdio** — all agents MUST support stdio.

`type: "stdio"` → `McpServerStdio`:

| Field | Type | Required |
|---|---|---|
| `name` | `string` | ✅ (human-readable identifier) |
| `command` | `string` | ✅ (**absolute** path to the executable) |
| `args` | `string[]` | ✅ |
| `env` | `EnvVariable[]` | ✅ |
| `_meta` | `object \| null` | optional |

`type: "http"` → `McpServerHttp` (requires `mcpCapabilities.http`):

| Field | Type | Required |
|---|---|---|
| `name` | `string` | ✅ |
| `url` | `string` | ✅ |
| `headers` | `HttpHeader[]` | ✅ |
| `_meta` | `object \| null` | optional |

`type: "sse"` → `McpServerSse` (requires `mcpCapabilities.sse`): identical field set to
`McpServerHttp`.

Note that `args`, `env` and `headers` are **required** on these structs — send `[]`, not `null`.

- `EnvVariable { name: string ✅, value: string ✅, _meta? }`
- `HttpHeader   { name: string ✅, value: string ✅, _meta? }`

Both are **arrays of name/value pairs**, not JSON objects. This differs from the usual
`{"KEY":"value"}` env representation.

### `session/load`

Gated on `agentCapabilities.loadSession`.

Params — `LoadSessionRequest`:

| Field | Type | Required |
|---|---|---|
| `sessionId` | `string` | ✅ |
| `cwd` | `string` (absolute) | ✅ |
| `mcpServers` | `McpServer[]` | ✅ |
| `additionalDirectories` | `string[]` | optional — when non-empty, this is the *complete resulting* additional-root list, and may differ from the original |
| `_meta` | `object \| null` | optional |

Result — `LoadSessionResponse`:

| Field | Type | Required |
|---|---|---|
| `modes` | `SessionModeState \| null` | optional |
| `configOptions` | `SessionConfigOption[] \| null` | optional |
| `_meta` | `object \| null` | optional |

Note: **no `sessionId` in the response** — the client already supplied it.

Semantics: the agent replays the entire conversation history to the client as `session/update`
notifications before responding.

### `session/resume`

Gated on `sessionCapabilities.resume`. The lighter-weight sibling of `session/load`: resumes an
existing session **without replaying previous messages**. For agents that can resume but do not
implement full history replay.

Params — `ResumeSessionRequest`: `sessionId` ✅, `cwd` ✅ (absolute), `mcpServers?: McpServer[]`
(**optional** here, unlike `load`), `additionalDirectories?`, `_meta?`.

Result — `ResumeSessionResponse`: `modes?`, `configOptions?`, `_meta?` — same shape as
`LoadSessionResponse`.

### `session/close`

Gated on `sessionCapabilities.close`. Params `CloseSessionRequest { sessionId ✅, _meta? }`; result
`CloseSessionResponse { _meta? }`.

Semantics: the agent **must** cancel any ongoing work for the session (treat it as if
`session/cancel` had been called) and then free all resources associated with it.

### `session/list`

Gated on `sessionCapabilities.list`.

Params — `ListSessionsRequest`: `cwd?: string|null` (filter, absolute), `cursor?: string|null`
(opaque pagination token from a previous `nextCursor`), `_meta?`.

Result — `ListSessionsResponse`: `sessions: SessionInfo[]` ✅, `nextCursor?: string|null` (absent ⇒
no more results), `_meta?`.

`SessionInfo`:

| Field | Type | Required |
|---|---|---|
| `sessionId` | `string` | ✅ |
| `cwd` | `string` (absolute) | ✅ |
| `additionalDirectories` | `string[]` | optional — complete ordered list; omitted and empty equivalent |
| `title` | `string \| null` | optional |
| `updatedAt` | `string \| null` | optional — ISO 8601 timestamp of last activity |
| `_meta` | `object \| null` | optional |

### `session/delete`

Gated on `sessionCapabilities.delete`. Removes a session from `session/list`. Params
`DeleteSessionRequest { sessionId ✅, _meta? }`; result `DeleteSessionResponse { _meta? }`.

---

## 6. `session/prompt` and `session/cancel`

*Source: `schema/v1/schema.json`; `docs/protocol/v1/prompt-turn.mdx`, `content.mdx`*

### `session/prompt`

Params — `PromptRequest`:

| Field | Type | Required |
|---|---|---|
| `sessionId` | `string` | ✅ |
| `prompt` | `ContentBlock[]` | ✅ |
| `_meta` | `object \| null` | optional |

Result — `PromptResponse`:

| Field | Type | Required |
|---|---|---|
| `stopReason` | `StopReason` | ✅ |
| `_meta` | `object \| null` | optional |

`StopReason` — all 5 values (snake_case strings):

| Value | Meaning |
|---|---|
| `end_turn` | The turn ended successfully. |
| `max_tokens` | The agent hit its maximum token count. |
| `max_turn_requests` | The agent hit the maximum number of model requests allowed between user turns. |
| `refusal` | The agent refused to continue. The user prompt and everything after it will not be included in the next prompt. |
| `cancelled` | The turn was cancelled by the client via `session/cancel`. **MUST** be returned when the client sends that notification. |

### The prompt-turn lifecycle

1. Client → `session/prompt` (user message).
2. Agent loops: `session/update` with `plan`, `agent_message_chunk`, `agent_thought_chunk`,
   `tool_call`, `tool_call_update`; interleaved with `session/request_permission`, `fs/*`,
   `terminal/*` requests back to the client.
3. Optional: client → `session/cancel`.
4. Agent → `session/prompt` response with a `stopReason`.

After the turn completes the client may send another `session/prompt` on the same session, building
on the accumulated context.

### `ContentBlock`

Internally tagged on `type`, payload flattened into the same object.

| `type` | Struct | Fields |
|---|---|---|
| `"text"` | `TextContent` | `text: string` ✅; `annotations?: Annotations\|null`; `_meta?` |
| `"image"` | `ImageContent` | `data: string` ✅ (base64); `mimeType: string` ✅; `uri?: string\|null`; `annotations?`; `_meta?` |
| `"audio"` | `AudioContent` | `data: string` ✅ (base64); `mimeType: string` ✅; `annotations?`; `_meta?` — **no `uri` field** |
| `"resource_link"` | `ResourceLink` | `uri: string` ✅; `name: string` ✅; `mimeType?: string\|null`; `title?: string\|null`; `description?: string\|null`; `size?: integer\|null` (bytes); `annotations?`; `_meta?` |
| `"resource"` | `EmbeddedResource` | `resource: EmbeddedResourceResource` ✅; `annotations?`; `_meta?` |

Note the discriminator value is `resource_link` with an underscore, while the embedded variant is
just `resource`.

`EmbeddedResourceResource` is an **untagged** either-or, discriminated structurally by which content
key is present:

- `TextResourceContents { uri: string ✅, text: string ✅, mimeType?: string|null, _meta? }`
- `BlobResourceContents { uri: string ✅, blob: string ✅ (base64), mimeType?: string|null, _meta? }`

`Annotations`:

| Field | Type | Notes |
|---|---|---|
| `audience` | `Role[] \| null` | intended recipients |
| `lastModified` | `string \| null` | ISO 8601 |
| `priority` | `number \| null` | relative importance for surfacing |
| `_meta` | `object \| null` | |

`Role` — 2 values: `"assistant"`, `"user"`.

**Capability rules:** agents **MUST** accept `text` and `resource_link` in prompts. `image` requires
`promptCapabilities.image`, `audio` requires `.audio`, `resource` requires `.embeddedContext`. The
**client** is responsible for adapting content the agent cannot accept. Embedded resources are
preferred over links when the client already has the content, since they avoid a round trip.

`ContentBlock` also appears in `session/update` chunks and in `ToolCallContent{type:"content"}` —
the same union is reused throughout.

### `session/cancel`

A **notification** (no response). Params `CancelNotification { sessionId ✅, _meta? }`.

Cancellation contract, verbatim from `prompt-turn.mdx`:

1. The client **SHOULD** preemptively mark all non-finished tool calls of the current turn as
   cancelled as soon as it sends `session/cancel`.
2. The client **MUST** respond to all pending `session/request_permission` requests with the
   `cancelled` outcome.
3. On receiving the notification the agent **SHOULD** stop all language model requests and tool
   invocations as soon as possible.
4. After aborting and flushing pending updates, the agent **MUST** respond to the original
   `session/prompt` request with the `cancelled` **stop reason** — *not* a JSON-RPC error. The spec
   carries an explicit warning: HTTP client libraries throw on abort, and agents **MUST** catch
   those so they do not surface as errors the client would display to the user.
5. The agent **MAY** send further `session/update` notifications after receiving the cancel, but
   **MUST** send them before responding to `session/prompt`. The client **SHOULD** still accept tool
   call updates that arrive after it sent the cancel.

Distinct from `$/cancel_request`, which cancels one specific in-flight JSON-RPC request by id.

---

## 7. `session/update` — all eleven discriminants

*Source: `schema/v1/schema.json` → `SessionNotification`, `SessionUpdate` and payload structs*

### `SessionNotification` (the params of `session/update`)

| Field | Type | Required |
|---|---|---|
| `sessionId` | `string` | ✅ |
| `update` | `SessionUpdate` | ✅ |
| `_meta` | `object \| null` | optional |

### `SessionUpdate` encoding

Internally tagged on **`sessionUpdate`** with snake_case values, and the variant payload
**flattened into the same object**. In JSON Schema terms each variant is:

```json
{ "properties": { "sessionUpdate": { "const": "tool_call" } },
  "required": ["sessionUpdate"],
  "allOf": [ { "$ref": "#/$defs/ToolCall" } ] }
```

There is **no nested payload object**. On the wire:

```json
{ "sessionUpdate": "tool_call", "toolCallId": "call_1", "title": "Read main.rs", "kind": "read" }
```

In Rust: `#[serde(tag = "sessionUpdate", rename_all = "snake_case")]` — *not* `tag` + `content`.

### The eleven variants

| `sessionUpdate` | Payload struct | Fields |
|---|---|---|
| `user_message_chunk` | `ContentChunk` | `content: ContentBlock` ✅; `messageId?: string\|null`; `_meta?` |
| `agent_message_chunk` | `ContentChunk` | same as above |
| `agent_thought_chunk` | `ContentChunk` | same as above |
| `tool_call` | `ToolCall` | §8 |
| `tool_call_update` | `ToolCallUpdate` | §8 |
| `plan` | `Plan` | `entries: PlanEntry[]` ✅; `_meta?` |
| `available_commands_update` | `AvailableCommandsUpdate` | `availableCommands: AvailableCommand[]` ✅; `_meta?` |
| `current_mode_update` | `CurrentModeUpdate` | `currentModeId: string` ✅; `_meta?` — **see discrepancy below** |
| `config_option_update` | `ConfigOptionUpdate` | `configOptions: SessionConfigOption[]` ✅ (complete state); `_meta?` |
| `session_info_update` | `SessionInfoUpdate` | `title?: string\|null`; `updatedAt?: string\|null` (ISO 8601); `_meta?` |
| `usage_update` | `UsageUpdate` | `used: integer` ✅; `size: integer` ✅; `cost?: Cost\|null`; `_meta?` |

### ⚠️ `current_mode_update` — schema vs prose discrepancy

**The JSON Schema defines the field as `currentModeId`.** The prose page
`docs/protocol/v1/session-modes.mdx` shows a worked example that uses **`modeId`** instead:

```json
{ "sessionUpdate": "current_mode_update", "modeId": "code" }
```

The JSON Schema is generated from the Rust source (`CurrentModeUpdate { current_mode_id }` with
`rename_all = "camelCase"`) and is authoritative; the doc example is stale.

**Recommendation:** serialise `currentModeId`; on deserialisation accept `modeId` as an alias, since
real agents may have been written against the doc example.

### Payload details

`ContentChunk`:

| Field | Type | Required | Notes |
|---|---|---|---|
| `content` | `ContentBlock` | ✅ | a single content item |
| `messageId` | `string \| null` | optional | all chunks of one logical message share a `messageId`; a change in it indicates a new message has started. Optional in v1, **required in v2**. |
| `_meta` | `object \| null` | optional | |

`Plan` / `PlanEntry`:

- `Plan { entries: PlanEntry[] ✅, _meta? }`
- `PlanEntry { content: string ✅, priority ✅, status ✅, _meta? }`
- `PlanEntryPriority` — 3 values: `high`, `medium`, `low`
- `PlanEntryStatus` — 3 values: `pending`, `in_progress`, `completed`

**A `plan` update is a full replacement.** The schema states: *"When updating a plan, the agent must
send a complete list of all entries with their current status. The client replaces the entire plan
with each update."*

`AvailableCommand` (slash commands):

| Field | Type | Required |
|---|---|---|
| `name` | `string` | ✅ — e.g. `create_plan`, `research_codebase` |
| `description` | `string` | ✅ |
| `input` | `AvailableCommandInput \| null` | optional |
| `_meta` | `object \| null` | optional |

`AvailableCommandInput` currently has exactly one variant,
`UnstructuredCommandInput { hint: string ✅, _meta? }` — all text typed after the command name is
passed through; `hint` is placeholder text shown before input is provided.

`SessionInfoUpdate` is a **partial** update: omitted ⇒ unchanged, explicit `null` ⇒ clear.

`UsageUpdate` / `Cost`:

- `UsageUpdate { used: integer ✅ (tokens currently in context), size: integer ✅ (total context
  window in tokens), cost?: Cost|null, _meta? }`
- `Cost { amount: number ✅ (total cumulative for the session), currency: string ✅ (ISO 4217, e.g.
  `"USD"`), _meta? }`

---

## 8. Tool calls

*Source: `schema/v1/schema.json` → `ToolCall`, `ToolCallUpdate`, `ToolKind`, `ToolCallStatus`,
`ToolCallContent`, `Diff`, `Terminal`, `ToolCallLocation`; `docs/protocol/v1/tool-calls.mdx`*

### `ToolCall` — the `tool_call` update

| Field | Type | Required | Notes |
|---|---|---|---|
| `toolCallId` | `string` (`ToolCallId`) | ✅ | unique within the session |
| `title` | `string` | ✅ | human-readable description of what the tool is doing |
| `kind` | `ToolKind` | optional | defaults to `other` |
| `status` | `ToolCallStatus` | optional | defaults to `pending` |
| `content` | `ToolCallContent[]` | optional | |
| `locations` | `ToolCallLocation[]` | optional | drives client "follow-along" |
| `rawInput` | any JSON | optional | raw parameters sent to the tool |
| `rawOutput` | any JSON | optional | raw output returned by the tool |
| `_meta` | `object \| null` | optional | |

### `ToolCallUpdate` — the `tool_call_update` update

Same field names, but **only `toolCallId` is required**; every other field is optional/nullable.

| Field | Type | Required |
|---|---|---|
| `toolCallId` | `string` | ✅ |
| `kind` | `ToolKind \| null` | optional |
| `status` | `ToolCallStatus \| null` | optional |
| `title` | `string \| null` | optional |
| `content` | `ToolCallContent[] \| null` | optional |
| `locations` | `ToolCallLocation[] \| null` | optional |
| `rawInput` | any JSON | optional |
| `rawOutput` | any JSON | optional |
| `_meta` | `object \| null` | optional |

**Patch semantics (v1):** *"All fields except the tool call ID are optional — only changed fields
need to be included."* Omitted ⇒ unchanged. `content` and `locations` **replace** the whole
collection ("Replace the content collection", "Replace the locations collection") — they do **not**
append in v1. (v2 changes this: content becomes streamable/appendable. See §13.)

`ToolCallUpdate` is also the type of the `toolCall` field in `session/request_permission` — so a
permission request may legitimately carry only a `toolCallId` plus whatever the agent chose to
include.

### `ToolKind` — all 10 values

| Value | Meaning |
|---|---|
| `read` | Reading files or data. |
| `edit` | Modifying files or content. |
| `delete` | Removing files or data. |
| `move` | Moving or renaming files. |
| `search` | Searching for information. |
| `execute` | Running commands or code. |
| `think` | Internal reasoning or planning. |
| `fetch` | Retrieving external data. |
| `switch_mode` | Switching the current session mode. |
| `other` | Other tool types (**default**). |

### `ToolCallStatus` — all 4 values

| Value | Meaning |
|---|---|
| `pending` | Not started: input is still streaming, or approval is awaited. **Default.** |
| `in_progress` | Currently running. |
| `completed` | Completed successfully. |
| `failed` | Failed with an error. |

There is deliberately **no `cancelled` status** in v1 — cancellation is handled by the client
locally marking unfinished calls and by the `cancelled` stop reason.

### `ToolCallContent` — 3 variants

Internally tagged on `type`, payload flattened.

**`type: "content"`** → `Content`:

| Field | Type | Required |
|---|---|---|
| `content` | `ContentBlock` | ✅ |
| `_meta` | `object \| null` | optional |

**`type: "diff"`** → `Diff` — exact field names:

| Field | Type | Required | Notes |
|---|---|---|---|
| `path` | `string` | ✅ | **absolute** path of the file being modified |
| `oldText` | `string \| null` | optional | the original content; **`null` for a new file** |
| `newText` | `string` | ✅ | the content after modification |
| `_meta` | `object \| null` | optional | |

Note this is a **whole-file before/after pair**, not a unified diff or a hunk list. The client
computes and renders the diff itself. (v2 replaces this entirely with structured file changes plus
an optional `git_patch` — §13.)

**`type: "terminal"`** → `Terminal`:

| Field | Type | Required |
|---|---|---|
| `terminalId` | `string` | ✅ |
| `_meta` | `object \| null` | optional |

Embeds a terminal previously created with `terminal/create` into the tool call's content stream, so
the client can render live output. **The terminal must be embedded before `terminal/release` is
called.**

### `ToolCallLocation`

| Field | Type | Required | Notes |
|---|---|---|---|
| `path` | `string` | ✅ | **absolute** path being accessed or modified |
| `line` | `integer \| null` | optional | **1-based** |
| `_meta` | `object \| null` | optional | |

Purpose: enables clients to implement "follow-along" — tracking which files the agent is working
with in real time and scrolling the editor accordingly.

---

## 9. `session/request_permission`

*Source: `schema/v1/schema.json` → `RequestPermissionRequest`, `RequestPermissionResponse`,
`RequestPermissionOutcome`, `SelectedPermissionOutcome`, `PermissionOption`, `PermissionOptionKind`;
`docs/protocol/v1/tool-calls.mdx`, `session-modes.mdx`*

An **agent → client request** (not a notification): the agent needs user authorisation before
performing a sensitive operation.

### Request params — `RequestPermissionRequest`

| Field | Type | Required | Notes |
|---|---|---|---|
| `sessionId` | `string` | ✅ | |
| `toolCall` | **`ToolCallUpdate`** | ✅ | note the type: the *patch* struct, so only `toolCallId` is strictly required inside it |
| `options` | `PermissionOption[]` | ✅ | the choices offered to the user |
| `_meta` | `object \| null` | optional | |

### `PermissionOption`

| Field | Type | Required | Notes |
|---|---|---|---|
| `optionId` | `string` (`PermissionOptionId`) | ✅ | echoed back in the response |
| `name` | `string` | ✅ | human-readable label to display |
| `kind` | `PermissionOptionKind` | ✅ | hint for icon/UI treatment |
| `_meta` | `object \| null` | optional | |

### `PermissionOptionKind` — all 4 values

| Value | Meaning |
|---|---|
| `allow_once` | Allow this operation only this time. |
| `allow_always` | Allow this operation and remember the choice. |
| `reject_once` | Reject this operation only this time. |
| `reject_always` | Reject this operation and remember the choice. |

The `kind` is a **hint only** — the actual semantics of the option are whatever the agent decides;
`optionId` is what is acted upon.

### Response — `RequestPermissionResponse`

| Field | Type | Required |
|---|---|---|
| `outcome` | `RequestPermissionOutcome` | ✅ |
| `_meta` | `object \| null` | optional |

`RequestPermissionOutcome` is internally tagged on **`outcome`** (not `type`), payload flattened:

| `outcome` | Payload | Fields |
|---|---|---|
| `"cancelled"` | — | no payload fields. The prompt turn was cancelled before the user responded. |
| `"selected"` | `SelectedPermissionOutcome` | `optionId: string` ✅; `_meta?` |

So the full wire shape has the word twice — the field is `outcome`, and its tag key is also
`outcome`:

```json
{ "outcome": { "outcome": "selected", "optionId": "allow_once_1" } }
```
```json
{ "outcome": { "outcome": "cancelled" } }
```

**Client obligation:** when the client sends `session/cancel`, it **MUST** answer every pending
`session/request_permission` with `outcome: "cancelled"`.

### The `switch_mode` idiom

*Source: `docs/protocol/v1/session-modes.mdx` §Exiting plan modes*

A recurring pattern worth knowing because it makes `optionId` carry domain meaning: agents in
plan/architect mode expose an "exit mode" tool to the model. When the model calls it, the agent
issues a permission request whose tool call has `kind: "switch_mode"` and whose **`optionId`s are
mode ids**:

```json
{
  "jsonrpc": "2.0", "id": 3, "method": "session/request_permission",
  "params": {
    "sessionId": "sess_abc123def456",
    "toolCall": {
      "toolCallId": "call_switch_mode_001",
      "title": "Ready for implementation",
      "kind": "switch_mode",
      "status": "pending",
      "content": [ { "type": "text", "text": "## Implementation Plan..." } ]
    },
    "options": [
      { "optionId": "code",   "name": "Yes, and auto-accept all actions",   "kind": "allow_always" },
      { "optionId": "ask",    "name": "Yes, and manually accept actions",   "kind": "allow_once"   },
      { "optionId": "reject", "name": "No, stay in architect mode",         "kind": "reject_once"  }
    ]
  }
}
```

Choosing an option runs the tool, which sets the mode and then emits a `current_mode_update`
`session/update`. A client must therefore not assume `optionId` values are opaque UI tokens with no
meaning outside the prompt — but it also must not *interpret* them; it just echoes the choice.

---

## 10. Modes vs. models vs. config options — the definitive resolution

**Schema artifacts checked:** `schema/v1/schema.json`, `schema/v1/schema.unstable.json`,
`schema/v2/schema.json`, `schema/v2/schema.unstable.json`, all on `main` @ 2026-09-03.
**Protocol version:** v1 stable, JSON-Schema marker `1.21.0`.

### The contradiction, resolved

**Both mechanisms exist in stable v1 today. `session/set_config_option` is the preferred one, and
`session/set_mode` is explicitly deprecated and scheduled for removal.**

This is now stated normatively in two places. From `docs/protocol/v1/session-modes.mdx`, the
top-of-page note:

> You can now use [Session Config Options]. **Dedicated session mode methods will be removed in a
> future version of the protocol.** Until then, you can offer both to clients for backwards
> compatibility.

And from `docs/protocol/v1/session-config-options.mdx`:

> Session Config Options are the preferred way to expose session-level configuration. If an Agent
> provides `configOptions`, Clients **SHOULD** use them instead of the `modes` field. **Modes will
> be removed in a future version of the protocol.**

**Confirming evidence from v2:** `schema/v2/meta.json` lists the agent methods for the v2 draft and
contains **no `session/set_mode`** at all — only `session/set_config_option`. The removal has
already happened there.

**Transition rule** (`session-config-options.mdx` §Relationship to Session Modes) — during the
transition, agents that expose mode-like configuration **SHOULD** send *both*:

- `configOptions` containing an option with `category: "mode"`, for clients that support config
  options, and
- `modes` (a `SessionModeState`), for clients that only support the older API,

keeping the two in sync. Clients that support config options **SHOULD** use `configOptions`
exclusively and ignore `modes`; clients that don't **SHOULD** fall back to `modes`.

So: earlier research claiming "a generic config-option mechanism supersedes `session/set_mode`" is
**correct**, and earlier research describing `session/set_mode` as the current mechanism is
**correct for v1 only, and describing something on its way out**. They are not in conflict; they
describe two overlapping generations that currently coexist.

### Modes — the legacy shape (still valid in stable v1)

`SessionModeState` — returned as the `modes` field of `NewSessionResponse`, `LoadSessionResponse`,
`ResumeSessionResponse`:

| Field | Type | Required |
|---|---|---|
| `currentModeId` | `string` (`SessionModeId`) | ✅ |
| `availableModes` | `SessionMode[]` | ✅ |
| `_meta` | `object \| null` | optional |

`SessionMode`:

| Field | Type | Required |
|---|---|---|
| `id` | `string` (`SessionModeId`) | ✅ |
| `name` | `string` | ✅ |
| `description` | `string \| null` | optional |
| `_meta` | `object \| null` | optional |

`session/set_mode`:

- Params `SetSessionModeRequest { sessionId ✅, modeId: string ✅, _meta? }`
- Result `SetSessionModeResponse { _meta? }` — empty

The mode may be changed at any point in a session, whether the agent is idle or generating.

Agent-initiated change → `session/update` with `sessionUpdate: "current_mode_update"`, carrying
`currentModeId` (**see the schema-vs-prose discrepancy in §7**).

### Config options — the successor

`SessionConfigOption` is internally tagged on `type` with common fields plus a per-type flattened
payload:

```
SessionConfigOption {
  id:           string                             ✅   // SessionConfigId
  name:         string                             ✅   // human-readable label
  description?: string|null
  category?:    SessionConfigOptionCategory|null         // UX hint only
  type:         "select" | "boolean"               ✅
  // flattened payload, per type:
  //   type "select"  → SessionConfigSelect  { currentValue: SessionConfigValueId ✅,
  //                                           options: SessionConfigSelectOptions ✅ }
  //   type "boolean" → SessionConfigBoolean { currentValue: boolean ✅ }
  _meta?: object|null
}
```

`SessionConfigSelectOptions` is an either-or: a **flat** `SessionConfigSelectOption[]`, **or** a
**grouped** `SessionConfigSelectGroup[]`.

- `SessionConfigSelectOption { value: string ✅ (SessionConfigValueId), name: string ✅,
  description?: string|null, _meta? }`
- `SessionConfigSelectGroup { group: string ✅ (SessionConfigGroupId), name: string ✅,
  options: SessionConfigSelectOption[] ✅, _meta? }`

`SessionConfigOptionCategory` — an **open** enum (unknown strings accepted; names beginning with `_`
are free for custom use, non-`_` names reserved for the spec):

| Value | Meaning |
|---|---|
| `mode` | Session mode selector |
| `model` | Model selector |
| `model_config` | Model-related parameter (context size, speed/quality trade-off) |
| `thought_level` | Thought/reasoning level selector |

Categories are **UX only** and **MUST NOT** be required for correctness; clients **MUST** handle
missing and unknown categories gracefully. Clients **SHOULD** render `model_config` options near the
`model` selector. No capability negotiation is required for category values.

**Ordering is significant:** the order of the `configOptions` array is the agent's preferred
priority. Clients SHOULD display in that order, use it to break ties within a category, and prefer
earlier entries when showing a limited number.

**Defaults:** agents **MUST** always provide a default value for every option, so they work when the
client ignores config options entirely. A client that sees an unrecognised `type` **SHOULD** ignore
that option; the agent continues with its default.

**Boolean gating:** agents **MUST NOT** include `type: "boolean"` options unless the client
advertised `clientCapabilities.session.configOptions.boolean: {}`. Agents supporting older clients
should omit the boolean option or offer a `select` fallback.

`session/set_config_option`:

- Params `SetSessionConfigOptionRequest`:

  | Field | Type | Required | Notes |
  |---|---|---|---|
  | `sessionId` | `string` | ✅ | |
  | `configId` | `string` | ✅ | the option's `id` |
  | `value` | `string \| boolean` | ✅ | |
  | `type` | `"boolean"` | conditional | **absent for string values** (the default variant); `"boolean"` for boolean values. Unknown `type` values with string payloads degrade gracefully to the string variant. |
  | `_meta` | `object \| null` | optional | |

- Result `SetSessionConfigOptionResponse { configOptions: SessionConfigOption[] ✅, _meta? }` —
  **always the complete set** of options with current values, so the agent can reflect dependent
  changes (e.g. changing the model changes the available reasoning levels).

Agent-initiated change → `session/update` with `sessionUpdate: "config_option_update"`, also
carrying the **complete** configuration state. Typical reasons: switching mode after a planning
phase, falling back to another model on rate-limit, adjusting options based on discovered context.

### Models — definitive answer

**`availableModels`, `currentModelId`, and `session/set_model` are not in the official ACP schema —
in any version.**

Verification method: grep for `availableModels`, `currentModelId`, `set_model` and `"models"` across
all four schema artifacts:

| Artifact | Hits |
|---|---|
| `schema/v1/schema.json` | 0 |
| `schema/v1/schema.unstable.json` | 0 |
| `schema/v2/schema.json` | 0 |
| `schema/v2/schema.unstable.json` | 0 |

**Model selection in official ACP is a session config option with `category: "model"`** — a `select`
option whose `currentValue` is the model id and whose `options` are the available models — changed
via `session/set_config_option`. `docs/protocol/v1/session-config-options.mdx` carries this as a
literal worked example:

```json
{
  "id": "model",
  "name": "Model",
  "category": "model",
  "type": "select",
  "currentValue": "model-1",
  "options": [
    { "value": "model-1", "name": "Model 1", "description": "The fastest model" },
    { "value": "model-2", "name": "Model 2", "description": "The most powerful model" }
  ]
}
```

The `model_config` category exists for related parameters (context size, speed/quality trade-offs).

If `availableModels` / `session/set_model` appear in some agent's traffic, they are a **vendor
extension** (or a pre-standardisation Zed-internal proposal), not ACP. Note also that the unstable
v1 draft has a `providers/list` / `providers/set` / `providers/disable` family — that is about
**authentication providers**, not model selection, and it is not in stable v1.

---

## 11. Client-side methods, and errors

*Source: `schema/v1/schema.json`; `docs/protocol/v1/file-system.mdx`, `terminals.mdx`,
`elicitation.mdx`*

### `fs/*`

Gated **individually**: `clientCapabilities.fs.readTextFile` and `clientCapabilities.fs.writeTextFile`.

**`fs/read_text_file`** — params `ReadTextFileRequest`:

| Field | Type | Required | Notes |
|---|---|---|---|
| `sessionId` | `string` | ✅ | |
| `path` | `string` | ✅ | **absolute** |
| `line` | `integer \| null` | optional | line to start reading from, **1-based** |
| `limit` | `integer \| null` | optional | maximum number of lines to read |
| `_meta` | `object \| null` | optional | |

Result `ReadTextFileResponse { content: string ✅, _meta? }`.

**`fs/write_text_file`** — params `WriteTextFileRequest`:

| Field | Type | Required | Notes |
|---|---|---|---|
| `sessionId` | `string` | ✅ | |
| `path` | `string` | ✅ | **absolute** |
| `content` | `string` | ✅ | |
| `_meta` | `object \| null` | optional | |

Result `WriteTextFileResponse { _meta? }` (empty).

The reason these exist rather than the agent just touching the filesystem: the **client may hold
unsaved buffer state** the agent cannot see on disk, and the client can apply its own editing
semantics (undo history, formatting, watchers).

### `terminal/*`

All five methods are gated by the **single** flag `clientCapabilities.terminal: true`.

| Method | Params | Result |
|---|---|---|
| `terminal/create` | `CreateTerminalRequest` — see below | `CreateTerminalResponse { terminalId: string ✅, _meta? }` |
| `terminal/output` | `{ sessionId ✅, terminalId ✅, _meta? }` | `TerminalOutputResponse { output: string ✅, truncated: boolean ✅, exitStatus?: TerminalExitStatus\|null, _meta? }` |
| `terminal/wait_for_exit` | `{ sessionId ✅, terminalId ✅, _meta? }` | `WaitForTerminalExitResponse { exitCode?: integer\|null, signal?: string\|null, _meta? }` |
| `terminal/kill` | `{ sessionId ✅, terminalId ✅, _meta? }` | `KillTerminalResponse { _meta? }` |
| `terminal/release` | `{ sessionId ✅, terminalId ✅, _meta? }` | `ReleaseTerminalResponse { _meta? }` |

`CreateTerminalRequest`:

| Field | Type | Required | Notes |
|---|---|---|---|
| `sessionId` | `string` | ✅ | |
| `command` | `string` | ✅ | |
| `args` | `string[]` | optional | |
| `env` | `EnvVariable[]` | optional | array of `{name, value}` pairs |
| `cwd` | `string \| null` | optional | **absolute** |
| `outputByteLimit` | `integer \| null` | optional | see below |
| `_meta` | `object \| null` | optional | |

`TerminalExitStatus { exitCode?: integer|null, signal?: string|null, _meta? }` — both nullable;
`exitCode` may be null if terminated by a signal, `signal` null if it exited normally. `signal` is a
string name.

`outputByteLimit` semantics: when the limit is exceeded, the **client** truncates **from the
beginning** of the output to stay within it, **MUST** cut on a character boundary so the string
stays valid, and reports `truncated: true` from `terminal/output`.

Lifecycle: `terminal/create` → poll `terminal/output` and/or block on `terminal/wait_for_exit` →
optionally `terminal/kill` → **`terminal/release`** to free the handle. A terminal embedded into a
tool call via `ToolCallContent { type: "terminal" }` must be embedded **before** release.

> **An ACP terminal is not a Ubiq pane.** This is the one place where ACP's model brushes against
> Ubiq's, and the shapes do not match. An ACP terminal is a **one-shot command execution with
> byte-capped captured output and an exit status**. There is no `terminal/write` and no
> `terminal/resize` — no way to send keystrokes, no PTY geometry, no alternate-screen semantics, no
> raw byte stream. It is closer to `Command::spawn` plus output capture than to a pseudo-terminal.
> An agent asking for `terminal/create` wants to run `cargo test` and read the result; it does not
> want an interactive terminal. Implementing `terminal/*` on top of Ubiq's PTY layer is possible but
> is a *narrowing* — do not model an ACP terminal as a pane, and do not expose ACP terminals as
> panes to the user without deciding deliberately what a resize or a keystroke would mean.

### `elicitation/*`

Gated by `clientCapabilities.elicitation.form` and/or `.url` (each `{}` to advertise).

**`elicitation/create`** — request, internally tagged on `mode`, with a **scope** also flattened in.
The scope is one of:

- `ElicitationSessionScope { sessionId ✅, toolCallId?: string|null }` — tied to a session, and
  optionally to a specific tool call (useful when the agent receives an MCP elicitation during a
  tool call and forwards it to the user)
- `ElicitationRequestScope { requestId ✅ }` — tied to a specific JSON-RPC request **outside** any
  session, e.g. during auth or configuration before a session exists

Common field: `message: string` ✅ — human-readable description of what input is needed.

| `mode` | Additional fields |
|---|---|
| `"form"` | `requestedSchema` ✅ — a restricted JSON Schema describing the form fields (`ElicitationSchema`, with `string`/`number`/`integer`/`boolean`/multi-select property schemas, `StringFormat`, `EnumOption`, etc.) |
| `"url"` | `elicitationId: string` ✅, `url: string` ✅ |
| any other string | Open extension point. Values beginning with `_` are reserved for implementation-specific extensions; unknown values not beginning with `_` are reserved for future ACP variants. |

**Response** `CreateElicitationResponse` — internally tagged on `action`:

| `action` | Payload |
|---|---|
| `"accept"` | `ElicitationAcceptAction { content?: map<string, string\|integer\|number\|boolean\|string[]>\|null }` — the user-provided values, matching the requested schema |
| `"decline"` | — |
| `"cancel"` | — |
| any other string | Open extension point, same `_`-prefix rule as `mode` |

`ElicitationContentValue` (the value type inside `content`) is an untagged union of: `string`,
`integer`, `number`, `boolean`, `string[]`.

**`elicitation/complete`** — notification, `{ elicitationId: string ✅, _meta? }`. Sent by the agent
when a URL-based elicitation has completed out of band.

### Errors

`Error` (the JSON-RPC error object):

| Field | Type | Required |
|---|---|---|
| `code` | `integer` | ✅ |
| `message` | `string` | ✅ — a concise single sentence |
| `data` | any JSON | optional |

`ErrorCode` — predefined values (any other integer is permitted):

| Code | Name | Meaning |
|---|---|---|
| `-32700` | Parse error | Invalid JSON received. |
| `-32600` | Invalid request | Not a valid Request object. |
| `-32601` | Method not found | Method does not exist or is unavailable. **Return this for unrecognised `_`-prefixed extension methods.** |
| `-32602` | Invalid params | Invalid method parameter(s). |
| `-32603` | Internal error | Internal JSON-RPC error. |
| `-32800` | Request cancelled | Execution aborted due to a cancellation request or resource constraints. |
| `-32000` | Authentication required | Auth needed before this operation. |
| `-32002` | Resource not found | A resource such as a file was not found. |

`-32000` and `-32002` are ACP-specific, within JSON-RPC's reserved implementation-defined range
(`-32000` … `-32099`).

General rule (`overview.mdx`): successful responses include `result`; errors include `error` with
`code` and `message`; **notifications never receive responses**, success or error.

---

## 12. Extensibility and versioning

*Source: `docs/protocol/v1/extensibility.mdx`, `docs/protocol/v1/initialization.mdx`*

### The `_meta` field

**Every type in the protocol** carries an optional `_meta` of type `{ [key: string]: unknown }` —
requests, responses, notifications, and nested types alike: content blocks, tool calls, plan
entries, capability objects, even `EnvVariable` and `HttpHeader`. It is the sanctioned place for
custom data.

```json
{
  "jsonrpc": "2.0", "id": 1, "method": "session/prompt",
  "params": {
    "sessionId": "sess_abc123def456",
    "prompt": [ { "type": "text", "text": "Hello, world!" } ],
    "_meta": {
      "traceparent": "00-80e1afed08e019fc1110464cfa66635c-7a085853722dc6d2-01",
      "zed.dev/debugMode": true
    }
  }
}
```

Reserved root keys inside `_meta`, for W3C trace-context / OpenTelemetry interop with existing MCP
tooling:

- `traceparent`
- `tracestate`
- `baggage`

**Implementations MUST NOT add custom fields at the root of any spec type.** All unprefixed names
are reserved for future protocol versions. Namespace your `_meta` keys (the docs use
`zed.dev/debugMode` — a reverse-domain or domain-prefixed convention).

The reference Rust implementation deserialises `_meta` with `DefaultOnError`, i.e. a malformed
`_meta` degrades to `None` rather than failing the whole message. Worth mirroring.

### Extension methods

Any JSON-RPC method name **starting with an underscore (`_`)** is reserved for custom extensions and
will never collide with a future protocol version. Standard JSON-RPC semantics apply — requests
carry an `id` and expect a response, notifications omit it.

```json
{ "jsonrpc": "2.0", "id": 1, "method": "_zed.dev/workspace/buffers", "params": { "language": "rust" } }
```
```json
{ "jsonrpc": "2.0", "method": "_zed.dev/file_opened", "params": { "path": "/home/user/project/src/editor.rs" } }
```

If the receiver does not recognise the method it **should** reply with `-32601 Method not found`. To
avoid that, extensions **SHOULD** advertise custom capabilities during `initialize` (inside
`clientCapabilities._meta` / `agentCapabilities._meta`) so callers can feature-detect first.

The schema also models generic `ExtRequest`, `ExtResponse` and `ExtNotification` types as arbitrary
JSON, for SDKs that want a typed escape hatch.

### Open enums

Several enums already accept unknown values, with `_`-prefixed reserved for implementations and
non-prefixed unknowns reserved for future ACP:

- `SessionConfigOptionCategory`
- elicitation `mode`
- elicitation response `action`
- `ErrorCode` (any integer)
- `AuthMethod.type` (defaults to `agent` when unrecognised/absent)

v2 generalises this to **every** enum in the schema. Even in v1, treat every enum as
non-exhaustive on the read side.

### Versioning and negotiation rules

- `protocolVersion` is a single integer, bumped **only** for breaking changes.
- **Non-breaking features are introduced via capabilities.** Adding a capability is never a breaking
  change.
- Both sides **MUST** treat any capability omitted at `initialize` as **UNSUPPORTED**.
- Both sides **SHOULD** support all combinations of their peer's capabilities.
- Capabilities are high-level and not attached to a specific base concept. They may gate a method, a
  notification, a *subset of a method's parameters*, or merely signal an implementation behaviour.
- Version negotiation is one round: client sends its latest, agent echoes or downgrades to its
  latest, client disconnects if it can't speak that.

---

## 13. Version status

### Protocol

- **v1 is the current stable protocol version.** `agent-client-protocol-schema/src/version.rs`
  defines `V0` (pre-release, "should likely be treated as unsupported"), `V1`, and
  `LATEST = V1`. `V2` exists only behind the cargo feature `unstable_protocol_v2`, and `LATEST` is
  deliberately made **unavailable** when that feature is enabled so that code opting into v2 must
  name `V1` or `V2` explicitly.

- **v2 is a published Draft.** Announced 2026-07-20 by Ben Brandt (Zed Industries / ACP lead
  maintainer) in `docs/announcements/acp-v2-draft.mdx`. In its own words:

  > **v2 is a Draft** … **However, various pieces can, and will, change before stabilization.** …
  > As you start implementing it, gate your implementation behind the version negotiation **AND**
  > feature flags. Don't ship it by default in production until we are closer to stabilization. …
  > Adding v2 support should not mean dropping v1. v1-only peers will remain common for some time,
  > so implementers should support both versions side by side.

- **There is also a v1 draft/unstable track** — `schema/v1/schema.unstable.json`,
  `schema/v1/meta.unstable.json`, and docs under `docs/protocol/v1/draft/`. These are *additive* to
  v1 and not yet stabilised: `providers/*` (auth providers), `session/fork`,
  `mcp/connect|message|disconnect`, `nes/*` (next-edit-suggestions), `document/did*` (LSB-style
  text-document sync). The repo's `docs/announcements/` directory tracks which features have been
  promoted from draft to stable — e.g. `session-list-stabilized`, `session-resume-stabilized`,
  `session-close-stabilized`, `elicitation-stabilized`, `boolean-config-option-stabilized`,
  `model-config-category-stabilized`, `logout-method-stabilized`, `message-id-stabilized`,
  `additional-directories-stabilized`, `session-usage-stabilized`,
  `request-cancellation-stabilized`, `session-config-options-stabilized`.

### What v2 changes (that matters for a client implementation)

*Source: `docs/announcements/acp-v2-draft.mdx`, `schema/v2/meta.json`. Full detail lives in
`docs/protocol/v2/migration.mdx` (~50 KB) — read that before writing any v2 code.*

1. **Beyond the turn.** `session/update` notifications may flow at **any** point in the session; the
   `session/prompt` response now means "the message was acknowledged", not "the turn is over". The
   agent replays the user message where it inserted it, and can signal when it is **idle** / ready
   for new input. This enables queueing, steering, background work reporting, and **multiple clients
   observing the same session**.
2. **Method renames and removals.** `authenticate` → `auth/login`; `logout` → `auth/logout`;
   **`session/load` removed** (use `session/resume`); **`session/set_mode` removed** (config options
   only). v2 stable agent methods per `schema/v2/meta.json`: `initialize`, `auth/login`,
   `session/new`, `session/set_config_option`, `session/prompt`, `session/cancel`, `session/list`,
   `session/delete`, `session/resume`, `session/close`, `auth/logout`.
3. **`fs/*` and `terminal/*` are gone from the v2 client method set.** `schema/v2/meta.json`
   `clientMethods` contains only `session/request_permission`, `session/update`,
   `elicitation/create`, `elicitation/complete`. However
   `agent-client-protocol-schema/src/v2/terminal.rs` still exists, so terminals appear to be being
   *re-homed* rather than deleted. **Verify against `docs/protocol/v2/migration.mdx` before relying
   on this.**
4. **Uniform patch semantics.** Messages, tool calls and terminal output are all patched by stable
   ids with the same rules: omitted stays unchanged, `null` clears, a value replaces, and chunks
   append. **`messageId` becomes required**, so messages can not only stream but also be updated or
   replaced (redaction, corrections).
5. **Streaming tool call content.** The message-streaming pattern is applied to tool call content,
   so tool calls no longer have to buffer and resend their whole content on every update.
6. **Diff overhaul.** `oldText`/`newText` is **replaced** by structured file changes expressing add,
   delete, modify, move, copy, plus binary and non-text cases; with an optional **`git_patch`** for
   rendering.
7. **More flexible permission requests.** A permission prompt carries its own required `title` and
   optional `description`, and an extensible **`subject`** instead of a hard-wired `toolCall` — so
   permissions can cover terminal commands and other objects, and the prompt text no longer
   overloads the tool call's own title and content.
8. **Forward compatibility by default.** Enum-like values across the whole schema accept unknown
   variants with a `_` prefix.

### Package versions (checked 2026-09-03)

| Package | Registry | Version | Notes |
|---|---|---|---|
| `agent-client-protocol` | crates.io | **2.0.0** (published 2026-07-23) | The Rust SDK, both sides (`Agent` and `Client` traits). **SDK semver ≠ protocol version** — 2.0.0 does *not* mean protocol v2. Source now at `github.com/agentclientprotocol/rust-sdk`. Prior: 1.3.0, 1.2.0, 1.1.0, 1.0.1, 1.0.0. |
| `agent-client-protocol-schema` | crates.io | **1.7.0** (2026-08-20) | Generated wire types only, no runtime. This is the crate the `schema/` and `docs/` in the protocol repo are generated from. |
| `@agentclientprotocol/sdk` | npm | **1.4.0** | The **current** TypeScript SDK. Fluent `agent()` / `client()` API. |
| `@zed-industries/agent-client-protocol` | npm | 0.4.5 | **Superseded** — the package was renamed. Its `AgentSideConnection` / `ClientSideConnection` classes are deprecated in favour of the fluent API, kept only for backwards compatibility. |
| `agent-client-protocol-json-schema-v1` | *(not published)* | 1.21.0 | Version marker for the v1 JSON-Schema GitHub release artifacts. **Not on crates.io** — it exists only as a `Cargo.toml` version stamp. |
| `agent-client-protocol-json-schema-v2` | *(not published)* | 2.0.0-alpha.3 | Same, for the v2 draft schema artifacts. The v2 announcement says SDK authors should generate against schemas "published in the repository releases as `v2.0.0-alphaX` alongside v1". |

Known production implementations: Zed (client), Gemini CLI (agent —
`packages/cli/src/acp` in `google-gemini/gemini-cli`, cited by the docs as a complete reference
implementation).

Community bindings documented upstream: Python, Java, Kotlin (`docs/libraries/`).

---

## 14. Practical notes for implementers

Six things that will bite when defining Rust wire types from this document.

**1. Casing is not uniform in the way you might assume.**
Object **keys** are `camelCase`; discriminator **values** are `snake_case`; method names are
`namespace/snake_case`. `#[serde(rename_all = "camelCase")]` on the struct plus snake_case enum
variant identifiers reproduces it exactly. Do **not** read field names off the upstream Rust source
— those are snake_case identifiers that serde renames. Read the JSON Schema.

**2. Every tagged union is internally tagged with the payload flattened, and the tag key differs per
union.**

| Union | Tag key |
|---|---|
| `ContentBlock` | `type` |
| `ToolCallContent` | `type` |
| `McpServer` | `type` |
| `SessionConfigOption` | `type` |
| `AuthMethod` | `type` |
| `SetSessionConfigOptionRequest` value | `type` |
| `SessionUpdate` | **`sessionUpdate`** |
| `RequestPermissionOutcome` | **`outcome`** |
| `CreateElicitationRequest` | **`mode`** |
| `CreateElicitationResponse` | **`action`** |

In serde terms: `#[serde(tag = "...")]`. **Not** `#[serde(tag = "...", content = "...")]` — there is
no nested payload object anywhere. `EmbeddedResourceResource` and `SessionConfigSelectOptions` are
the exceptions: they are **untagged**, discriminated structurally (`text` vs `blob`; flat array vs
grouped array).

**3. Three unions have an implicit default variant when the tag is absent.**

- `McpServer` with no `type` ⇒ **stdio**
- `AuthMethod` with no `type` ⇒ **agent**
- `SetSessionConfigOptionRequest` with no `type` ⇒ **string value**

A `#[serde(other)]`-style fallback or an untagged wrapper is needed; a plain `tag` with all variants
requiring the tag will reject valid messages.

**4. `{}` vs `null` vs omitted is load-bearing for the newer nested capabilities.**
For `sessionCapabilities.{list,delete,resume,close,additionalDirectories}`, `auth.logout`,
`elicitation.{form,url}`, and `session.configOptions.boolean`: omitted == `null` == unsupported,
`{}` == supported. Model as `Option<CapStruct>`, never `bool`, or you lose the ability to add
sub-fields later — which is the entire point of the encoding. And remember: **`elicitation: {}` does
not mean form support** (unlike MCP).

**5. Every id is an opaque string. Newtype them; never parse them.**
`SessionId`, `ToolCallId`, `TerminalId`, `MessageId`, `ElicitationId`, `PermissionOptionId`,
`SessionModeId`, `SessionConfigId`, `SessionConfigValueId`, `SessionConfigGroupId`, `AuthMethodId`
are all bare `string` in the schema. `RequestId` is the only structured one (`null | integer |
string`). Note that `optionId` in a `switch_mode` permission request happens to be a mode id — that
is the agent's business, and the client must echo it back unchanged without interpreting it.

**6. If you implement only one configuration mechanism, implement config options.**
`session/set_config_option` + `configOptions` + `config_option_update` is the forward path;
`session/set_mode` + `modes` + `current_mode_update` is deprecated and already absent from v2.
Reading `modes` as a fallback is cheap and worth doing for older agents, but do not build UI around
it. And when you do read `current_mode_update`, serialise `currentModeId` while accepting `modeId`
as an alias, because the upstream prose example is wrong.

**Two more, free of charge:**

- **All paths are absolute and all line numbers are 1-based** — enforce this at your boundary rather
  than discovering it from a misbehaving agent.
- **`session/cancel` never produces an error response.** The agent must answer the outstanding
  `session/prompt` with `stopReason: "cancelled"`, and the client must answer every outstanding
  `session/request_permission` with `outcome: "cancelled"`. A cancellation that surfaces as a
  JSON-RPC error is a bug on whichever side produced it.
