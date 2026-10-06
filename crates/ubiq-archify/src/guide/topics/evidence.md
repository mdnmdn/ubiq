<!-- Adapted from Archify 3.0.1 references/repository-authoring.md (MIT, (c) tt-a1i). -->
# Repository evidence

For a diagram that must reflect real code. This server checks the shape of the evidence fields and
reads git: against the project's checkout (`repo_root` to name another) it verifies that the cited
files and lines exist in the committed blobs at the pinned SHA, never in the working tree, and that
the checkout's `origin` is the declared `url` (the project root must be the repository's top-level
directory). It cannot say whether a citation means what you claim: that truth is your responsibility.
A pinned SHA that is not committed yet (`revision-unavailable`) or a path that is only in the
working tree (`file-missing`) means commit first, then pin.

1. **Freeze identity.** `meta.repository {url, revision, link_mode?}`: `url` is the credential-free
   origin (HTTP(S), `git@host:path` or `ssh://git@host/path`), `revision` a full 40-hex commit,
   `link_mode` `web` (GitHub/Gitee HTTPS) or `local-only` (default for internal forges).
2. **Map the slice**: entrypoints, runtime boundaries, storage, transports, deployment config.
3. **Trace ownership.** Source evidence comes from call sites (actor, operation, target). Controller,
   executor and store are different nodes. A configured provider is not an injected adapter, not a
   local stub, not a durable service. An exported but uncalled function is an optional capability,
   not a runtime edge. Never infer causality from file proximity or naming, and never cite a startup
   file as protocol or persistence evidence.
4. **Record while reading.** Each key node gets 1-3 `sources`: `{path, line?, end_line?, label?}`
   (`path` repo-relative POSIX, no `..`; `end_line` needs `line`; `label` <= 48 chars). Architecture
   `components[]`, workflow/dataflow `nodes[]`, sequence `participants[]`, lifecycle `states[]`.
5. **Name uncertainty** in a card or label instead of guessing.
