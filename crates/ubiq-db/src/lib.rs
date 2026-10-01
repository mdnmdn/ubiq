//! ubiq-db — the database engine: connection configs, typed values, SQL analysis and the
//! read-only check, edit rendering and, behind the `drivers` feature, the four engines. A leaf:
//! it names no Ubiq crate and draws nothing.
//!
//! | Part | Modules | Feature |
//! |---|---|---|
//! | Model | [`model`], [`plan`] | default |
//! | Pure engine | [`conn`], [`value`], [`sql`], [`edit`] | default |
//! | Drivers | `driver` | `drivers` |
//!
//! The statement log is `tracing` under the target `ubiq_db::sql`.

pub mod conn;
#[cfg(feature = "drivers")]
pub mod driver;
pub mod edit;
pub mod model;
pub mod plan;
pub mod sql;
pub mod value;

pub use model::{
    ColumnMeta, DbError, DbObject, ExecOptions, ExecOutcome, ObjectKind, Result, ResultSet,
    TableRef,
};
pub use plan::{Plan, PlanFormat, PlanNode};
