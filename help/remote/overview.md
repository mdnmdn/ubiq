---
id: remote-overview
title: Remote hosts
summary: Drones and other hosts you can reach from this window, and the manager that lists them.
keywords: [remote, drone, host, manager, ssh, reconnect, forget, another machine]
order: 10
status: current
related: [remote-connect, settings-hosts, guides-remote-machine]
---

A remote host is another machine Ubiq works on from this window. Once you are connected, its
folders open as projects beside your local ones, and an agent or a shell you start there runs on
that machine, with its files, its search and its terminals in the same window as everything local.
The **Remote hosts** manager lists the hosts Ubiq remembers and the ones you are attached to.

Open it from the **More** chevron beside the **Settings** button ![Settings](../img/ui/titlebar-settings.png)
in the title bar, with **Remote hosts**, or from **Settings**, then **Hosts**, with **Open remote
hosts**.

## The parts

| Part | What it does |
|---|---|
| Host row | The host's name and its address, with a note of what Ubiq last learned about the machine |
| Status chip | **attached**, **reconnecting**, **last attempt failed** or **saved** |
| **Test** | Tries to reach the host without attaching to it, and writes the outcome on the row |
| **Connect** / **Disconnect** / **Reconnect now** | Attaches to a saved host, ends an attachment and closes the panes it was running, or retries a dropped link at once |
| **Rename** | Changes the name shown; the address and the saved token are untouched |
| **Forget** | Removes the host from the list |
| **New connection** | Opens the dialog to reach a host; see [Connecting to a host](remote-connect) |
| A row with only **Disconnect** | A live connection that was never saved, such as a one-off |

## What you can do here

- Reach a machine for the first time: [Connecting to a host](remote-connect).
- Work end to end on another machine: [Work on a remote machine](guides-remote-machine).
- Open a folder on a host as a project: pick **Open a project on** and the host's name from the **More
  ways to add** chevron beside **Add project**, or **Open remote project…** when none is saved. See
  [The title bar](workbench-titlebar).
- Choose which host new panes and projects go to by default, manage SSH profiles and stop drones
  that outlive a connection: [Hosts, SSH and drones](settings-hosts).

## When something goes wrong

- **The chip says last attempt failed.** Press **Test** to see why, then **Connect** again; the row
  keeps the address, so nothing needs retyping.
- **The chip says reconnecting.** Ubiq is retrying a dropped link; **Reconnect now** retries at once.
- **Disconnect closed my panes.** Disconnecting ends every pane that host was running. A link that
  merely drops is different: a drone started with **Session** or **Managed** keeps its panes for you
  to reattach to.
- **A host will not come back.** **Forget** it and use **New connection** with the address again.
