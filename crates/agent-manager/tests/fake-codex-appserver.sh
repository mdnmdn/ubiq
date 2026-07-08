#!/bin/sh
# Fake `codex app-server --listen stdio://` for testing
# `agent_manager::io::CodexBridge` without the real `codex` binary or
# network access. See `tests/codex_bridge.rs`.
#
# Protocol handled (mirrors `_docs/harness/codex.md` §"Orchestration /
# headless invocation"):
#   1. `initialize` request  -> response echoing the same `id`.
#   2. `initialized` notification -> ignored (no response expected; it
#      carries no `id`).
#   3. `thread/start` request -> response with `result.thread.id = "t-1"`.
#   4. `turn/start` request -> response with `result.turn.id`, then TWO
#      notifications exercising the v2/raw dialect mapping:
#        - `item/completed` (itemType: agentMessage) -> assistant text
#        - `turn/completed` (with a usage block)     -> terminal success
#      then the script exits. This is the key behavior under test: the
#      script terminates on its own once the turn is "done", so the
#      integration test's event-drain loop returns rather than hanging.
#
# `CodexBridge` always serializes outbound JSON-RPC objects via
# `serde_json`'s default (non-`preserve_order`) map, which sorts object keys
# alphabetically. For every REQUEST we send (`id` + `jsonrpc` + `method` +
# `params`), that means `id` always sorts first ("i" < "j" < "m" < "p"), so
# every request line looks like `{"id":<N>,"jsonrpc":"2.0","method":"...",...`.
# This script relies on that fixed ordering to pull `id` out with `sed`
# rather than a JSON parser (kept POSIX `sh` + `sed`/`grep`, no `jq`
# dependency), and matches on `method` with a `case` glob against the raw
# line rather than parsing.

extract_id() {
    # $1: a JSON-RPC request line shaped like {"id":<N>,"jsonrpc":...
    echo "$1" | sed -n 's/^{"id":\([0-9][0-9]*\).*/\1/p'
}

while IFS= read -r line; do
    case "$line" in
        *'"method":"initialize"'*)
            id=$(extract_id "$line")
            echo "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"serverInfo\":{\"name\":\"fake-codex\"}}}"
            ;;
        *'"method":"initialized"'*)
            # Notification (no `id`) — nothing to answer.
            ;;
        *'"method":"thread/start"'*)
            id=$(extract_id "$line")
            echo "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"thread\":{\"id\":\"t-1\"}}}"
            ;;
        *'"method":"turn/start"'*)
            id=$(extract_id "$line")
            echo "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
            echo '{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"item-1","itemType":"agentMessage","text":"hello from fake codex"}}}'
            echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"turn":{"usage":{"input_tokens":3,"output_tokens":4}}}}'
            # The turn is done — exit so the script (and the pipe) closes
            # rather than blocking on another `read`.
            exit 0
            ;;
    esac
done

exit 0
