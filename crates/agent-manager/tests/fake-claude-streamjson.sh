#!/bin/sh
# Fake Claude Code stream-json (headless) harness for testing
# `agent_manager::io::JsonlBridge` without the real `claude` binary or
# network access. See `tests/jsonl_bridge.rs`.
#
# Protocol (mirrors `_docs/harness/claude-code.md` §"Output stream
# protocol" / §"Tool approval in headless mode"):
#   1. Read one NDJSON line from stdin — the prompt line the bridge sends
#      via `AgentInput::Prompt`. Drained, not inspected.
#   2. Emit a `system`/`init` event carrying a session id.
#   3. Emit an `assistant` event with a `text` content block.
#   4. Emit a `control_request` (a tool-approval ask), then read ANOTHER
#      line from stdin before continuing. This is the key behavior under
#      test: `JsonlBridge`'s reader thread must auto-allow it (write a
#      `control_response`) without any consumer answering, or this script
#      blocks forever on the second `read` and the test hangs.
#   5. Emit a `user` event with a `tool_result` content block.
#   6. Emit a terminal `result` event (`is_error:false`) with a
#      `modelUsage` map, then exit.

IFS= read -r _prompt_line

echo '{"type":"system","subtype":"init","session_id":"fake-session-1"}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"hello from fake claude"}]}}'
echo '{"type":"control_request","request_id":"req-1","request":{"type":"tool_use","tool_use":{"id":"tool-1","name":"Bash","input":{"command":"echo hi"}}}}'

# Blocks here until the bridge's reader thread auto-allows the request
# above by writing a control_response line to our stdin.
IFS= read -r _control_response_line

echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":[{"type":"text","text":"hi"}]}]}}'
echo '{"type":"result","result":"success","is_error":false,"usage":{},"modelUsage":{"fake-model":{"input_tokens":5,"output_tokens":7}}}'

exit 0
