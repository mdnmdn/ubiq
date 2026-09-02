Observed running inside Ubiq's own terminal pane (2026-09-02): `.zshrc` sourcing throws
`command not found: pyenv` / `jump` / `starship`, and `uv` (present at `/opt/homebrew/bin/uv`) is
not on `PATH`, even though the same shell works fine in Terminal.app/iTerm. Shift+Enter also behaves
differently than in a normal terminal.

Likely cause: however Ubiq spawns the PTY shell, it's not matching a real terminal emulator's
invocation — non-login and/or non-interactive, or with a `PATH`/env that hasn't gone through
Homebrew's `shellenv`/`path_helper` the way login shells do. `.zshrc` still runs (hence the visible
errors) but the tools those hooks depend on were never put on `PATH` first. Shift+Enter differing
suggests the key event may be getting mapped/intercepted by Ubiq's input handling rather than passed
through as the raw byte sequence a real terminal would send.

Not investigated or fixed. Relevant to whichever code spawns the shell process and to the keyboard
pass-through work already tracked in
[`terminal-interaction-proposal.md`](./terminal-interaction-proposal.md) (section 1, keyboard
pass-through audit) — worth checking as part of that audit whether Shift+Enter is one of the gaps.
