/// Orchestrator module
///
/// Manages:
/// - Process spawning and lifecycle
/// - PTY allocation and configuration
/// - Signal handling (SIGWINCH for resize)
/// - Process exit handling and cleanup
///
/// TODO: Implement using `portable-pty` for cross-platform PTY management
