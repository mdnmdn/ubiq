//! The Archify engine: format, validation, layout, scene, SVG. No GPUI, no Ubiq crate.
//!
//! Everything here is pure: no process, no socket, no window. The host drives it through
//! [`compile`] and [`layout_json`].

pub mod brand;
pub mod compile;
pub mod diag;
pub mod engineering;
pub mod evidence;
pub mod gates;
pub mod geom;
pub mod graph;
pub mod guide;
pub mod i18n;
pub mod labels;
pub mod layout;
pub mod layout_json;
pub mod legend;
pub mod model;
pub mod motion;
pub mod route;
pub mod scene;
pub mod schema;
pub mod sigils;
pub mod svg;
pub mod text;
pub mod tokens;
