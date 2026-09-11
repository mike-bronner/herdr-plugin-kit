//! Shared building blocks for Herdr plugins.
//!
//! Three published Herdr plugins each hand-maintain the same socket client, the
//! same environment loader, and the same build shims. This crate ends that
//! duplication, and turns a Herdr release into an ingestion step rather than a
//! manual patch across three repositories.
//!
//! Today the kit carries the wire types, the launch-contract reader, version
//! reporting, and the styled dialogs. The transport, report, and update modules
//! land in later stages, in the order SCOPE.md section 13 sets out.
//!
//! # Features
//!
//! Nothing is on by default.
//!
//! | Feature | Turns on | Cost |
//! |---|---|---|
//! | `dialog` | [`dialog`], the four-state popup dialogs | `crossterm`, for raw mode |
//!
//! 🔑 Gated because recent-spaces is a headless watcher, and should carry
//! neither popup machinery nor a terminal library.
//!
//! ⚠️ [`dialog`] does not send anything itself. It takes a
//! [`dialog::PaneOpener`], because the kit's own transport is a later stage.
//!
//! # Regenerating
//!
//! [`api::generated`] is produced from Herdr's own published schema and
//! committed. Nothing here needs `cargo-typify` to build, and a plugin that
//! depends on this crate never sees it.

#![forbid(unsafe_code)]

pub mod api;
#[cfg(feature = "dialog")]
pub mod dialog;
pub mod env;
pub mod version;
