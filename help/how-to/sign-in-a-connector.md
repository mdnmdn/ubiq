---
id: how-to-sign-in-a-connector
title: Sign in a connector
summary: Connect a GitHub or GitLab identity so Ubiq can reach your repositories.
keywords: [github, gitlab, sign in, connect, oauth, token, clone, connector]
order: 70
related: [settings-connectors, agents-accounts]
---

Use this when you want Ubiq to act on a code host as you, for example to list and clone your repositories. This is not the login a harness runs under; that is an [account](agents-accounts).

1. Open **Settings** from the title bar, then **Connectors**, and press **Connect…**.
2. Pick the **Provider**. For a company's own install, type its address under **Instance**; empty means the provider's own cloud.
3. Pick **How**: **Paste a token**, **Enter a code** or **Open a browser**. Only the ways that work for that provider are offered.
4. If **Application** is shown, keep **Default** or choose an application you registered.
5. Type a **Name** for the identity and press **Connect**.
6. Finish the way you chose. For a token, paste it and press **Continue**. For a code, type the code shown at the address shown. For a browser, finish in the browser while Ubiq waits.

The connection appears in the list with a `valid` chip.

If the server's certificate is not recognised, Ubiq shows its details; **Trust this certificate** or **Cancel**. To sign in again after a chip reads `expired`, disconnect the connection and connect once more. [Connectors](settings-connectors) covers registering your own application.
