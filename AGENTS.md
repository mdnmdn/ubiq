# AGENTS.md - Guidelines for AI Agents

## Project Overview
Ubiq is a Harness Multiplexer - a tmux-like application for hosting and orchestrating multiple interactive agent harnesses (Claude Code, Gemini CLI, Codex, opencode, etc.) side by side. Each harness runs in a real terminal pane using xterm.js and portable-pty.

## Architecture Principles

### 1. Transport Contract
All communication between UI and coordinator follows the defined contract:
- **Downstream (coordinator → UI):** `output{ pane_id, bytes }`, `exited{ pane_id, code }`
- **Upstream (UI → coordinator):** `input{ pane_id, bytes }`
- **Control (bidirectional):** `spawn{ pane_id, harness, args }`, `resize{ pane_id, cols, rows }`, `focus{ pane_id }`

### 2. Separation of Concerns
- **UI (Frontend):** Only handles rendering, user input, and stream visualization
- **Coordinator (Rust):** Manages processes, PTYs, and I/O routing
- **No bypasses:** All communication must go through the defined contract messages

### 3. Future Compatibility
- Design decisions must support later split into separate processes
- Must support distributed harnesses on remote hosts
- Keep the UI agnostic about PTY locality

## Development Workflow

### Getting Started
```bash
# Install dependencies
just install

# Start development
just dev
```

### Code Structure
```
ubiq/
├── src/                    # Frontend (vanilla JS + xterm.js)
│   └── main.js            # Main UI logic
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── lib.rs          # Tauri commands and app state
│   │   └── coordinator.rs  # PTY management (to be implemented)
│   └── Cargo.toml          # Rust dependencies
├── index.html              # Main HTML structure
└── package.json            # npm dependencies
```

### Key Files to Modify
1. **`src-tauri/src/coordinator.rs`** - Implement actual PTY spawning and I/O
2. **`src/main.js`** - Connect xterm.js to Tauri commands
3. **`src-tauri/src/lib.rs`** - Add new Tauri commands as needed

## Coding Conventions

### Rust (Backend)
- Use `portable-pty` for cross-platform PTY management
- Handle all errors with proper `Result` types
- Use `uuid::Uuid` for pane IDs
- Keep coordinator methods async-ready for future Tokio integration

### JavaScript (Frontend)
- Use ES modules with Vite
- Keep xterm.js instances isolated per pane
- Use Tauri's `invoke()` for backend communication
- Maintain responsive UI during terminal I/O

### General
- Never implement terminal emulation - use xterm.js
- Never bypass the transport contract
- Keep pane IDs consistent across all messages
- Test resize handling thoroughly (common bug source)

## Implementation Priorities

### Phase 1: Single Pane (Current)
1. Implement PTY spawning in `coordinator.rs`
2. Connect xterm.js input/output to PTY streams
3. Test full-screen redraw, colors, resize, keystroke round-tripping

### Phase 2: Multiple Panes
1. Implement pane-ID tagging on all messages
2. Add focus management (route keystrokes to focused pane only)
3. Implement pane splitting/layout

### Phase 3: Production Features
1. Handle harness exit/crash
2. Add restart behavior
3. Implement proper error handling and recovery

## Testing Approach

### Manual Testing
- Test with real harnesses: `claude`, `gemini`, `codex`, `opencode`
- Verify terminal colors and formatting
- Test resize behavior (SIGWINCH → TIOCSWINSZ)
- Test keystroke routing (arrows, Ctrl/Alt, bracketed paste)

### Automated Testing (Future)
- Unit tests for coordinator methods
- Integration tests for PTY I/O
- Frontend tests for pane management

## Common Pitfalls

1. **Resize Issues:** Always propagate resize events to PTY via TIOCSWINSZ
2. **Focus Management:** Ensure keystrokes only go to focused pane
3. **Memory Leaks:** Properly dispose xterm.js instances when panes close
4. **Cross-Platform:** Test on macOS, Linux, and Windows (different PTY implementations)

## Security Considerations

- Never expose PTY operations directly to frontend
- Validate all input data from frontend
- Use Tauri's capability system for permissions
- Keep harness execution sandboxed where possible

## Performance Notes

- PTY I/O should be non-blocking
- Use buffering for high-throughput terminal output
- Consider backpressure if UI can't keep up
- Monitor memory usage with multiple panes

## Next Steps for Implementation

1. **Implement `coordinator.rs`:**
   - Use `portable_pty::NativePtySystem` for PTY creation
   - Spawn harness processes with proper environment
   - Set up output reading threads/tasks
   - Handle PTY resize via `TIOCSWINSZ`

2. **Connect Frontend:**
   - Use `@tauri-apps/api/core` invoke for commands
   - Set up xterm.js data listeners to send input
   - Feed PTY output into xterm.js instances
   - Handle window resize events

3. **Test End-to-End:**
   - Spawn a simple shell (bash/zsh)
   - Verify full terminal functionality
   - Test with actual agent harnesses

## Architecture Validation Checklist

- [ ] All communication uses contract messages
- [ ] UI never touches PTY directly
- [ ] Coordinator never renders anything
- [ ] Pane IDs are consistent across all messages
- [ ] Resize events propagate correctly
- [ ] Focus management works properly
- [ ] Harness exit is handled gracefully

## Questions to Ask Before Changes

1. Does this change affect the transport contract?
2. Does this make assumptions about PTY locality?
3. Will this work when coordinator and UI are separate processes?
4. Does this maintain separation of concerns?
5. Is this compatible with distributed harnesses?

Remember: The goal is to build a foundation that can evolve from in-process to two processes to distributed without rewriting core logic.