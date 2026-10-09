---
id: prod-features
title: Ubiq features
kind: product
status: current
summary: What a person can do with Ubiq, grouped by area — agents and chat, panes and layout, projects, the IDE, Git, knowledge base, database, tasks and missions, teams, remote hosts, connectors, notifications and settings — each pointing at the document that owns it.
read_when: you want the list of what Ubiq does, or you are checking whether a capability exists before proposing it
updated: 2026-10-09
verified: 2026-10-09
review_cycle: quarterly
depends_on: [prod-overview, feat-workbench, feat-chat, feat-panes, feat-sessions, feat-workbench-agents, feat-workbench-ide, feat-workbench-git, feat-workbench-db, feat-workbench-tasks, feat-workbench-teams, feat-drone, feat-connectors, feat-notifications, feat-logs, feat-stats]
---

# Ubiq features

Ubiq is an agentic workspace: one window that runs several AI coding agents side by side, as real
terminals or as conversations, next to the project's files, history, documents, database and task
board. This page lists what the shipped build lets a person do, area by area. Each area names the
document that owns the detail; the reasons behind the product are in [`overview.md`](./overview.md).

## Agents and harnesses

Owned by [Agents mode](../features/workbench-agents.md) and [Sessions and workspaces](../features/sessions-and-workspaces.md).

- **Several harnesses at once.** Run Claude Code, Codex, Gemini CLI, opencode and Copilot CLI side by side, each unchanged.
- **New agent form.** Pick a harness and the identity it signs in as, or a saved setup, plus model, effort level and permission mode, then start it.
- **Terminal or conversation.** Start the same run as a full-screen terminal or as a transcript with a composer.
- **Agent definitions.** Save a harness, model, skills and tool servers as a named setup and start it again in one pick.
- **Skills and tool servers per run.** Choose which skills and MCP servers an agent is launched with, instead of inheriting whatever is installed.
- **Agent columns.** Lay up to eight agents out as parallel columns, group several into tabs in one column, and drag tabs between columns.
- **Bench and sidebar.** Hide an agent from the screen without ending it, find it again on the bench, and list every conversation the window holds.
- **Per-agent menu.** Fork, stop, abort, unload, hide or close a live agent from its three-dots menu, and read its details.
- **Auto-named conversations.** A conversation takes a short name from its opening exchange.
- **Isolated runs.** Each run gets its own throwaway configuration, so the user's real harness config stays untouched; a run's record is kept after it ends.

## Conversations and chat

Owned by [The chat panel](../features/chat.md).

- **Chat tabs.** Open as many chat tabs as needed, move them anywhere in the window, and attach each to a different run; closing a tab ends nothing.
- **Transcript.** Read what an agent was asked and concluded, with its tool calls drawn as blocks.
- **Permission prompts.** Allow or deny what an agent asks to do, in the transcript.
- **Structured questions.** Answer questions an agent poses as a form.
- **Attachments.** Add files and images to a message.
- **Turn controls.** Send, enqueue a message mid-turn, or stop the turn, from one control.
- **Run readouts.** See the run state, activity, context use and what the turn has spent in the footer.

## Accounts, profiles and permissions

Owned by The workbench (settings) and [Agents mode](../features/workbench-agents.md).

- **Identities per harness.** Register several accounts for one harness and choose which one a run uses; accounts hold references to credentials, never the credentials.
- **Plan usage per account.** See how much of an account's plan is left, under the harness it belongs to.
- **Permission modes.** Choose how freely an agent may act when it starts.
- **Isolation.** Decide whether an agent is confined, whose home it runs in, and which extra folders it may reach.

## Panes, panels and layout

Owned by [Panes and terminals](../features/panes-and-terminals.md) and The workbench (the dock and rail).

- **Real terminal panes.** Each pane is a full terminal: colours, full-screen programs, raw keystrokes, paste and mouse.
- **One focus.** Exactly one pane receives keystrokes while the others keep drawing.
- **Correct resize.** Resizing a pane tells the harness, so the screen never corrupts.
- **Shells and harnesses from one menu.** The `+` opens the default shell; the chevron lists every installed shell and every harness.
- **Hide or Close.** A tab's × either detaches the pane and leaves the agent running or ends it, as the setting says; the tab menu offers both.
- **Exit closes the tab.** An agent that exits takes its pane with it.
- **Rename tabs.** Type over a pane's title.
- **Dock.** Arrange panels in the left, right, bottom and centre regions, drag them between regions, and have the arrangement come back.
- **Activity rail.** Switch the centre between modes (IDE, Git, Agents, Teams, Tasks, KB, DB, Control and more).
- **Run tools.** Run named commands and a project's runner targets from the titlebar.

