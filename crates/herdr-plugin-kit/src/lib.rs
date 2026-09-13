//! Shared building blocks for Herdr plugins.
//!
//! Three published Herdr plugins each hand-maintain the same socket client, the
//! same environment loader, and the same build shims. This crate ends that
//! duplication, and turns a Herdr release into an ingestion step rather than a
//! manual patch across three repositories.
//!
//! The kit carries the wire types, the socket transport, the launch-contract
//! reader, version reporting, the styled dialogs, issue reporting, and the
//! update check. ⚠️ **None of it has run against a live Herdr server.** Every
//! module that sends anything is exercised against a fake answering what a
//! measured Herdr answers, and no further.
//!
//! # Features
//!
//! Nothing is on by default.
//!
//! | Feature | Turns on | Cost |
//! |---|---|---|
//! | `dialog` | `dialog`, the four-state popup dialogs | `crossterm`, for raw mode |
//! | `report` | `report`, the delivery reason and the issues pane | nothing |
//! | `update` | `update`, the release check and the refresh | nothing |
//!
//! `report` and `update` each cost no dependency, which was measured rather
//! than assumed: `cargo tree` lists `crossterm` for `dialog` and for neither of
//! the other two. `report` opens its pane through `plugin.pane.open`, a socket
//! call, and draws nothing.
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
//! ⚠️ Neither `dialog` nor `report` sends anything itself. Both take a
//! `surface::Transport`, the two socket calls that put something in front of a
//! user, which is how every path through them stays testable without a live
//! server. ✅ [`api::client::Client`] implements that trait once for both, so a
//! consumer supplies nothing.
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
#[cfg(feature = "report")]
pub mod report;
#[cfg(any(feature = "dialog", feature = "report"))]
pub mod surface;
#[cfg(feature = "update")]
pub mod update;
pub mod version;
