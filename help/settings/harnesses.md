---
id: settings-harnesses
title: Harnesses and isolation
summary: The Harnesses and Isolation sections — which harnesses are available, their accounts, and confining an agent.
keywords: [harness, accounts, isolation, confinement, sandbox, home, acp, registry, command, grants, identities]
order: 40
status: current
related: [settings, agents-accounts, concepts-harnesses, agents-definitions]
---

**Harnesses** lists the agent programs on this machine and the accounts they run as; **Isolation**
decides what an agent may reach. Both are sections of [Settings](settings). What a harness is is in
[Harnesses](concepts-harnesses), and the accounts are explained in [Accounts](agents-accounts).

## The parts

| Setting | What it changes for you |
|---|---|
| **Installed** | One row per harness Ubiq knows, dimmed when it is not on this machine, with the plan usage of its first signed-in account under it |
| **Command** / **Custom command** | Sets the command Ubiq starts that harness with; it reads **Custom command** once you have set one |
| What this harness said it can do | An info button on harnesses added from the registry, listing what the harness reported it supports |
| **ACP registry** | Finds more harnesses from the Agent Client Protocol registry; **Check** looks for them on this machine, **Locate…** points Ubiq at one's command, **Download**, **Install** and **Update** fetch it, and **Show … more** lists those not found |
| **Identities** | Your accounts, grouped by name, each with its harnesses and their sign-in state |
| **Add harness** | Opens the sign-in dialog to create an account and sign a harness in |
| **Enable … ACP** | One switch per tool that has both a native and a registry version; off, only the native one is offered, which states more about context and spend |
| **Confine agents to their project** | An agent reads and writes only its project's folder plus a throwaway configuration of its own; off, it can reach the whole machine |
| **The agent's home** | Which home folder a confined agent runs with: **Your home**, **A fresh one each run**, or **A named home** kept between runs |
| **Extra grants** | Folders an agent may reach beyond its limits, such as a toolchain or a shared cache; click a grant to switch between `ro` and `rw`, remove it with its remove button, add a path with the plus button |

## What you can do here

- Add an account and sign a harness in: [Accounts](agents-accounts).
- Save a harness, account and model as one pick: [Agent definitions](agents-definitions).
- Choose how much an agent may do without asking: [Switch a harness's permission mode](how-to-switch-permission-mode).
- Set up accounts for several clients end to end: [Accounts and agent definitions for several clients](guides-accounts-and-definitions).

## When something goes wrong

- **A harness is dimmed.** It is not found on this machine; install it, or set **Command** to where
  it lives.
- **A sign-in has expired.** Open its row under **Identities** and use **Re-authenticate**.
- **A harness is missing from the start menus.** If it is a registry version of a tool that has a
  native one, turn on its **Enable … ACP** switch.
- **Confinement is off and says why.** It is available on macOS and Windows only; agents on other
  systems run unconfined.
- **A confined agent cannot find its tools.** Its home holds nothing you installed; use **Your
  home**, or add the tool's folder under **Extra grants**.
