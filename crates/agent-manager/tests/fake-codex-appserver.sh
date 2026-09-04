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
# **Key order is not fixed, and nothing here may assume it is.** `serde_json`
# sorts object keys alphabetically only in its default configuration; with the
# `preserve_order` feature its map is an `IndexMap` and keys come out in
# insertion order instead. That feature is not agent-manager's choice — it
# arrives through Cargo's workspace-wide feature unification, because Zed's
# `gpui`/`http_client` crates (which `crates/ubiq` needs) turn it on. So the
# same request is `{"id":1,"jsonrpc":...}` under `cargo test -p agent-manager`
# and `{"jsonrpc":"2.0","id":1,...}` under `cargo test --workspace`.
#
# An earlier version of this script pulled `id` out with a `sed` anchored to
# `^{"id":`, which quietly produced a malformed response under the workspace
# build; the bridge then never matched a response to its `initialize` request
# and timed out after 10s. That looked for a long time like a load-dependent
# flake. It was not: it is deterministic per build configuration.
#
# `method` is matched with a `case` glob against the raw line, which is
# already order-independent. Kept to POSIX `sh` builtins — no `jq` dependency.

# Sets `$id` from a JSON-RPC request line, wherever `"id":` appears in it.
extract_id() {
    rest=${1#*\"id\":}  # everything after the first `"id":`
    rest=${rest%%,*}    # up to the next comma …
    id=${rest%%\}*}     # … or the closing brace, when `id` is the last key
}

while IFS= read -r line; do
    case "$line" in
        *'"method":"initialize"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"serverInfo\":{\"name\":\"fake-codex\"}}}"
            ;;
        *'"method":"initialized"'*)
            # Notification (no `id`) — nothing to answer.
            ;;
        *'"method":"thread/start"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"thread\":{\"id\":\"t-1\"}}}"
            ;;
        *'"method":"turn/start"'*)
            extract_id "$line"
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
