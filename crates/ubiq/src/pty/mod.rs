//! PTY I/O handling and stream management
//!
//! Provides:
//! - PTY spawning and configuration
//! - Output stream reading
//! - Input stream writing with backpressure handling
//! - Terminal emulation support (delegated to xterm.js in UI)
//!
//! TODO: Implement PTY stream management using portable-pty
