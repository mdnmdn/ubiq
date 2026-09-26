//! The extension spine: the one mechanism every customization container in the base rides.
//!
//! No ABI, no dynamic loading, no second mechanism (`D177`). A container is a
//! [`Registry<Spec>`](Registry) plus a spec struct — this module owns the id type and the generic
//! registry only, written once here and never again; a container itself (settings sections, rail
//! modes, …) is a spec struct in its own module, added when it has a base-side user.
//!
//! See `_docs/tech/decisions.md` `D177`–`D178`.

mod id;
pub mod ids;
pub mod rail;
mod registry;
pub mod settings;

pub use id::SlotId;
pub use registry::{Registry, Slotted};