## Projects and workspace sessions

Owned by [The workbench](../features/workbench.md) and [Sessions and workspaces](../features/sessions-and-workspaces.md).

- **Project picker.** Add a folder, clone a repository, or open a remote project; search, locate a moved folder and forget a project.
- **Clone from a URL.** Paste a repository URL anywhere the navigator takes input to clone it, optionally as an ephemeral project.
- **Project colours and window letters.** Each project wears a colour; several windows are told apart by letter, and a project is open in one window at a time.
- **Sessions.** A named piece of work with a folder that outlives the agents in it; detach without ending anything and reattach later.
- **Persistent conversations.** Mark a conversation to survive a restart and resume it.
- **Project settings.** Per-project agent definitions, skills, tool servers, task sync, indexing depth and where the project lives.
- **Navigator and bookmarks.** Jump to any view, file or item from one field, go back and forward, and bookmark a place.
- **Temporary projects.** Drop a folder onto the editor to open it at once.
- **Web export.** Open the project in a browser from the titlebar.
- **Window capture.** Photograph the window into an editable picture.

## IDE: explorer, editor and search

Owned by [IDE mode](../features/workbench-ide.md).

- **File explorer.** Browse the project, create, rename, move, copy, cut, paste and delete paths, with git colours and badges.
- **Drag and drop.** Move paths by dragging; drop files from the system file manager to copy them in.
- **Live updates.** Changes on disk appear without a refresh.
- **Editor tabs.** Open files as tabs with preview-on-click, pinning, soft wrap and a dirty-tab guard.
- **Viewers by kind.** Text, Markdown, images, diagrams and drawings each open in a fitting viewer.
- **Markdown.** Edit, preview or split with linked scrolling; choose reading width and density; use a minimap and a heading navigator.
- **Diagrams.** Draw Mermaid, Excalidraw and draw.io files, edit the drawing formats in place, and zoom and pan.
- **Image editor.** Annotate any picture and save a flattened copy.
- **Untitled buffers.** Open a new buffer and save it where you choose.
- **Diff against HEAD.** Open a file's change as a tab.
- **Search panel.** Search file contents across the project.

## Git

Owned by [Git mode](../features/workbench-git.md).

- **Refs explorer.** Branches, remotes, tags, stashes and submodules, across every repository a project holds.
- **History.** Page through commits with painted branch lanes, and compare two commits.
- **Changes.** Review conflicted, staged and unstaged files, stage and unstage, discard, and commit from the commit box.
- **Diffs.** Read a changed file's diff under the lists.
- **Ref actions.** Check out, reset, delete and similar actions from the strip and right-click menus, with a confirmation before anything is lost.

## Knowledge base

Owned by The workbench (KB mode and its settings).

- **Several sources.** Gather documents from a folder, a git repository or an internal wiki Ubiq keeps for the project.
- **Explorer and tabs.** Browse sources and open their documents as tabs beside the code.
- **Write access.** Create, rename and edit documents in a source that allows writing; read-only sources stay read-only.
- **Agent access.** Agents read and write the knowledge base through a tool server.
- **Protected wikis.** Lock an internal wiki with a password.

## Database

Owned by [DB mode](../features/workbench-db.md).

- **Connections.** Save PostgreSQL, MySQL/MariaDB, SQLite and SQL Server connections per project; passwords are stored sealed, never shown again.
- **Explorer.** Walk connection, database, schema and objects lazily.
- **Table tabs.** Page, filter, sort and edit rows.
- **SQL tabs.** Run the statement under the cursor or all statements, with a timer, Stop and Explain.
- **Read-only guard.** A read-only connection cannot write, enforced by the host.
- **DBML export.** Export a schema as DBML.
- **Agent SQL.** Agents query through a SQL tool server and the result opens in a tab.

## Tasks and planning

Owned by [Tasks mode](../features/workbench-tasks.md).

