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
#   3. `thread/start` request -> response with `result.thread.id = "t-1"`,
#      `model` and `sandbox`.
#   4. `turn/start` request -> response with `result.turn.id`, then the v2
#      notifications (shapes from `codex app-server generate-ts`, 0.161.0):
#        - `item/completed` (type: agentMessage)    -> assistant text
#        - `thread/tokenUsage/updated`               -> usage against a window
#        - `thread/status/changed` idle              -> nothing (not a 2nd end)
#        - `turn/completed` (status: completed)      -> terminal success
#      then the script exits. This is the key behavior under test: the
#      script terminates on its own once the turn is "done", so the
#      integration test's event-drain loop returns rather than hanging.
#   5. `turn/interrupt` request -> response with an empty `result`, then a
#      `turn/completed { status: interrupted }`: the turn ends, the thread
#      does not.
#
# Two env vars, both optional, both for the cancellation test:
#   - `AM_FAKE_CODEX_STAY` set  -> do NOT exit after a turn, so the same
#     process can be interrupted and then take another turn (which is what
#     `turn/interrupt` leaving the session alive means).
#   - `AM_FAKE_STDIN` set       -> append every stdin line read to that file,
#     so a test can assert what the bridge actually wrote.
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
    if [ -n "$AM_FAKE_STDIN" ]; then
        printf '%s\n' "$line" >>"$AM_FAKE_STDIN"
    fi
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
            echo "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"thread\":{\"id\":\"t-1\"},\"model\":\"gpt-fake\",\"sandbox\":{\"type\":\"workspaceWrite\"}}}"
            ;;
        *'"method":"thread/resume"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"result\":{\"thread\":{\"id\":\"t-resumed\"},\"model\":\"gpt-fake\",\"sandbox\":{\"type\":\"readOnly\"}}}"
            ;;
        *'"method":"model/list"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"result\":{\"data\":[{\"id\":\"gpt-fake\",\"model\":\"gpt-fake\",\"displayName\":\"Fake\",\"description\":\"\",\"hidden\":false,\"isDefault\":true,\"supportedReasoningEfforts\":[{\"reasoningEffort\":\"low\",\"description\":\"\"},{\"reasoningEffort\":\"high\",\"description\":\"\"}],\"defaultReasoningEffort\":\"low\"}],\"nextCursor\":null}}"
            ;;
        *'"method":"turn/steer"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"result\":{\"turnId\":\"turn-1\"}}"
            ;;
        *'"id":"srv-1"'*)
            # Our answer to the approval request below: the turn may now finish.
            echo '{"method":"serverRequest/resolved","params":{"threadId":"t-1","requestId":"srv-1"}}'
            echo '{"method":"turn/completed","params":{"threadId":"t-1","turn":{"id":"turn-1","status":"completed","error":null}}}'
            ;;
        *'"method":"turn/start"'*'please ask'* | *'please ask'*'"method":"turn/start"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
            echo '{"method":"item/started","params":{"threadId":"t-1","turnId":"turn-1","item":{"id":"c-1","type":"commandExecution","command":"rm -rf build","status":"inProgress"}}}'
            echo '{"id":"srv-1","method":"item/commandExecution/requestApproval","params":{"threadId":"t-1","turnId":"turn-1","itemId":"c-1","startedAtMs":0,"command":"rm -rf build"}}'
            ;;
        *'"method":"turn/start"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{\"turn\":{\"id\":\"turn-1\"}}}"
            # A held turn stays live until it is interrupted.
            if [ -n "$AM_FAKE_CODEX_HOLD" ]; then
                continue
            fi
            echo '{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"t-1","item":{"id":"item-1","type":"agentMessage","text":"hello from fake codex"}}}'
            echo '{"jsonrpc":"2.0","method":"thread/tokenUsage/updated","params":{"threadId":"t-1","turnId":"turn-1","tokenUsage":{"total":{"totalTokens":7,"inputTokens":3,"cachedInputTokens":0,"cacheWriteInputTokens":0,"outputTokens":4,"reasoningOutputTokens":0},"last":{"totalTokens":7,"inputTokens":3,"cachedInputTokens":0,"cacheWriteInputTokens":0,"outputTokens":4,"reasoningOutputTokens":0},"modelContextWindow":1000}}}'
            echo '{"jsonrpc":"2.0","method":"thread/status/changed","params":{"threadId":"t-1","status":{"type":"idle"}}}'
            echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"t-1","turn":{"id":"turn-1","status":"completed","error":null}}}'
            # The turn is done — exit so the script (and the pipe) closes
            # rather than blocking on another `read`, unless the test wants
            # the session to outlive the turn.
            if [ -z "$AM_FAKE_CODEX_STAY" ]; then
                exit 0
            fi
            ;;
        *'"method":"account/login/start"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"result\":{\"type\":\"chatgptDeviceCode\",\"loginId\":\"l-1\",\"verificationUrl\":\"https://auth.example/device\",\"userCode\":\"ABCD-1234\"}}"
            echo '{"method":"account/login/completed","params":{"loginId":"l-1","success":true,"error":null}}'
            ;;
        *'"method":"account/read"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"result\":{\"account\":{\"type\":\"chatgpt\",\"email\":\"dev@example.com\",\"planType\":\"pro\"},\"requiresOpenaiAuth\":true}}"
            ;;
        *'"method":"account/rateLimits/read"'*)
            extract_id "$line"
            if [ -n "$AM_FAKE_CODEX_SIGNED_OUT" ]; then
                echo "{\"error\":{\"code\":-32600,\"message\":\"codex account authentication required to read rate limits\"},\"id\":$id}"
            else
                echo "{\"id\":$id,\"result\":{\"rateLimits\":{\"limitId\":\"codex\",\"primary\":{\"usedPercent\":42.4,\"windowDurationMins\":300,\"resetsAt\":1791490000},\"secondary\":{\"usedPercent\":13,\"windowDurationMins\":10080,\"resetsAt\":1791900000},\"planType\":\"pro\"},\"rateLimitsByLimitId\":null}}"
            fi
            ;;
        *'"method":"turn/interrupt"'*)
            extract_id "$line"
            echo "{\"id\":$id,\"jsonrpc\":\"2.0\",\"result\":{}}"
            echo '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"t-1","turn":{"id":"turn-1","status":"interrupted","error":null}}}'
            ;;
    esac
done

exit 0
