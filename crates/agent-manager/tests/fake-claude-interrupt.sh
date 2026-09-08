#!/bin/sh
# Fake Claude Code stream-json harness for the *cancellation* half of
# `agent_manager::io::JsonlBridge` — see `tests/jsonl_bridge.rs`.
#
# The permission fixture next door (`fake-claude-streamjson.sh`) reads exactly
# two lines and exits, so it cannot show what a cancel does to a session that
# is still open. This one never stops reading: it echoes every stdin line it
# is given into `$AM_FAKE_STDIN` and answers the two it recognises, so a test
# can assert both *what* the bridge wrote on a cancel and that the session was
# still there for the turn after it.
#
# Protocol (mirrors `_docs/harness/claude-code.md` §"Output stream protocol" /
# §"Process lifecycle"):
#   1. Emit a `system`/`init` event carrying a session id.
#   2. Loop over stdin, appending each line to `$AM_FAKE_STDIN`:
#      - a `{"subtype":"interrupt"}` `control_request` → answer it with a
#        `control_response` and end the turn with a `result`, the way the real
#        harness aborts a turn without ending the session;
#      - a `{"type":"user"}` prompt line → one assistant text block naming the
#        turn, then a terminal `result`.
#   3. Exit on EOF — which is what closing stdin (`AgentInput::Shutdown`, or
#      the bridge's `Drop`) does.

: "${AM_FAKE_STDIN:=/dev/null}"

echo '{"type":"system","subtype":"init","session_id":"fake-session-1"}'

turn=0
while IFS= read -r line; do
	printf '%s\n' "$line" >>"$AM_FAKE_STDIN"
	case "$line" in
	*'"subtype":"interrupt"'*)
		echo '{"type":"control_response","response":{"subtype":"success","request_id":"am-interrupt","response":{}}}'
		echo '{"type":"result","subtype":"error_during_execution","result":"interrupted","is_error":false,"usage":{}}'
		;;
	*'"type":"user"'*)
		turn=$((turn + 1))
		echo "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"turn $turn\"}]}}"
		echo '{"type":"result","result":"success","is_error":false,"usage":{}}'
		;;
	esac
done

exit 0
