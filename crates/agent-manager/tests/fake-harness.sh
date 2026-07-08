#!/bin/sh
# Fake harness for testing the PTY passthrough runner without a real agent
# binary or network access. See `tests/passthrough.rs`.
#
# Behavior:
#   - prints argv and a couple of selected env vars to stdout;
#   - if $FAKE_OUT is set, writes $PWD and the full environment to that file;
#   - exits with $FAKE_EXIT (default 0).
#
# NOTE: it deliberately does NOT read stdin. Under `spawn_in_pty` the PTY
# master's writer is never closed by the test, so a blocking `cat` on stdin
# would wait for an EOF that never comes and hang `child.wait()`. A real
# harness reads stdin interactively; this stand-in just fires and exits.

echo "argv: $*"
if [ -n "${CLAUDE_CONFIG_DIR+x}" ]; then
    echo "CLAUDE_CONFIG_DIR=$CLAUDE_CONFIG_DIR"
else
    echo "CLAUDE_CONFIG_DIR unset"
fi
if [ -n "${CLAUDECODE+x}" ]; then
    echo "CLAUDECODE set"
else
    echo "CLAUDECODE unset"
fi

if [ -n "${FAKE_OUT:-}" ]; then
    {
        echo "PWD=$PWD"
        env
    } >"$FAKE_OUT"
fi

exit "${FAKE_EXIT:-0}"
