# Ubiq — every command anyone runs. Reference: _docs/tech/operations.md

# The bundle version baked into the footer — env override, else derived from git.
# See _devops/scripts/bundle-version.sh.
export UBIQ_VERSION := `_devops/scripts/bundle-version.sh`

# List the recipes
default:
    @just --list

# ── the application ────────────────────────────────────────────────

# Run Ubiq
dev: help-bundle
    cargo run -p ubiq-app --features assist-apple

# Run Ubiq with debug logging
verbose: help-bundle
    RUST_LOG=debug cargo run -p ubiq-app --features assist-apple

# Build the whole workspace for release
build: help-bundle
    cargo build --workspace --release --features ubiq-app/assist-apple

# Build the macOS application icon from the logo in assets/
icns:
    uv run _tools/icns.py

# Assemble Ubiq.app in target/ — icon, binary, Info.plist
bundle: help-bundle
    uv run _tools/icns.py
    cargo build -p ubiq-app --release --features assist-apple
    rm -rf target/Ubiq.app
    mkdir -p target/Ubiq.app/Contents/MacOS target/Ubiq.app/Contents/Resources
    cp target/release/ubiq target/Ubiq.app/Contents/MacOS/ubiq
    cp target/AppIcon.icns target/Ubiq.app/Contents/Resources/AppIcon.icns
    cp _tools/Info.plist target/Ubiq.app/Contents/Info.plist
    # The help bundle ships inside the .app, where `help::resolve` looks first after the override
    cp target/help/help.bundle target/Ubiq.app/Contents/Resources/help.bundle
    @echo "Done at $(date)"

# Assemble the Windows release in target/ubiq-windows-x86_64/ — the .exe
bundle-win: help-bundle
    cargo build -p ubiq-app --release --features assist-apple
    rm -rf target/ubiq-windows-x86_64
    mkdir -p target/ubiq-windows-x86_64
    cp target/release/ubiq.exe target/ubiq-windows-x86_64/ubiq.exe
    cp target/help/help.bundle target/ubiq-windows-x86_64/help.bundle

# ── the harness library ────────────────────────────────────────────

# Run the `am` CLI: `just am claude --print-config`
am *ARGS:
    cargo run -p agent-manager -- {{ARGS}}

# Build agent-manager's core the way an embedder consumes it
core:
    cargo build -p agent-manager --no-default-features

# ── the boundary ───────────────────────────────────────────────────

# The host draws nothing: no GPUI crate may reach its dependency tree
host:
    cargo build -p ubiq-host --all-targets
    @! cargo tree -p ubiq-host -e no-dev --prefix none | grep -q '^gpui' \
        || { echo "the host draws: a gpui crate is in its tree"; exit 1; }

# The lean host a drone links: no version control, no index, no harness library, no listener and
# nothing that talks to a desktop. What is left is a machine's terminal, its files and its facts.
relay:
    cargo build -p ubiq-host --no-default-features --all-targets
    @! cargo tree -p ubiq-host --no-default-features -e normal,build --prefix none \
        | awk '{print $1}' | sort -u \
        | grep -qxE 'git2|tantivy|rusqlite|agent-manager|isol8|notify-rust|trash|ureq|rustls|tiny_http|gpui' \
        || { echo "the lean host is not lean: a gated crate reached its tree"; exit 1; }

# The interface names the protocol and never the host, and never a type size of its own
ui:
    cargo build -p ubiq --all-targets
    @! cargo tree -p ubiq -e no-dev --prefix none | grep -q '^ubiq-host' \
        || { echo "the interface names the host"; exit 1; }
    # `theme.rs` is the one file allowed to name a size. `text_size(px(font))`, where the size is
    # computed from the project's zoom, is legitimate — hence the digit.
    @! grep -rqE 'text_size\(px\([0-9]' crates/ubiq/src \
        || { echo "a literal type size outside theme.rs — use theme::font(Family, Role)"; exit 1; }
    # The same rule for icons. The component library's `Size` enum is discrete and does not follow
    # the UI scale, so an icon drawn at a size of its own is `Size::Size(theme::icon_*())`.
    @! grep -rqE '(with_size\(px\(|Size::Size\(px\()[0-9]' crates/ubiq/src \
        || { echo "a literal icon size outside theme.rs — use theme::icon_sm/icon_md/icon_lg"; exit 1; }

