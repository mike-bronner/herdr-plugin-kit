//! Shared building blocks for Herdr plugins.
//!
//! Three published Herdr plugins each hand-maintain the same socket client, the
//! same environment loader, and the same build shims. This crate ends that
//! duplication, and turns a Herdr release into an ingestion step rather than a
//! manual patch across three repositories.
//!
//! Today the kit carries the wire types, the launch-contract reader, and
//! version reporting. The transport, report, and update modules land in later
//! stages, in the order SCOPE.md section 13 sets out.
//!
//! # Regenerating
//!
//! [`api::generated`] is produced from Herdr's own published schema and
//! committed. Nothing here needs `cargo-typify` to build, and a plugin that
//! depends on this crate never sees it.

#![forbid(unsafe_code)]

pub mod api;
pub mod env;
