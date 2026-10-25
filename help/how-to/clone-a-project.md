---
id: how-to-clone-a-project
title: Clone a project
summary: Fetch a repository from your code host, or from a public URL, and open it as a project.
keywords: [clone, git, repository, github, gitlab, url, download, shallow, branch]
order: 35
related: [projects, settings-connectors, how-to-sign-in-a-connector]
---

Use this when the code is in a repository and not yet on this machine.

1. Open the chevron beside **Add a project** in the [title bar](workbench-titlebar), labelled **Open a project**, and pick **Clone a project…**.
2. Pick a repository from the list under **Repositories**, filtering it by typing. The **Identity** picker above chooses whose repositories are listed; it appears once you have a [connector](settings-connectors). Or paste a public repository's https address under **Or a URL**.
3. Under **Branch**, pick the branch to check out, or leave **The default branch**.
4. Under **Destination**, press **Choose…** for the folder the clone goes in, and check the name below it.
5. Tick **Throwaway** to discard the clone when you close it, or **Shallow** to fetch only the latest commit.
6. Press **Clone**. The progress reads at the foot, where **Cancel** stops it.

When the clone finishes the dialog closes and the project is in your list. A failure is written at the top of the dialog; correct the field and press **Clone** again.
