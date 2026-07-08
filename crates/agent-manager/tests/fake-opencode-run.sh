#!/bin/sh
# Fake opencode run --format json (headless) harness for testing
# `agent_manager::io::OpencodeBridge` without the real `opencode` binary or
# network access. See `tests/opencode_bridge.rs`.
#
# Protocol (mirrors `_docs/harness/opencode.md` §"Orchestration / headless
# invocation" / §"Output stream protocol"):
#   1. Emit a `step_start` event with a session ID.
#   2. Emit a `text` part with assistant text.
#   3. Emit a `tool_use` part with a tool call and its completed result.
#   4. Emit a `step_finish` event with token usage.
#   5. Exit with status 0 (stream end = completion).

echo '{"type":"step_start","sessionID":"fake-sess-123"}'
echo '{"type":"text","part":{"text":"hello from fake opencode"},"sessionID":"fake-sess-123"}'
echo '{"type":"tool_use","part":{"tool":"bash","callID":"call-1","state":{"status":"complete","input":{"command":"echo test"},"output":"test"}},"sessionID":"fake-sess-123"}'
echo '{"type":"step_finish","part":{"tokens":{"input":42,"output":13,"cache":{"read":0,"write":0}}},"sessionID":"fake-sess-123"}'

exit 0
