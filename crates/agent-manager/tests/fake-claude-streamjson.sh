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
#   4. Emit a `can_use_tool` `control_request` — the real shape, captured
#      against claude 2.1.258: the call is described inline (`tool_name`,
#      `input`, `tool_use_id`), there is no `request.tool_use` object.
#      Then read ANOTHER line from stdin before continuing. This is the key
#      behavior under test: nothing answers this but the *caller*, so the
#      script blocks here until the test sends `AnswerPermission` (or a
#      `Cancel`, which denies it) — an unanswered ask stalls the turn, same
#      as the real harness.
#   5. Record the answer in `$AM_FAKE_ANSWER` when that env var is set, so a
#      test can assert both *that* it was answered and *how*.
#   6. Emit a `user` event with a `tool_result` content block.
#   7. Emit a terminal `result` event (`is_error:false`) with a
#      `modelUsage` map (camelCase field names, as Claude actually reports
#      them — see `_docs/harness/claude-code.md`), then exit.

IFS= read -r _prompt_line

echo '{"type":"system","subtype":"init","session_id":"fake-session-1"}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"hello from fake claude"}]}}'
echo '{"type":"control_request","request_id":"req-1","request":{"subtype":"can_use_tool","tool_name":"Bash","display_name":"Bash","input":{"command":"echo hi"},"description":"echo hi","permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}],"tool_use_id":"tool-1"}}'

# Blocks here until the caller answers the request above with a
# control_response line on our stdin.
IFS= read -r _control_response_line

if [ -n "$AM_FAKE_ANSWER" ]; then
	printf '%s' "$_control_response_line" >"$AM_FAKE_ANSWER"
fi

echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":[{"type":"text","text":"hi"}]}]}}'
echo '{"type":"result","result":"success","is_error":false,"usage":{},"modelUsage":{"fake-model":{"inputTokens":5,"outputTokens":7,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"contextWindow":200000}}}'

exit 0
