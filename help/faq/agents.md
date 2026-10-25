---
id: faq-agents
title: Agent questions
summary: What people ask about starting, running and stopping agents.
keywords: [agent, harness, conversation, stop, fork, permission]
order: 10
related: [agents-starting, agents-chat]
---

## Why does closing a chat tab not stop my agent?

A chat tab's **×** hides it, because the conversation belongs to Ubiq, not to the tab, and the agent keeps working while it is out of sight. To end it, open the conversation's three-dots menu and pick **Close**, which asks first because it deletes the transcript. See [Hide or close a tab](how-to-hide-or-close-a-tab).

## How do I get back an agent whose tab I closed?

Click its row in the sidebar of Agents mode, or pick **Attach existing agent** from **New agent**. The title bar's **Agents not on screen** chevron lists the same agents. See [Agents mode](agents-starting).

## Can I stop an agent asking for permission every time?

Yes. Choose a different mode for the agent from the picker beside **Send**, in the harness's own words, or pick **Accept all** from the conversation's three-dots menu to answer every request in that conversation for you. **Accept all** starts off in each conversation and reads **Stop accepting all** while on. See [Switch a harness's permission mode](how-to-switch-permission-mode).

## Why is Fork dimmed?

**Fork** works only while the agent is idle, and only on harnesses that keep a history of their own, such as Claude Code and Codex. Wait for the turn to end, or pick **Stop** first. See [Resume or fork a conversation](how-to-resume-or-fork-a-conversation).

## What is the difference between Start and Start in terminal?

**Start** runs the agent as a conversation you read as a transcript, in a column or chat tab. **Start in terminal** runs the same harness as a terminal pane showing its own screen, and it ignores the initial prompt. See [The new agent dialog](agents-new-agent).

## Why did my agent's name change?

After the agent answers its first prompt, it takes a short title of its own if a provider is set up under **Settings**, then **Assistance**. A name you typed yourself is never overwritten. See [The agents columns and sidebar](agents-columns).

## Does Ubiq choose the model for me?

No. Each harness brings its own models, and you choose among them for each agent, in the new agent dialog or beside **Send**. See [Harnesses](concepts-harnesses).