- **Task board.** A column per status, a card per task; drag to change stage or place, archive finished cards, and keep several named boards.
- **Find and label filters.** Narrow the board by text, labels and mission.
- **Task panel.** Read a task whole and edit it field by field, with attachments.
- **Prerequisites.** Make one task wait on another and see which cards are ready.
- **Missions.** A task with children and a plan; an assistant agent is launched with it and spawns work.
- **Plan surface.** Read and edit a mission's plan as Markdown, with annotation threads between you and the agent.
- **Phases.** Move a mission through its phases and see what needs you.
- **Mission settings and views.** Tune a mission and view its work breakdown.
- **Annotate any Markdown file.** Collaborate with an agent on any document in the project.
- **Task sync.** Bind a project's tasks to an outside board.

## Teams

Owned by [The Teams graph modes](../features/workbench-teams.md).

- **Team canvas.** See a project's, or every project's, running agents as cards with delegates and a status mark.
- **Arrangements.** Choose among twelve automatic layouts, fit the view, and pan and zoom.
- **Task fences.** Drop a card into a task's outline to assign it; a mission draws as one fence.
- **Filters.** Narrow by session, mission and finished delegates.
- **Tasks drawer.** List tasks grouped by mission beside the canvas.
- **Live conversation.** Open the selected agent's real conversation in the dock.

## Remote hosts and drones

Owned by [Drones](../features/drone.md) and The workbench (hosts and SSH settings).

- **Attach a remote host.** Connect to another Ubiq host or to a machine over SSH; its projects appear in the same list.
- **Drones.** Ubiq places a small program on the remote machine and serves its terminals, files and search; it uploads the program when the machine has none.
- **Three lifetimes.** Choose whether a remote session ends with the window, survives a dropped link, or keeps running until stopped.
- **SSH profiles.** Save named targets, authenticating by agent, key, password or SSH config alias, with secrets in the OS credential store.
- **Drone manager.** See what a drone runs, check reachability, refresh and stop it.
- **Remote projects.** Run a project's panes, files and search on the remote machine.

## Connectors

Owned by [Connectors](../features/connectors.md).

- **Named identities.** Connect GitHub, GitLab, Gitea, Azure DevOps, Atlassian and Google Workspace, several per provider, cloud or self-hosted.
- **Sign-in flows.** Complete a browser flow or paste a token; tokens are kept in the OS keychain.
- **Application registrations.** Register an OAuth application for a self-hosted install.
- **Certificate pinning.** Accept an untrusted certificate by pinning one confirmed fingerprint.
- **Clone through a connector.** List an identity's repositories and clone one.

## Notifications, logs and stats

Owned by [Notifications](../features/notifications.md), [Logs](../features/logs.md) and [Stats](../features/stats.md).

- **Notification bell.** A history of notices with a level, an origin and a link, an unread badge, and a click that goes to where it points.
- **Mute rules.** Mute by scope, level and duration, which also decides what the desktop shows.
- **Log console.** Read every subsystem's diagnostics in one panel, filtered by subsystem and level; set a log file to keep them.
- **Control screen.** See the host's memory, uptime, open projects, live agents and agents run since launch.

## Help

Owned by The workbench.

- **In-app help.** Open help from the titlebar and point at a control to ask about it.
- **Kitchen sink.** Opt into a test bench of fixtures for viewers, pickers and layouts (see [Sink mode](../features/workbench-sink.md)).

## Settings and appearance

Owned by [The workbench](../features/workbench.md).

- **Themes.** Light and dark palettes, accent colours, text brightness, and your own themes.
- **Size.** Scale the interface and text, with saved presets and separate trims for chrome, conversation and content.
- **Editor and explorer options.** Preview-on-click, Markdown default view and clone folders.
- **Search and indexing.** Choose what search skips and what a project is indexed to.
- **Assistance.** Choose the backend that writes short names and summaries, and configure API providers with a test.
- **Tools.** Define runnable commands every project inherits.
- **Command line.** Install the `ubiq` command on the shell's path.
- **Updates.** Pick a channel, check, download and install updates.
- **Shell integration.** Add Open in Ubiq to the Windows right-click menu.

## Related docs

- [`overview.md`](./overview.md) — why Ubiq exists and what it refuses to be
- [`glossary.md`](./glossary.md) — the terms used above
- [`../features/workbench.md`](../features/workbench.md) — the window shell, settings and projects
- [`../backlog.md`](../backlog.md) — what is unresolved