# ── checks ─────────────────────────────────────────────────────────

# The app compiles with no `ui` feature — no `use` escapes `#[cfg(feature = "ui")]` into a headless
# build. Studio's root Justfile has its own `headless`, routed through `--manifest-path
# ubiq/Cargo.toml` because `ubiq-app` is a patched non-member there; here it is a plain workspace
# member, so no manifest path is needed.
headless:
    cargo check -p ubiq-app --no-default-features --all-targets
    @! cargo tree -p ubiq-app -e no-dev --prefix none --no-default-features | grep -q '^gpui' \
        || { echo "the headless base draws: a gpui crate is in its tree"; exit 1; }

# The on-device model is Apple's: `foundation-models` may reach a macOS tree and no other. It
# needs a Swift toolchain and the macOS 26 SDK to build, so a Linux or Windows build that pulled
# it in would fail at its build script — a broken port, reported as a compiler error in a crate
# nobody on that platform asked for. Resolution alone answers this, so neither toolchain has to
# be installed to run it.
apple:
    @cargo tree -p ubiq-app -e normal,build --features assist-apple \
        --target aarch64-apple-darwin --prefix none \
        | grep -q '^foundation-models' \
        || { echo "the on-device backend is gone: foundation-models is not in the macOS tree"; exit 1; }
    @for triple in x86_64-unknown-linux-gnu x86_64-pc-windows-msvc; do \
        ! cargo tree -p ubiq-app -e normal,build --features assist-apple \
            --target $triple --prefix none \
            | grep -q '^foundation-models' \
            || { echo "foundation-models reached the $triple tree: the backend is not macOS-only"; exit 1; }; \
    done

# Type-check everything, tests and examples included
check:
    cargo check --workspace --all-targets
    # And with nothing on. The developer loop already runs without `assist-apple`; this is what
    # covers `quickjs` off too, where the script facade reports itself unavailable.
    cargo check -p ubiq-app --no-default-features --all-targets

# Not in `verify`: `assist-apple` compiles a Swift bridge, so it wants a Swift toolchain and the
# macOS 26 SDK, and it costs a minute of `swiftc` for a backend no test exercises. The developer
# loop builds the host's stub in its place; `just dev` is what runs the real thing.
#
# Type-check the on-device model backend — needs a Swift toolchain and the macOS 26 SDK
assist:
    cargo check -p ubiq-app --features assist-apple --all-targets

# Lint, warnings are errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Format
fmt:
    cargo fmt --all

# Test. Stdin is closed: the PTY passthrough tests want a non-interactive one
test:
    cargo test --workspace < /dev/null

# check + clippy + test + the crate boundary + docs-lint + help-check
verify: check clippy test host relay ui apple docs-lint help-check

# Can a confined agent build? Run unconfined for a baseline, then under
# `am run <harness> --isolate -- bash _tools/toolchain-smoke.sh` and diff.
smoke:
    bash _tools/toolchain-smoke.sh

# ── documentation ──────────────────────────────────────────────────

# Lint _docs/ — L1, L2, L4, L5, L7, L9, L10
docs-lint *PATHS:
    uv run _tools/docs.py lint {{PATHS}}

# Regenerate the INDEX catalogue and the code map
docs-index:
    uv run _tools/docs.py index

# Fail if a generated block is out of date, without writing
docs-check:
    uv run _tools/docs.py index --check

# L3: documents whose anchored files moved after they were verified
docs-drift:
    uv run _tools/docs.py drift

# Which documents your change owes an update — no args reads the working diff
docs-touched *PATHS:
    uv run _tools/docs.py touched {{PATHS}}

# The depends_on graph: roots, isolated documents, over-connected hubs
docs-graph:
    uv run _tools/docs.py graph

# Render a diagram: `just diagram _docs/design/wireframe-opus/02-session.excalidraw.yaml`
diagram SOURCE:
    uv run _tools/excalidraw.py to-image -i {{SOURCE}} -o {{without_extension(SOURCE)}}.png --scale 2

