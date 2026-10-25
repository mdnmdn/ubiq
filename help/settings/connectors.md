---
id: settings-connectors
title: Connectors
summary: The Connectors section — your GitHub and GitLab identities, and registering an application to sign in with.
keywords: [connector, github, gitlab, gitea, azure devops, atlassian, google, oauth, device code, certificate, identity, token]
order: 60
status: current
related: [settings, how-to-sign-in-a-connector, agents-accounts]
---

A connector is you, signed in at a code host or workspace provider: GitHub, GitLab, Gitea, Azure
DevOps, Atlassian or Google Workspace, in the cloud or on your own install. Ubiq uses it to list the
repositories you clone and to reach issues and pull requests. Several connectors for one provider are
ordinary, such as a work identity and a personal one. This is not the account a harness signs in as;
see [Accounts](agents-accounts).

## The parts

| Part | What it does |
|---|---|
| **Connect…** | Opens the connect dialog; its steps are in [Sign in a connector](how-to-sign-in-a-connector) |
| **Provider**, **Instance** | In that dialog: which service, and the base URL of a self-managed install; empty means the provider's own cloud |
| **How** | **Paste a token** (needs no registered application), **Enter a code** (you type a code Ubiq shows) or **Open a browser**; only the ways that work for that provider are offered |
| **Application** | Which registered application Ubiq signs in through: **Default** where this build ships one, one of your registrations, or **Another instance…** |
| **Name** | What to call the identity; you can rename it later |
| A connection row | Its name, the account it is, where it lives (**cloud** or the install's URL), a chip reading `valid`, `expired`, `unknown` or `missing`, and `pinned` when you trusted that server's certificate |
| **Check** | Re-reads the stored token's expiry; a connection says `valid` when it has not expired, not that it will work |
| Rename and disconnect buttons | Rename the connection, or disconnect it after a confirm, which removes its stored token |
| **Trusted certificates** | Servers whose certificate you vouched for, each with **Forget** |
| **App registration…** | Registers an application you made at a provider: **Name**, **Provider**, **URL**, **Client id**, an optional **Client secret**, and the **Callback URL** to copy to the provider; its rows have **Edit** and **Delete** |

## What you can do here

- Add an identity step by step: [Sign in a connector](how-to-sign-in-a-connector).
- Use one to fetch a repository: pick **Clone a project…** from the **Open a project** chevron in
  the [title bar](workbench-titlebar), then pick the connector in the clone dialog. [Clone a project](how-to-clone-a-project) has the steps.
- Sign in to a company's own GitLab or Azure DevOps Server: choose **Paste a token**, or register
  an application there first and use **Open a browser**.

## When something goes wrong

- **The dialog says This server's certificate did not validate.** Compare the details shown with
  what you expect, then press **Trust this certificate** or **Cancel**. The choice applies to that
  server for every connection on it; remove it later with **Forget**.
- **Open a browser needs an application.** On a self-managed install, register one with **App
  registration…** and pick it under **Application**, or paste a token instead.
- **The browser does not open.** The dialog shows the link to open yourself and keeps waiting.
- **Disconnecting kept my certificate.** Trust belongs to the server, so **Forget** is separate.
- **Deleting a registration did not break my connections.** Each keeps the application it was made
  with; only new sign-ins under it are gone.
- **A connection says expired.** Disconnect it and connect again.
