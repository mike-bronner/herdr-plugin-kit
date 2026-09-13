//! The two socket calls a module needs to put something in front of a user.
//!
//! 🔑 **Shared because the contract is shared, not because the shapes match.**
//! [`Transport`] began inside `dialog`, named for what that one module
//! required. `report` then needed the same two calls, and the reason is
//! visible in [`Transport::show_notification`]'s own documentation: it was
//! written to SCOPE.md §7.2, which is `report`'s section rather than
//! `dialog`'s. A rule that exists to stop a caller discarding the delivery
//! reason cannot be stated twice, because the second statement is free to
//! drift into exactly the defect §7.2 names.
//!
//! ⚠️ **A shared seam is a coupling, and this one is bounded by keeping the
//! trait at two methods.** If `dialog` ever needs a third call, it declares
//! that call itself rather than widening this. A module here holds only what
//! both modules require.
//!
//! The filesystem work stays out. `dialog`'s answer file and `report`'s issues
//! file are both temporary files keyed on the pid and a counter, and they are
//! still two different things: `dialog` makes a directory it removes once the
//! answer arrives, and `report` writes one file that outlives the call because
//! the popup reads it afterwards. Four similar lines are a cheaper duplicate
//! than a helper that has to be told which of those it is doing.

use crate::api::generated::{NotificationShowParams, NotificationShowReason, PluginPaneOpenParams};

/// The one Herdr error code the kit matches, and the measurement behind it.
///
/// ✅ Reproduced verbatim 2026-09-11 on Herdr 0.9.0, protocol 22, macOS, by
/// opening a second popup while one was up:
///
/// ```text
/// {"code":"ui_busy","message":"a popup pane is already open"}
/// ```
///
/// 🚨 **The single-popup limit is global, not per workspace.** Measured
/// 2026-09-11. So an unanswered dialog in one workspace blocks dialogs in
/// **every** workspace, with nothing on screen to explain it, and the kit
/// cannot even say where the blocker is: a popup has no pane id and is absent
/// from `pane.list`. `ui_busy` is therefore not a rare race to guard against
/// defensively, it is a state a user can sit in indefinitely without knowing.
/// That is what makes the notification fallback worth an extra socket call
/// rather than a nicety.
///
/// ⚠️ **The schema enumerates no error code anywhere** (SCOPE.md §3.5), so
/// nothing here is generatable and nothing here is checkable against the
/// published contract. §4.2.1 is why this constant carries its measurement
/// rather than standing alone: an entry with no measurement beside it is a
/// claim rather than a check, and no guard in the pipeline can tell the two
/// apart.
pub const BUSY_CODE: &str = "ui_busy";

/// Why `plugin.pane.open` did not open a popup.
///
/// 🔑 **`Busy` is separate on purpose.** Only one popup exists at a time, so a
/// caller that loses that race has to be able to learn it did. Flattening
/// `ui_busy` into a generic failure is the same defect SCOPE.md §7.2 was
/// written to fix for notifications: a dropped message and a delivered one
/// indistinguishable to the caller, from information the caller already
/// received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// ✅ Herdr refused because a popup pane is already open ([`BUSY_CODE`]).
    Busy,
    /// Anything else, carrying whatever the sender can say about it.
    Failed(String),
}

impl OpenError {
    /// Classifies a Herdr error body into [`Busy`](OpenError::Busy) or not.
    ///
    /// Implementors of [`Transport`] call this rather than matching the string
    /// themselves, so the one hand-maintained error code in this kit lives in
    /// one place with its measurement beside it.
    pub fn from_error(code: &str, message: &str) -> OpenError {
        match code {
            BUSY_CODE => OpenError::Busy,
            other => OpenError::Failed(format!("{}: {}", other, message)),
        }
    }
}

/// The two socket calls a user-facing module needs, and deliberately no more.
///
/// 🔑 **Named for what its callers require, not for the transport behind it.**
/// Calling it `Herdr` would overclaim: it holds two of the protocol's hundred
/// and two methods, and a consumer reading `impl Herdr for MyClient` would
/// reasonably expect far more. A name describing the requirement also survives
/// a caller needing a third call later, where `Herdr` would have been wrong the
/// whole time and never said so.
///
/// The two callers use it in opposite directions, which is why it holds both
/// calls rather than splitting into one trait each. `dialog` opens a popup and
/// falls back to a notification when the popup is busy. `report` sends a
/// notification and falls back to a popup when the reason says nothing was
/// delivered. Either module alone would justify only half of this.
///
/// Filesystem work stays outside the trait. It is not something a transport
/// owns, and both callers do it differently.
///
/// This shape is not a workaround for the kit's transport being unbuilt. A
/// module that takes its sender as a trait is how this would be designed even
/// with `client.rs` in place, because it is what makes every path testable
/// without a live server. [`api::client::Client`](crate::api::client::Client)
/// implements this trait and no
/// caller changed when it landed.
pub trait Transport {
    /// Sends `plugin.pane.open`. `Ok(())` means Herdr accepted the request.
    ///
    /// ⚠️ **An `Ok` is not evidence that a pane appeared.** `plugin.pane.open`
    /// answers `{"type":"ok"}` for a popup whether or not the process starts,
    /// and carries no handle to ask with. `dialog`'s started marker is what
    /// decides that, and `dialog::ask` is what reads it.
    fn open_pane(&mut self, params: PluginPaneOpenParams) -> Result<(), OpenError>;

    /// Sends `notification.show`, and answers **what Herdr said about
    /// delivery** rather than a bare success.
    ///
    /// 🚨 **Returning the reason is the contract, not a convenience.** SCOPE.md
    /// §7.2 exists because both plugins that send a toast today discard the
    /// whole response, so a dropped message and a delivered one are
    /// indistinguishable from information the caller already received. An
    /// implementation that throws the reason away and answers `Shown`
    /// reintroduces exactly that defect inside the module that fixes it.
    ///
    /// `Err` is for a notification that could not be sent at all, which is a
    /// different fact from one Herdr accepted and chose not to display.
    fn show_notification(
        &mut self,
        params: NotificationShowParams,
    ) -> Result<NotificationShowReason, String>;
}
