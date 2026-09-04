#!/usr/bin/env bash
# The bundle version shown in the app's footer and in web-export output. Printed on stdout with
# no trailing newline, so a caller can drop it straight into UBIQ_VERSION.
#
# Precedence:
#   1. UBIQ_VERSION already in the environment — used verbatim.
#   2. A clean working tree whose HEAD carries a tag starting with "v" — that tag.
#   3. No git available (or not inside a repo) — dev-<ubiq-app version>-<UTC timestamp>.
#   4. Anything else (dirty tree, or clean but untagged) — dev-<UTC timestamp>-<8-char hash>.
set -euo pipefail

if [ -n "${UBIQ_VERSION:-}" ]; then
    printf '%s' "$UBIQ_VERSION"
    exit 0
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
timestamp="$(date -u +%Y%m%d%H%M)"

if ! command -v git >/dev/null 2>&1 || ! git -C "$repo_root" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    cargo_version="$(awk -F'"' '/^version *=/ { print $2; exit }' "$repo_root/crates/ubiq-app/Cargo.toml")"
    printf 'dev-%s-%s' "$cargo_version" "$timestamp"
    exit 0
fi

if [ -z "$(git -C "$repo_root" status --porcelain)" ]; then
    tag="$(git -C "$repo_root" tag --points-at HEAD | grep '^v' | head -n1 || true)"
    if [ -n "$tag" ]; then
        printf '%s' "$tag"
        exit 0
    fi
fi

hash="$(git -C "$repo_root" rev-parse --short=8 HEAD)"
printf 'dev-%s-%s' "$timestamp" "$hash"