# ── icons ──────────────────────────────────────────────────────────

# The icon set's mechanical rules — registry, spec, ink. I01-I11
icons-check:
    uv run _tools/icons.py check

# Write crates/ubiq/src/ui/kit/icons.rs from the registry — `icons-check` fails if it is stale
icons-gen:
    uv run _tools/icons.py codegen

# A review sheet: `just icons-sheet pane-thinking`, or `just icons-sheet --category pane`
icons-sheet *ARGS:
    uv run _tools/icons.py sheet {{ARGS}}

# Copy a shipped icon in-tree as one of ours — no args fills every unfilled `adopted:` row
icons-adopt *ARGS:
    uv run _tools/icons.py adopt {{ARGS}}

# Compare the competing takes in assets/icons/variants/: `just icons-variants --category tool`
icons-variants *ARGS:
    uv run _tools/icons.py variants {{ARGS}}

# The whole set, one contact sheet per category — the periodic coherence pass
icons-audit *ARGS:
    uv run _tools/icons.py audit {{ARGS}}

# Icons that look like each other
icons-dupes:
    uv run _tools/icons.py dupes

# ── web assets ─────────────────────────────────────────────────────

# Re-snapshot the Excalidraw mirror from jsDelivr and rewrite the host's hash manifest
web-assets *ARGS:
    uv run _tools/webassets.py snapshot {{ARGS}}

# Re-fetch every file in that manifest and report CDN drift
web-assets-verify:
    uv run _tools/webassets.py verify

# Re-snapshot the draw.io mirror from jsDelivr's GitHub CDN and rewrite its hash manifest
web-assets-drawio *ARGS:
    uv run _tools/webassets.py snapshot --tenant drawio {{ARGS}}

# Re-fetch every file in the drawio manifest and report CDN drift
web-assets-verify-drawio:
    uv run _tools/webassets.py verify --out crates/ubiq-host/src/web_assets/manifest_drawio.rs

# ── help ───────────────────────────────────────────────────────────

# Validate help/ — passes trivially when it does not exist
help-check:
    uv run _tools/helpbundle.py check

# Validate help/, then write target/help/help.bundle
help-bundle:
    uv run _tools/helpbundle.py bundle

# Remove target/help/
help-clean:
    uv run _tools/helpbundle.py clean

# ── the drone ──────────────────────────────────────────────────────

# Cross-build ubiq-drone for one triple, or every triple this toolchain has installed
drone-build TRIPLE="":
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -n "{{TRIPLE}}" ]; then
        cargo build -p ubiq-drone --release --target {{TRIPLE}}
        exit 0
    fi
    installed=$(rustup target list --installed)
    triples="x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin x86_64-pc-windows-msvc"
    for triple in $triples; do
        if echo "$installed" | grep -qx "$triple"; then
            cargo build -p ubiq-drone --release --target "$triple"
        else
            echo "skipping $triple — not installed (rustup target add $triple)"
        fi
    done

# Hash every locally built drone binary into the generated manifest
drone-manifest:
    uv run _tools/drone.py snapshot

# Re-hash each named binary on disk and report drift against the manifest
drone-manifest-verify:
    uv run _tools/drone.py verify

# ── the teams graph ────────────────────────────────────────────────

# Render the graph's arrangements as PNGs: `just teamsim --all-scenarios --algo all --sheet`
teamsim *ARGS:
    uv run _tools/teamsim/teamsim.py {{ARGS}}

# ── reading the tree ───────────────────────────────────────────────

# Print many files at once: paths, globs, directories, or `path:start-end`
dump *ARGS:
    @uv run _tools/dump.py {{ARGS}}

# What those targets hold, and what printing them would cost
dump-list *ARGS:
    @uv run _tools/dump.py --list {{ARGS}}

# Just the declaration lines and the module headers
dump-outline *ARGS:
    @uv run _tools/dump.py --outline {{ARGS}}

# ── housekeeping ───────────────────────────────────────────────────

# Remove build output
clean:
    cargo clean

# Update dependencies
update:
    cargo update

# Audit dependencies for advisories
audit:
    cargo audit
