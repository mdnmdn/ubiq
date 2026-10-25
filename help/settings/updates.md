---
id: settings-updates
title: Updates
summary: The Updates section — the version running, the release channel, and checking, downloading and installing a newer build.
keywords: [update, upgrade, version, release, channel, stable, beta, nightly, download, restart, release notes]
order: 100
status: current
related: [settings, how-to-find-the-logs]
---

**Updates** shows which build of Ubiq is running and looks for a newer one. Nothing is installed
without your say: a downloaded update waits until you restart or quit.

## The parts

| Part | What it changes for you |
|---|---|
| **Version** | The build running now; nothing to set |
| **Channel** | **Stable**, **Beta** or **Nightly**; beta includes stable, and nightly includes both |
| **Check for updates automatically** | Looks shortly after start, then every few hours |
| **Download updates automatically** | Fetches an available update in the background; installing it is still your call |
| A status chip | What the last check found: up to date, an update available, downloading, ready, or why it failed |
| **Check now** | Looks straight away |
| **Download** | Fetches the available update |
| **Open download page** | Shown instead of **Download** when this build cannot update itself; opens the page in your browser |
| **Restart and install** | Once downloaded: closes Ubiq and installs |
| **Install on quit** | Once downloaded: installs the next time you close Ubiq |
| **Release notes** | Opens what changed in the update |

## What you can do here

- Follow a different release line: pick another **Channel**, then **Check now**.
- Stop all automatic activity: turn off both checkboxes; **Check now** still works.

## When something goes wrong

- **The controls are dimmed.** This build cannot update itself; the status chip says why. Get a newer
  one from the download page.
- **The chip says it failed.** Press **Check now** again; if it keeps failing, look in
  [the logs](how-to-find-the-logs).
- **I changed the channel and nothing happened.** The channel only decides what the next check looks
  for; press **Check now**.
