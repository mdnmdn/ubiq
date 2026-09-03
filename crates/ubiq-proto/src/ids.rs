//! The contract's identifiers: one newtype per kind, over a ULID.
//!
//! Every id in the message set is one of these. They are newtypes rather than a shared alias
//! because a pane ID and a session ID are both 128 bits and nothing but care would stop one being
//! passed where the other belongs — the compiler does it here instead.
//!
//! A ULID rather than a UUID because it sorts by creation time, prints as 26 case-insensitive
//! characters with no hyphens, and so gives a readable directory name and a stable ordering for
//! free. It carries its own timestamp, which is a feature in a config directory and a fact worth
//! knowing before ids travel to a host the user does not own.
//!
//! `gpui::WindowId` is not one of these. It is the framework's, and it stays as it is.

use std::fmt;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use ulid::{DecodeError, Ulid};

/// The process's one generator.
///
/// Monotonic within a millisecond, which is most of why a ULID is worth having: two ids minted in
/// the same millisecond by a bare `Ulid::new()` sort arbitrarily. Nothing here calls that.
///
/// Both constructors are `const`, so this needs no `OnceLock`. Contention is nil — every call site
/// is on a control path, never on the byte stream.
static IDS: Mutex<ulid::Generator> = Mutex::new(ulid::Generator::new());

/// The next ULID, monotonic against the last.
///
/// A generator overflows when it has minted 2^80 ids inside one millisecond. The recovery is to
/// step into the next millisecond, which is what the caller wants and cannot be an error here.
fn mint() -> Ulid {
    let mut generator = IDS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    generator
        .generate()
        .unwrap_or_else(|overflow| overflow.commit_overflow_random())
}

/// Declare one id kind. Every kind gets the same surface, so none of them grows its own.
macro_rules! ulid_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// Serialises as its 26-character canonical string.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(Ulid);

        impl $name {
            /// Mint the next one. There is no `new()` and no `Default`: an id that was not minted
            /// is a nil id that looks real.
            pub fn generate() -> Self {
                Self(mint())
            }

            /// When it was minted. A ULID carries its own timestamp.
            pub fn created_at(self) -> SystemTime {
                self.0.datetime()
            }

            /// The ULID underneath, for the few places that need one — a directory name, a sort.
            pub fn as_ulid(self) -> Ulid {
                self.0
            }
        }

        /// The bare 26 characters, which is what a directory name and a log line want.
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }

        /// Named, so a debug line says which kind of id it is looking at.
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl FromStr for $name {
            type Err = DecodeError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Ulid::from_str(text).map(Self)
            }
        }
    };
}

ulid_id! {
    /// One pane, and the byte stream it is. Every message in the pane family carries one.
    PaneId
}

ulid_id! {
    /// One session: a named grouping of panes with a folder. Ubiq's sense of the word, not the
    /// harness library's resumable conversation.
    SessionId
}

ulid_id! {
    /// One running workspace, which is also one agent: the work family calls it
    /// [`AgentId`](crate::work::AgentId), an alias of this.
    ///
    /// A workspace is its pane where the host started one, so [`WorkspaceInfo`] carries a
    /// [`PaneId`] instead; the two ids come apart the day a workspace outlives its terminal.
    ///
    /// [`WorkspaceInfo`]: crate::messages::WorkspaceInfo
    WorkspaceId
}

ulid_id! {
    /// One task on the board, and one outline in the graph. Minted by the host and written down,
    /// so it survives a restart — which is why it is a ULID and not a counter over a vector.
    TaskId
}

ulid_id! {
    /// One step of a task. A step is addressed by this rather than by its place in the list: two
    /// clicks in one frame — a remove and a tick — would otherwise arrive as two indices into two
    /// different lists, and the second would land on the wrong step.
    StepId
}

ulid_id! {
    /// One project in the catalogue. Stable across rename, recolour and a move on disk — the path
    /// is a uniqueness key, never the identity.
    ProjectId
}

ulid_id! {
    /// One search across a project. Minted by the interface so that batches arriving after the user
    /// has moved on are discarded by id, the same discipline the generation counter buys for git.
    SearchId
}
