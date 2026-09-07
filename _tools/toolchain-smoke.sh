#!/usr/bin/env bash
# toolchain-smoke.sh — can a confined agent actually build?
#
# Run this INSIDE a sandbox to find out what the policy took away:
#
#   am run claude --isolate -- bash _tools/toolchain-smoke.sh   # the real path
#   isol8 --profile-path ... -- bash _tools/toolchain-smoke.sh  # a policy by hand
#   bash _tools/toolchain-smoke.sh                              # unconfined baseline
#
# Every check runs twice on purpose: once for `exec` (is the binary reachable?)
# and once for a real write into the tool's own home cache. That split is the
# whole point — isol8 forwards $PATH, so `cargo --version` passes today while
# `cargo fetch` still fails on `~/.cargo`. A script that only probed versions
# would report a working toolchain to a user who cannot build.
#
# Exit 0 when nothing FAILed. SKIP is not a failure: a machine without dotnet
# is not a broken policy, and treating it as one is how a smoke test starts
# getting ignored.
set -uo pipefail

pass=0 fail=0 skip=0
declare -a failures=()

say() { printf '%s\n' "$*"; }
hdr() { printf '\n\033[1m%s\033[0m\n' "$*"; }

# report <status> <name> <detail>
report() {
  case "$1" in
    PASS) pass=$((pass + 1)); printf '  \033[32mPASS\033[0m  %-22s %s\n' "$2" "${3:-}" ;;
    FAIL) fail=$((fail + 1)); failures+=("$2"); printf '  \033[31mFAIL\033[0m  %-22s %s\n' "$2" "${3:-}" ;;
    SKIP) skip=$((skip + 1)); printf '  \033[90mSKIP\033[0m  %-22s %s\n' "$2" "${3:-}" ;;
  esac
}

