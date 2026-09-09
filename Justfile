# Ubiq — every command anyone runs. Reference: _docs/tech/operations.md

# The bundle version baked into the footer — env override, else derived from git.
# See _devops/scripts/bundle-version.sh.
export UBIQ_VERSION := `_devops/scripts/bundle-version.sh`

# List the recipes
default:
    @just --list

# ── the application ────────────────────────────────────────────────

# Run Ubiq
dev:
    cargo run -p ubiq-app

# Run Ubiq with debug logging
verbose:
    RUST_LOG=debug cargo run -p ubiq-app

# Build the whole workspace for release
build:
    cargo build --workspace --release

# Build the macOS application icon from the logo in assets/
icns:
    uv run _tools/icns.py

# Assemble Ubiq.app in target/ — icon, binary, Info.plist
bundle:
    uv run _tools/icns.py
    cargo build -p ubiq-app --release
    rm -rf target/Ubiq.app
    mkdir -p target/Ubiq.app/Contents/MacOS target/Ubiq.app/Contents/Resources
    cp target/release/ubiq target/Ubiq.app/Contents/MacOS/ubiq
    cp target/AppIcon.icns target/Ubiq.app/Contents/Resources/AppIcon.icns
    cp _tools/Info.plist target/Ubiq.app/Contents/Info.plist

# Assemble the Windows release in target/ubiq-windows-x86_64/ — the .exe
bundle-win:
    cargo build -p ubiq-app --release
    rm -rf target/ubiq-windows-x86_64
    mkdir -p target/ubiq-windows-x86_64
    cp target/release/ubiq.exe target/ubiq-windows-x86_64/ubiq.exe

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

# The interface names the protocol and never the host
ui:
    cargo build -p ubiq --all-targets
    @! cargo tree -p ubiq -e no-dev --prefix none | grep -q '^ubiq-host' \
        || { echo "the interface names the host"; exit 1; }

# ── checks ─────────────────────────────────────────────────────────

# Type-check everything, tests and examples included
check:
    cargo check --workspace --all-targets
    # And with no on-device model backend. `assist-apple` is on by default and needs a Swift
    # toolchain and the macOS 26 SDK; a build without it answers every suggestion from the stub
    # backend, and every call site has to read correctly against that.
    cargo check -p ubiq-app --no-default-features --all-targets

# Lint, warnings are errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Format
fmt:
    cargo fmt --all

# Test. Stdin is closed: the PTY passthrough tests want a non-interactive one
test:
    cargo test --workspace < /dev/null

# check + clippy + test + the crate boundary + docs-lint
verify: check clippy test host ui docs-lint

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
