---
id: remote-connect
title: Connecting to a host
summary: The dialog that reaches a host or a drone — by address and token, or through an SSH profile.
keywords: [connect, address, token, ssh profile, folder, lifetime, https, certificate, drone]
order: 20
status: current
related: [remote-overview, settings-hosts, guides-remote-machine]
---

The **Connect to a remote host** dialog attaches this window to another machine. Open it with
**New connection** in [Remote hosts](remote-overview), or from the **More** chevron beside the
**Settings** button ![Settings](../img/ui/titlebar-settings.png) with **Connect to a remote host**.
Pick how you reach the machine at the top: **Ubiq host** for a machine where Ubiq is already
running as a host, or **Drone over ssh** for any machine you can sign in to with SSH.

## The parts

| Part | What it does |
|---|---|
| **Ubiq host** | Reaches a machine that is running `ubiq --serve`, by address and token |
| **Address** | A host, or `host:port`; pasting the whole connection string that machine printed fills the token too |
| **Token** | The token that machine printed |
| **Protocol** | **http**, or **https** when the host was started with a certificate and key |
| **Trust certificate** | Skips checking a self-signed certificate; only needed with https, and shown on the saved entry afterwards |
| **Drone over ssh** | Starts Ubiq's helper on the machine over SSH, uploading it first if the machine has none |
| **SSH profile** | Which saved profile says where to connect and how to sign in, each with a **Check** that tries it; none is listed until you add one |
| **Folder** | The folder on that machine the drone serves, written as that machine spells it; empty is the login directory |
| **Lifetime** | **Attached** ends with this connection; **Session** survives a dropped link, but not Ubiq quitting; **Managed** survives both, until you stop it |
| **Connect** / **Cancel** | Starts the connection, or closes the dialog |

## What you can do here

- Add the SSH profile the dialog asks for: [Hosts, SSH and drones](settings-hosts).
- See and manage the hosts you have reached: [Remote hosts](remote-overview).
- Work end to end on another machine: [Work on a remote machine](guides-remote-machine).

## When something goes wrong

- **The dialog says Not connected.** Read the reason it gives, correct the field and press **Try
  again**; **Close** abandons it.
- **No SSH profiles listed.** Add one in **Settings**, then **SSH profiles**, then come back.
- **A drone keeps running after you quit.** That is what **Managed** does; stop it under
  **Settings**, then **Drones**.
- **https refuses a certificate.** Tick **Trust certificate** only for a host you control.
