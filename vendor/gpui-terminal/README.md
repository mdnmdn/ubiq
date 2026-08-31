# gpui-terminal (vendored)

A vendored copy of [zortax/gpui-terminal](https://github.com/zortax/gpui-terminal), the terminal
emulator component Ubiq's panes are drawn with. It parses VT with
[`alacritty_terminal`](https://docs.rs/alacritty_terminal) and takes any `Read`/`Write` pair, which
is what lets Ubiq hand it a bus endpoint instead of a pseudo-terminal.

Upstream revision: `51f0292938876c8da3de03f0139088591e3be518`.

It is vendored rather than depended on because it is written against the `gpui` published on
crates.io and Ubiq builds against Zed's `main`, where two calls have since changed shape. The
divergence from upstream is:

- `Cargo.toml` — `gpui` comes from Zed's git, the example binary and its `portable-pty` dependency
  are dropped.
- `src/lib.rs` — one crate-level `allow` for the lint upstream trips, so `just clippy` stays clean
  without editing upstream code.
- `src/render.rs` — `ShapedLine::paint` takes a `TextAlign` and a wrap width.
- `src/view.rs` — `Window::focus` takes the app context, and the view's background comes from the
  configured palette rather than a hard-coded grey, so Ubiq's theme reaches it.

Keep that list accurate: it is what a future rebase onto upstream has to reapply.
