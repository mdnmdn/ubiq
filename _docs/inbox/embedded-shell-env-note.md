Observed running inside Ubiq's own terminal pane (2026-09-02): `.zshrc` sourcing throws
`command not found: pyenv` / `jump` / `starship`, and `uv` (present at `/opt/homebrew/bin/uv`) is
not on `PATH`, even though the same shell works fine in Terminal.app/iTerm. Shift+Enter also behaves
differently than in a normal terminal. Root cause found for both (2026-09-02), not fixed.

**PATH / shell-init errors — confirmed root cause.**
Both `crates/ubiq-host/src/pty/mod.rs:49` (`spawn`) and `crates/ubiq-host/src/coordinator.rs:755`
(`default_agent_type`, which resolves to `$SHELL`, e.g. `/bin/zsh`) build the shell via
`CommandBuilder::new(program)`. In `portable-pty` 0.9.0
(`cmdbuilder.rs:497-524`, `as_command`), that path only runs the login-shell branch
(`arg0 = "-<basename>"`) when the builder was made with `new_default_prog()`; `new(program)` always
takes the plain branch (`cmd.arg0(&self.args[0])`, no leading `-`). So the spawned zsh is a
**non-login** shell: `.zshrc` runs (hence the visible errors), but `.zprofile`/`.zlogin` — where
Homebrew's `eval "$(brew shellenv)"` and most `pyenv`/`starship`/`jump` PATH setup normally lives —
never runs, so those tools aren't on `PATH` yet when `.zshrc` calls their init hooks.
Compounding factor: `CommandBuilder::new`'s base env (`get_base_env()`, `cmdbuilder.rs:75`) is
`std::env::vars_os()` captured from Ubiq's own process at spawn time — i.e. whatever `PATH` Ubiq
itself was launched with (thin, if launched via Finder/LaunchServices rather than a shell), not a
freshly computed one.

**Shift+Enter — confirmed root cause.**
`vendor/gpui-terminal/src/input.rs:126`, `keystroke_to_bytes`: the `"enter"` match arm is
`"enter" => return Some(b"\r".to_vec())`, unconditional — it never looks at
`keystroke.modifiers.shift` (unlike `"tab"`, a few lines below, which does branch on shift). So
Shift+Enter and plain Enter send the identical byte (`\r`) to the pane. A harness that expects
Shift+Enter to send a distinct sequence (to insert a literal newline instead of submitting) can't
tell the two apart here.

Both are in the pass-through/spawn code, not something a user setting can work around. Relevant to
the keyboard pass-through work already tracked in
[`terminal-interaction-proposal.md`](./terminal-interaction-proposal.md) (section 1, "every special
keystroke reaches the harness correctly" — Shift+Enter is a gap in that claim, worth folding in
there). The login-shell issue has no existing tracking document; it belongs wherever PTY spawn
behavior is documented next.

**Status (2026-09-02).** The login-shell half is fixed: `crates/ubiq-host/src/shells.rs` says what a
shell is, `pty::command_for` starts one the way a terminal does, and
`crates/ubiq-host/tests/coordinator.rs` asserts a shell pane's argv0. The behaviour is
`_docs/features/panes-and-terminals.md`'s and the trade is `D49`; the base-environment factor is
`G87`. The Shift+Enter half is untouched and stays with the keyboard pass-through work in
[`terminal-interaction-proposal.md`](./terminal-interaction-proposal.md).
