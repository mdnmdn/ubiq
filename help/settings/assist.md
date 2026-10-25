---
id: settings-assist
title: Assistance
summary: The Assistance section — the backend that names conversations and writes short prose, and the API providers behind it.
keywords: [assist, assistance, api provider, api key, model, name conversations, on-device, commit message]
order: 50
status: current
related: [settings, agents-chat, concepts-conversations]
---

**Assistance** is where you let Ubiq write a short line of text for you, such as a commit message from
what is staged, or a title for a conversation's tab. It is off by default and only ever suggests: a
suggestion fills a field you can edit. The section is part of [Settings](settings); a chip at the top
says whether this machine can answer at all.

## The parts

| Setting | What it changes for you |
|---|---|
| Status chip | **Checking what this host can do…**, then what is running, or **Assistance is unavailable here** with the reason under it |
| **Suggestions** | Which backend writes a suggestion: **Off** calls no model, **On-device** uses this machine's own model where it has one, and each API provider you add appears by name |
| **Name conversations** | Writes a title for a conversation's tab from its opening exchange, with a short summary on hover, once per conversation; dimmed while **Suggestions** is **Off** |
| **Add provider…** | Opens the form for a model behind someone else's API |
| **Kind**, **Name**, **Base URL**, **API key** | In that form: the protocol the endpoint speaks, what you call it, the endpoint when it is not the kind's own, and the key, which is kept in this machine's credential store and never shown again |
| Models | On an existing provider, the **Fast** model and optionally a **Smart** one, chosen from the provider's list or typed by hand |
| **Test** | Opens a dialog to run a prompt against the provider's **Fast** or **Smart** model, with **Run test** and **Close** |
| **Edit**, **Remove** | Changes a provider, or removes it after a confirm along with its key |

## What you can do here

- Turn naming on for conversations you start later: [Conversations](concepts-conversations).
- See a named conversation in its tab: [The chat panel](agents-chat).
- Add a hosted provider next to a local one; several providers are ordinary, and the **Name** is what
  tells them apart.

## When something goes wrong

- **The chip says unavailable.** The reason is written under it. Where it is only that the setting
  is off, **Suggestions** can be switched here; where the machine cannot run it, add an API provider
  instead.
- **A provider reports itself unavailable.** It has no model yet. Save it, then press **Edit** and
  choose a **Fast** model; type an id if the provider will not list its own.
- **I typed a key wrongly.** Press **Edit** and type the right one; leaving the box blank keeps the
  filed key.
- **Removing a provider turned suggestions off.** If it was the backend, assistance falls back to
  calling no model.
- **A suggestion never arrives.** Press **Test** to see what the provider answers.
