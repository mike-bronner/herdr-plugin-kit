//! Herdr's wire protocol: the generated types, and the one wrapper around them.
//!
//! [`generated`] is machine-written from Herdr's published API schema and is
//! never edited by hand. [`Request`] is the single exception, and
//! `envelope.rs` explains why it has to be.
//!
//! Generated types keep their own module rather than being re-exported here.
//! A glob re-export would let a future generated `Request` shadow the
//! hand-written one silently, which is the same class of quiet breakage the
//! whole pipeline exists to prevent.
//!
//! ```
//! use herdr_plugin_kit::api::{
//!     generated::{PaneListParams, RequestMethod},
//!     Request,
//! };
//!
//! let request = Request {
//!     id: "pick-project-1".to_string(),
//!     method: RequestMethod::PaneList(PaneListParams { workspace_id: None }),
//! };
//!
//! assert_eq!(
//!     serde_json::to_string(&request).unwrap(),
//!     r#"{"id":"pick-project-1","method":"pane.list","params":{}}"#,
//! );
//! ```

pub mod client;
mod envelope;
mod response;

// `large_enum_variant` fires twice inside the generated types, and its fix is
// to box a field. That cannot be applied: the file is machine-written, and the
// next regeneration would drop the change without a word. The lint is allowed
// here rather than inside the generated file so that it stays visible, and so
// that the list can only grow where a human puts it. Every other clippy lint
// still applies to the generated module.
#[allow(clippy::large_enum_variant)]
pub mod generated;

pub use envelope::Request;
pub use generated::{GENERATED_FOR_HERDR_TAG, GENERATED_PROTOCOL, GENERATED_SCHEMA_VERSION};
pub use response::ResponseVariant;
