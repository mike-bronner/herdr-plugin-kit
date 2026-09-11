//! Shared building blocks for Herdr plugins.
//!
//! Three published Herdr plugins each hand-maintain the same socket client, the
//! same environment loader, and the same build shims. This crate ends that
//! duplication, and turns a Herdr release into an ingestion step rather than a
//! manual patch across three repositories.
//!
//! Today the kit carries the wire types, the socket transport, the
//! launch-contract reader, version reporting, and the styled dialogs. The
//! report and update modules land in later stages, in the order SCOPE.md
//! section 13 sets out.
//!
//! # Features
//!
//! Nothing is on by default.
//!
//! | Feature | Turns on | Cost |
//! |---|---|---|
//! | `dialog` | `dialog`, the four-state popup dialogs | `crossterm`, for raw mode |
//!
//! 🔑 Gated because recent-spaces is a headless watcher, and should carry
//! neither popup machinery nor a terminal library.
//!
//! ⚠️ **A gated item is named in prose, never linked, and that is deliberate.**
//! An intra-doc link to `dialog` resolves only in a build that turned the
//! feature on, and reports `broken_intra_doc_links` in every build that did
//! not. A link that is a link in one configuration and a warning in another is
//! the kind of claim this crate does not make elsewhere, so the path is
//! written out instead and the documentation is warning-free either way.
//!
//! ⚠️ `dialog` still does not send anything itself. It takes a
//! `dialog::Transport`, the two socket calls it needs, which is how every
//! path through it stays testable without a live server. ✅
//! [`api::client::Client`] implements that trait, so a consumer supplies
//! nothing.
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
