//! Re-exported from [`ubiq_proto::drone`].
//!
//! Phase 9's deployer runs from `crates/ubiq/src/app/ssh_connect.rs`, which cannot depend on this
//! crate (`just ui` checks it) — so the manifest lookup, the triple mapping and the hash verifier
//! moved to `ubiq-proto`, the one crate both halves already depend on. This module keeps the old
//! name, `ubiq_host::drone`, working for anything here that still names it. See
//! `ubiq_proto::drone`'s module doc for the reasoning and for `G108` — hash-pinned, not signed.

pub use ubiq_proto::drone::*;
