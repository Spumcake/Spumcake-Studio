//! The document model both the host and the Storyboard Generator draw from.
//! It has no session, registry, schema of sections and pages, store, or preferences.

pub mod camera;
pub mod defaults;
pub mod document;
pub mod gesture;
pub mod ink;
pub mod prop;
pub mod text;

pub use prop::{Inspect, PropId, PropValue};