# check <name> <binary> <version-args...> -- <write-probe...>
#
# Missing binary is a SKIP. A binary that runs but whose write probe fails is a
# FAIL, and that is the case this script exists to catch.
check() {
  local name=$1 bin=$2
  shift 2
  local -a ver=() probe=()
  local seen=0
  for arg in "$@"; do
    if [ "$arg" = "--" ]; then seen=1; continue; fi
    if [ $seen -eq 0 ]; then ver+=("$arg"); else probe+=("$arg"); fi
  done

  if ! command -v "$bin" >/dev/null 2>&1; then
    report SKIP "$name" "no $bin on PATH"
    return
  fi

  local out
  if ! out=$("$bin" "${ver[@]}" 2>&1); then
    report FAIL "$name" "exec: $(head -1 <<<"$out")"
    return
  fi
  local version
  version=$(head -1 <<<"$out" | cut -c1-40)

  if [ ${#probe[@]} -eq 0 ]; then
    report PASS "$name" "$version"
    return
  fi

  if out=$("${probe[@]}" 2>&1); then
    report PASS "$name" "$version"
  else
    report FAIL "$name" "$version — ${probe[0]}: $(grep -iEm1 'denied|not permitted|permission|read-only|error' <<<"$out" | cut -c1-70 || head -1 <<<"$out" | cut -c1-70)"
  fi
}

hdr "Environment"
say "  HOME      ${HOME:-<unset>}"
say "  PWD       $PWD"
say "  TMPDIR    ${TMPDIR:-<unset>}"
say "  sandboxed ${ISOL8_SANDBOXED:-no}"
say "  LANG      ${LANG:-<unset>}  LC_ALL=${LC_ALL:-<unset>}"
# A replaced home is the single most common cause of everything below failing:
# a layer's `~/.cargo` grant expands against the EFFECTIVE home, so a scratch
# home aims every toolchain grant at an empty directory.
if [ -n "${ISOL8_SANDBOXED:-}" ] && [ ! -d "${HOME:-}/.cargo" ] && [ ! -d "${HOME:-}/.npm" ]; then
  say "  note      \$HOME holds no toolchain state — a replaced home?"
fi

work=$(mktemp -d 2>/dev/null) || { say "cannot even mktemp; TMPDIR is denied"; exit 1; }
trap 'rm -rf "$work"' EXIT

hdr "Writability"
for dir in "$PWD" "$work" "$HOME"; do
  name=$([ "$dir" = "$PWD" ] && echo cwd || { [ "$dir" = "$work" ] && echo tmpdir || echo home; })
  if touch "$dir/.smoke-$$" 2>/dev/null; then
    rm -f "$dir/.smoke-$$"
    report PASS "$name writable" "$dir"
  else
    # A read-only $HOME is correct under the default policy — only the named
    # toolchain paths inside it are granted. cwd and TMPDIR are not optional.
    [ "$name" = "home" ] && report SKIP "$name writable" "$dir (read-only: expected)" \
                         || report FAIL "$name writable" "$dir"
  fi
done

hdr "Version managers"
check mise    mise    --version
check asdf    asdf    --version
check nvm     bash    -c 'source "${NVM_DIR:-$HOME/.nvm}/nvm.sh" && nvm --version'
check fnm     fnm     --version
check volta   volta   --version
check pyenv   pyenv   --version

hdr "Toolchains"
# `cargo metadata` reads ~/.cargo/registry and the project's lockfile — the
# cheapest command that proves the registry cache is readable.
check rust    cargo   --version   -- cargo metadata --no-deps --format-version 1
check rustup  rustup  --version   -- rustup toolchain list
# `npm config get cache` then a real cache write: `npm ping` needs the network,
# `npm cache ls` needs ~/.npm.
check node    node    --version
check npm     npm     --version   -- npm config get cache
check pnpm    pnpm    --version   -- pnpm store path
check yarn    yarn    --version
check bun     bun     --version
check deno    deno    --version
# `dotnet --info` writes the first-run sentinel under ~/.dotnet; isol8 ships no
# layer for it, which is why it is the canary for DEV_RW_HOME_ROOTS.
check dotnet  dotnet  --version   -- dotnet --info
check go      go      version     -- go env GOMODCACHE
check python  python3 --version   -- python3 -c 'import sysconfig;sysconfig.get_paths()'
check uv      uv      --version   -- uv cache dir
check java    java    -version
check ruby    ruby    --version
check php     php     --version

hdr "Git"
check git     git     --version   -- git config --get-regexp '^user\.'
if command -v git >/dev/null 2>&1; then
  # A commit with no author is the failure a missing ~/.gitconfig actually
  # produces, and it looks nothing like a permission error.
  ( cd "$work" && git init -q . 2>/dev/null \
      && git commit -q --allow-empty -m smoke 2>/dev/null ) \
    && report PASS "git commit" "author resolved" \
    || report FAIL "git commit" "no author — ~/.gitconfig unreadable?"
fi

hdr "Compiler and linker"
printf 'int main(void){return 0;}\n' >"$work/t.c"
check cc      cc      --version   -- cc -o "$work/t" "$work/t.c"
if [ -x "$work/t" ]; then
  report PASS "link and run" "$("$work/t" && echo ok)"
else
  # Without /Library/Developer/CommandLineTools ro, `cargo build` dies here and
  # nowhere earlier. This is what toolchains/apple-toolchain-core buys.
  report FAIL "link and run" "no linked binary"
fi

hdr "Isolation (these SHOULD be denied when confined)"
if [ -n "${ISOL8_SANDBOXED:-}" ]; then
  for secret in "$HOME/.ssh/id_ed25519" "$HOME/.ssh/id_rsa" "$HOME/.aws/credentials"; do
    if [ -e "$secret" ] && head -c1 "$secret" >/dev/null 2>&1; then
      report FAIL "denied: $(basename "$secret")" "READABLE — the policy leaks"
    else
      report PASS "denied: $(basename "$secret")" "unreadable"
    fi
  done
else
  report SKIP "denial checks" "not confined; nothing to prove"
fi

hdr "Result"
say "  $pass passed, $fail failed, $skip skipped"
if [ $fail -gt 0 ]; then
  say "  failed: ${failures[*]}"
  say ""
  say "  A FAIL on a toolchain means the binary ran but its home cache is not"
  say "  granted. Check the resolved layer list and whether \$HOME was replaced."
  exit 1
fi
say "  a confined agent can build here"
