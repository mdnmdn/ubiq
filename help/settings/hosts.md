---
id: settings-hosts
title: Hosts, SSH and drones
summary: The Hosts, SSH and Drones sections — the machines you can reach, their SSH profiles, and stopping a drone.
keywords: [hosts, ssh, ssh profile, drone, remote, stop drone, key file, password, passphrase, lifetime]
order: 70
status: current
related: [settings, remote-overview, remote-connect]
---

Three sections of [Settings](settings) cover working on other machines. **Hosts** lists the machines
you can reach and picks where new work goes, **SSH profiles** keeps how to sign in to them, and
**Drones** finds and stops the helpers Ubiq left running there. Using them is in
[Remote hosts](remote-overview).

## The parts

| Setting | What it changes for you |
|---|---|
| **Active host** | The machine a new pane or a new project lands on by default; it never moves one that already exists |
| **Saved hosts** | Every host you have connected to, with a chip (**attached**, **reconnecting**, **last attempt failed** or **saved**), **Disconnect** while attached, and **Forget** |
| **Open remote hosts** | Opens the [Remote hosts](remote-overview) manager |
| **Add profile…** | Opens the form for an SSH profile |
| **Name**, **Method** | In that form: what you call the target, and how it signs in: **Agent**, **Key file**, **Password** or **ssh config alias** |
| **Host**, **Port**, **User** | The address to dial; a port left empty means 22, and a user left empty is the one `ssh` would pick; an alias asks only for the **Alias** |
| **Key file** | The private key's path on this machine, and a passphrase if the key has one |
| A profile row | Its name, address and method, with **Forget secret** for a passphrase or password you filed, **Edit** and **Remove** |
| **Drones** | One block per saved host reached over SSH, with **Check** to try the connection and **Refresh** to list what runs there |
| A drone row | The folders it serves, how long it has been up, its panes and its linger time, with **Stop** |

## What you can do here

- Reach a machine using a profile: [Connecting to a host](remote-connect).
- Work on it from start to finish: [Work on a remote machine](guides-remote-machine).
- Choose where a project's folder lives from then on: [Project settings](settings-project).

## When something goes wrong

- **A password or passphrase is not remembered.** It is stored in this machine's credential store and
  never shown again; leave the box blank on **Edit** to keep it, and use **Forget secret** to clear it.
- **Drones says no saved ssh hosts.** Connect to a host with the **Session** or **Managed** lifetime;
  it then appears here.
- **A drone row says version differs.** It was started by another build; **Stop** it, then connect
  again so a matching one is started.
- **Stopping a drone ended my panes.** **Stop** asks first, because every pane the drone holds ends
  with it.
- **No SSH profiles in the connect dialog.** Add one here with **Add profile…**. A machine already
  set up with an SSH agent needs only a profile naming it.
