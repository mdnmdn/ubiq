//! Workflow layout (P6). Port of `renderers/workflow/workflow-compiler.mjs`; `00` section 5.5
//! (Workflow v1, v2); `03` section 2.6.
//!
//! - [`readable`]: the v2 rank solver (`createReadableLayout` and the lane/node geometry the
//!   compiler derives from it), P6.2. The router (P6.3) reads its [`readable::ReadablePlacement`].
//! - [`readable_build`]: v2 end to end (feedback loop, router, viewBox, scene), P6.3.
//! - [`legacy`]: the v1 fixed layout, P6.1.
//! - [`migrate`]: the v1 to v2 projections the `column-capacity` migration fix needs, P6.4.

pub mod legacy;
pub mod migrate;
pub mod readable;
pub mod readable_build;
