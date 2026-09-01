//! What a service wants said, so the caller can address it.
//!
//! A service does not hold the bus: it answers in terms of who should hear a thing, and the
//! coordinator turns that into a `To`. That is what keeps a service testable without a bus. It
//! lives here rather than under any one of them because more than one answers in these terms.

/// What the service wants said, so the caller can address it.
#[derive(Debug)]
pub enum Reply {
    /// Only the window that asked. A listing, or its own preferences.
    Asker(ubiq_proto::messages::Message),
    /// Every window, because the catalogue is one thing they all show.
    Everyone(ubiq_proto::messages::Message),
}

impl Reply {
    /// The message itself, whoever it is for.
    pub fn message(&self) -> &ubiq_proto::messages::Message {
        match self {
            Reply::Asker(message) | Reply::Everyone(message) => message,
        }
    }

    /// Whether every window should hear this.
    pub fn is_broadcast(&self) -> bool {
        matches!(self, Reply::Everyone(_))
    }

    pub fn into_message(self) -> ubiq_proto::messages::Message {
        match self {
            Reply::Asker(message) | Reply::Everyone(message) => message,
        }
    }
}
