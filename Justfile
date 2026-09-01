# Ubiq — every command anyone runs. Reference: _docs/tech/operations.md

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
