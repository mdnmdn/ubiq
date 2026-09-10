---
id: inbox-rdp-tunnel
title: Tunneling application traffic over RDP (full Rust)
kind: proposal
status: proposal
summary: A design for carrying the router's own protocol over a Dynamic Virtual Channel inside an RDP session — a headless IronRDP client with no GUI, and a Windows session worker using the WTS DVC APIs — when only RDP (TCP/3389) is reachable and no external tool such as FreeRDP or rdp2tcp may be used.
read_when: you are deciding how application traffic reaches a Windows host when only RDP is reachable, or how a Dynamic Virtual Channel is opened and framed from both ends
updated: 2026-09-07
---

# Tunneling application traffic over RDP (full Rust)

## Purpose

Allow the existing Rust client and server to exchange traffic when only RDP (TCP/3389) is reachable, without FreeRDP, rdp2tcp, or other external tools.

The RDP session is used as a **byte pipe** (Dynamic Virtual Channel). The application protocol and router stay unchanged.

## Architecture

```
Client host                         Windows host
───────────                         ────────────
Rust client                         Session worker (already installed)
  IronRDP (headless)                  embedded router
  DRDYNVC + custom DVC                WTSVirtualChannelOpenEx (DVC)
  mux (yamux / existing framing)  ↔   same mux
        RDP + NLA (TCP/3389)
```

| Side | Role |
|---|---|
| Client | Opens the RDP connection, registers a Dynamic Virtual Channel (DVC), multiplexes router streams on that channel. No GUI. |
| Server | Already present on the Windows host. A **session-scoped** worker embeds the router and attaches to the same DVC. |

Static virtual channels (rdp2tcp-style, 8-character names, rdesktop add-ins) are not used.

## Why DVC

- Channel names can be long and namespaced (example: `YourApp::Router`).
- The channel is created after the session exists.
- On Windows the server API is file I/O (`ReadFile` / `WriteFile`) after `WTSVirtualChannelOpenEx`.
- IronRDP supports DVC via `DrdynvcClient` and `DvcProcessor`.

## Connection sequence

1. Client starts a headless IronRDP session (TLS + NLA).
2. Windows creates an interactive RDP session and logs the user on.
3. The session worker starts (logon task, Run key, or a system service that launches a process *into that session*).
4. Worker opens the DVC with the agreed name.
5. Client `DvcProcessor::start` runs; both sides handshake.
6. Application framing / mux runs on the channel; the router is unaware of RDP.

If the worker is not up yet, the client retries channel open for a short window.

## Headless client (IronRDP)

- Library only: `ironrdp-connector`, `ironrdp-session`, `ironrdp-dvc`. Graphics decoding is optional and is never displayed.
- Drive TCP yourself; no window toolkit.
- Use a minimal desktop size and performance flags that disable wallpaper / themes.
- Drain or ignore bitmap updates.
- Keep the session alive (periodic input or IronRDP echo/heartbeat) so the host does not lock and drop the channel.
- NLA credentials (password or Kerberos) are required.

Outbound router bytes are turned into DVC messages and encoded on the active session loop. Inbound DVC payloads are pushed into the mux.

## Windows worker (embedded router)

Must run **inside the RDP session**. Session 0 services cannot open the channel.

```
WTSVirtualChannelOpenEx(
    WTS_CURRENT_SESSION,
    "YourApp::Router",
    WTS_CHANNEL_OPTION_DYNAMIC | priority flags)
WTSVirtualChannelQuery(..., WTSVirtualFileHandle, ...)
DuplicateHandle → overlapped ReadFile / WriteFile
```

Reads include `CHANNEL_PDU_HEADER` and may be fragmented; reassemble before handing bytes to the mux.

Recommended process model:

- Preferred: per-session worker started at Remote Desktop logon.
- Acceptable: system service that on `WTS_SESSION_LOGON` starts the worker with `CreateProcessAsUser` in that session.
- Invalid: a single service in session 0 waiting on `WTS_CURRENT_SESSION`.

## Protocol layering

```
application / router
        ↑
mux (yamux or existing stream mux)
        ↑
DVC byte stream (reassembled)
        ↑
RDP (DRDYNVC over the session)
```

Keep a `ByteTransport` (read / write / close) so the router can use TCP in development and DVC in production.

Do not adopt rdp2tcp’s controller language (`t LHOST LPORT …`, SOCKS CLI). Only the idea of “one virtual channel = one pipe” is reused.

## Workspace layout (suggested)

```
protocol/           shared framing
router/             platform-agnostic
transport-dvc/      IronRDP DvcProcessor (client) + WTS DVC (Windows)
client/             headless IronRDP session loop
server-windows/     session worker, cfg(windows)
```

## Operational constraints

- Channel name must match exactly on both ends.
- Group Policy may block unlisted DVCs; allow the name if required.
- Throughput and latency follow RDP, not raw TCP.
- Session drop tears down the channel; reconnect by opening a new RDP session, not by resuming the old mux.
- The Windows worker must be installed beforehand. The DVC cannot be used to copy the worker on first contact (chicken-and-egg).
- One RDP session maps to one worker and one DVC; multiple logical tunnels are muxed on that channel.

## Out of scope

- FreeRDP / xfreerdp / rdesktop add-ins
- rdp2tcp client, `rdp2tcp.py`, `rdpupload`
- Implementing a full remote desktop UI
- Attaching the router from session 0 without impersonating the RDP user

## Summary

The client originates a headless IronRDP connection and exposes a custom DVC. A Windows session worker, already installed and embedding the router, attaches to that DVC with the WTS APIs. Application traffic is multiplexed over the channel. No external RDP tooling is required, provided the worker runs in the session the client just created.