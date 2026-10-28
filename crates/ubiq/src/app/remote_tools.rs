//! The window's half of the remote-tool family (`D226`): the host's MCP listener cannot reach a
//! drone, so a `RemoteToolCall` comes here and is relayed, unread, to the drone link its `remote`
//! names; the drone's `RemoteToolResult` is relayed back to the host it came from. The routing
//! itself is [`Bus::relay_tool_call`] and [`Bus::relay_tool_result`].

use super::*;

impl AppState {
    /// The remote-tool family. `None` means the message was ours.
    pub(super) fn receive_remote_tools(&mut self, host: HostRef, message: Message) -> Option<Message> {
        match message {
            call @ Message::RemoteToolCall { .. } => self.bus.relay_tool_call(host, call),
            result @ Message::RemoteToolResult { .. } => self.bus.relay_tool_result(host, result),
            other => return Some(other),
        }
        None
    }
}
