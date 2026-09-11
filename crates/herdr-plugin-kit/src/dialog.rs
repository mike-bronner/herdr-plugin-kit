//! Styled dialogs in four states, drawn in a Herdr popup.
//!
//! Two variants. [`notify`] informs and returns at once. [`ask`] puts a
//! question with two buttons and waits for which one the user chose. See
//! SCOPE.md §7.5.
//!
//! # Why a popup, and what that costs
//!
//! ✅ **Measured 2026-09-11 on Herdr 0.9.0**, protocol 22, in an isolated
//! server, with the rendering captured through a real client in a pty and
//! machine-counted.
//!
//! `plugin.pane.open`'s answer follows the **effective placement**, not the
//! method. `overlay`, `split`, `tab` and `zoomed` all return a
//! `plugin_pane_opened` result carrying a pane object with a `pane_id`.
//! **`popup` alone returns a bare `{"type":"ok"}` with no handle at all.**
//!
//! That is structural rather than an omission. A popup is not in the pane tree:
//! it does not appear in `pane.list`, and its process environment carries no
//! `HERDR_PANE_ID`, `HERDR_TAB_ID` or `HERDR_WORKSPACE_ID`. An overlay's
//! carries all three, with the pane id matching the returned value exactly. So
//! the response carries no id because **there is no id to carry**.
//!
//! | | `popup` | `overlay` |
//! |---|---|---|
//! | Floats over the panes below | ✅ across a divider, both panes surviving | ❌ covers the whole tab, zero rows survive |
//! | `width` / `height` accepted | ✅ percentages | ❌ `invalid_params` |
//! | Returns a `pane_id` | ❌ | ✅ |
//! | In `pane.list` | ❌ | ✅ |
//! | More than one at once | ❌ `ui_busy` | ✅ stacks |
//!
//! 🔑 **Herdr's own dialogs match the popup shape**, captured by the same
//! harness: bordered, centred, partial, with content surviving around them. So
//! this module uses `popup`, and therefore gets no handle.
//!
//! # The two files, which are load-bearing rather than defensive
//!
//! A popup is a **separate process**, and there is no handle to poll and no
//! entry in `pane.list` to find. Herdr also has no plugin-to-plugin channel. So
//! the answer travels through a **file** whose path is handed to the popup in
//! `plugin.pane.open`'s `env` map, and the popup writes its own **process id**
//! to a second file before drawing anything.
//!
//! The marker is what proves a process actually started, because
//! `plugin.pane.open` answers `ok` whether or not it does. Its pid is what lets
//! a popup the user closed be noticed at once rather than waited out.
//!
//! Promoted from `agentic-panes-layout/src/confirm.rs`, where this machinery
//! was solved once against a live server. ⚠️ **Neither file is removable while
//! the placement is `popup`.** Both exist because the measurement above says
//! there is nothing else to ask.
//!
//! Only [`ask`] builds a channel. [`notify`] waits for nothing, so it has
//! nothing to learn and creates no files.
//!
//! # Mouse first, and keyboard in full
//!
//! **Decided by Mike: mouse first**, matching Herdr's own dialogs, rather than
//! a keyboard dialog with clicking bolted on.
//!
//! ✅ Measured 2026-09-10 on 0.9.0, in an isolated server with a real client in
//! a pty, driven by writing SGR sequences and reading what the pane's own
//! process received:
//!
//! - The client turns mouse reporting on for itself the moment it attaches,
//!   emitting `1000h 1002h 1003h 1015h 1006h`. So reporting is on by default.
//! - **A click inside a plugin pane arrives at that pane's own pty**,
//!   SGR-encoded and **rebased to pane-local coordinates**. A click at screen
//!   (60, 20) arrived as `\x1b[<0;14;4M`.
//! - **A click outside the pane is not forwarded at all**, so a dialog cannot
//!   see clicks meant for anything else.
//! - The pane in that run had requested reporting itself, which is why
//!   [`TerminalState`] does the same rather than trusting the client's setting
//!   to reach it.
//! - A control confirmed keystrokes kept arriving throughout, so the two input
//!   methods coexist rather than trading off.
//!
//! Pane-local rebasing is what makes this cheap: [`layout`] already knows where
//! it drew the buttons, and the coordinates arrive in that same space, so
//! hit-testing is a rectangle comparison with nothing to translate.
//!
//! ⚠️ **That measurement was taken on a plugin pane, and this module draws in a
//! popup specifically.** Nobody has confirmed the placement from the source
//! text, so treat click forwarding into a popup as **very likely rather than
//! settled**. Every answer is therefore reachable from the keyboard alone, and
//! a dialog whose clicks never arrive is fully usable rather than unusable.
//!
//! # Which workspace a dialog appears in
//!
//! **Mike's requirement: a dialog must only be visible in the workspace that
//! triggered it**, rather than sitting there after the user switches away.
//!
//! ✅ **Delivered by the default, measured 2026-09-11.** With `workspace_id`
//! unset, a popup opened in one workspace does not appear when the user
//! switches away, and is intact when they return. It persists invisibly, which
//! is exactly the behaviour asked for.
//!
//! 🚨 **Setting `workspace_id` is not merely unnecessary, it is refused.** The
//! same measurement: `plugin.pane.open` with `workspace_id` and `popup`
//! placement answers `invalid_params`, with the message that overlay and popup
//! plugin panes target the active pane. A nonexistent workspace id produces the
//! **identical** error, so the refusal fires on the parameter being present
//! rather than on any lookup failing.
//!
//! So sending it would have meant no dialog at all. [`open_params`] leaves it
//! unset, and now does so on a measurement rather than on caution.
//!
//! Confinement to the triggering *pane* was explicitly not chosen, and is
//! contradicted by measurement anyway: a popup floats centred over the whole
//! tab and was captured across a pane divider, and there is no position
//! parameter at all.
//!
//! # ⚠️ Colour is a proposal, and was never measured
//!
//! The probe captured **geometry and characters, not colour**: the capture
//! library discards SGR attributes, so borders and titles were confirmed
//! present but never confirmed coloured. **Nobody has established how Herdr
//! colours its own dialogs.** The palette in [`State::colour`] is a proposal
//! that looks right, not a match that was verified.
//!
//! Inversion is different. It is a standard terminal attribute rather than a
//! claim about Herdr, so the primary button will render as intended.
//!
//! SCOPE.md §7.1 records why this distinction is drawn so sharply: four
//! documented Herdr claims in three days were right about a conclusion and
//! wrong about the mechanism behind it.
//!
//! # The `crossterm` dependency, and what it costs
//!
//! ⚠️ **This fails the dependency bar the kit applied in §5.** `toml` was
//! accepted because all three donor plugins already depended on it directly.
//! `crossterm` is in agentic-panes-layout's graph **only**, so two of the three
//! consumers do gain a dependency by adopting this module.
//!
//! It is justified anyway, on two grounds. Reading one keypress needs raw mode,
//! and the alternative is hand-rolled termios, which `lib.rs`'s
//! `#![forbid(unsafe_code)]` rules out. And the `dialog` feature is off by
//! default, so recent-spaces, a headless watcher, never compiles it.
//!
//! **The drawing is hand-written ANSI.** The dependency buys input and terminal
//! state and nothing else: raw mode, key decoding, SGR mouse decoding, and the
//! three-state teardown in [`TerminalState`]. Every character of the frame is
//! this module's own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::api::generated::{
    NotificationShowParams, NotificationShowReason, PluginPaneOpenParams, PluginPanePlacement,
    PopupSize, PopupSizeString,
};
use crate::env::Environment;

/// The pane entrypoint every consuming plugin declares for its dialogs.
///
/// The kit fixes the convention so the two halves cannot disagree. Each plugin
/// declares the matching `[[panes]]` entry, which a crate cannot supply.
pub const ENTRYPOINT: &str = "dialog";

/// Which of the four states to draw.
pub const STATE_VAR: &str = "HERDR_PLUGIN_DIALOG_STATE";

/// The dialog's title, drawn in the border beside the glyph.
pub const TITLE_VAR: &str = "HERDR_PLUGIN_DIALOG_TITLE";

/// The dialog's body. Newlines separate paragraphs, and each is wrapped.
pub const BODY_VAR: &str = "HERDR_PLUGIN_DIALOG_BODY";

/// The primary button's label. **Its absence is what makes a dialog bare.**
pub const PRIMARY_VAR: &str = "HERDR_PLUGIN_DIALOG_PRIMARY";

/// The cancel affordance's label.
pub const CANCEL_VAR: &str = "HERDR_PLUGIN_DIALOG_CANCEL";

/// Where the popup writes which button the user chose.
pub const ANSWER_FILE_VAR: &str = "HERDR_PLUGIN_DIALOG_ANSWER_FILE";

/// Where the popup writes its own process id, before drawing anything.
pub const STARTED_FILE_VAR: &str = "HERDR_PLUGIN_DIALOG_STARTED_FILE";

/// The word the popup writes when the user chose the primary button.
pub const PRIMARY_WORD: &str = "primary";

/// The word the popup writes when the user cancelled.
pub const CANCEL_WORD: &str = "cancel";

/// The label a dialog uses when the caller names no cancel affordance.
pub const DEFAULT_CANCEL: &str = "Cancel";

/// The key affordance drawn on the primary button, by the kit and not the caller.
///
/// 🔑 **The kit draws this, so three plugins cannot disagree about it.** A
/// caller writing its own `↵` into a label is how the symbol, the spacing, or
/// whether to bother at all drift apart across repositories, which is the exact
/// class of divergence this crate exists to end.
///
/// It matters more now the dialogs are mouse first: a button somebody clicks
/// still has to advertise the key for somebody who will not.
///
/// U+21B5 rather than U+23CE, and neither has an emoji presentation, so no
/// text-presentation selector is needed on either.
pub const PRIMARY_KEY: &str = "\u{21b5}";

/// The key affordance drawn on the cancel affordance. See [`PRIMARY_KEY`].
///
/// A word rather than a glyph, because no single character means Escape and an
/// invented one would have to be learned.
pub const CANCEL_KEY: &str = "esc";

/// The one Herdr error code this module matches, and the measurement behind it.
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

/// The popup's requested width, as a percentage of the tab.
///
/// ⚠️ **A proposal.** ✅ What was measured is only that percentages are
/// accepted on `popup` and rejected on everything else, and that `60% x 30%`
/// produced a 53x9 cell box at a 120x40 viewport. There is no position
/// parameter at all.
pub const WIDTH: &str = "60%";

/// The popup's requested height, as a percentage of the tab.
///
/// ⚠️ **A proposal, and deliberately larger than the donor's 30%.** The frame
/// below needs nine rows for a single line of body text, and 30% measured nine
/// rows at a 40-row viewport. That is exactly the edge, so a second body line
/// would have scrolled.
pub const HEIGHT: &str = "40%";

/// Blank rows inside the border, at the top and again at the bottom.
///
/// Herdr's own dialogs sit tight against the frame. Mike asked for breathing
/// room, so this is a deliberate divergence from what was captured.
const VERTICAL_PADDING: usize = 2;

/// Blank columns inside the border, on each side.
const SIDE_PADDING: usize = 3;

/// Blank columns between the primary button and the cancel affordance.
const BUTTON_GAP: usize = 2;

/// The narrowest frame that still has a column of text inside it.
///
/// Below this the borders, the side padding, and a one-character title would
/// collide. A caller asking for less gets this instead, because a frame that
/// cannot be drawn correctly is worse than one that is wider than asked.
const MIN_WIDTH: usize = 24;

/// The width [`run`] draws to when the terminal will not report its own size.
const FALLBACK_WIDTH: usize = 60;

/// How long [`ask`] waits for an answer before giving up.
///
/// Generous, because the user may have walked away mid-task.
const WAIT: Duration = Duration::from_secs(120);

/// How long to wait for the popup to report that it started.
///
/// Short, because this is not the user thinking: it is the gap between asking
/// Herdr for a pane and a process running in it. `plugin.pane.open` answers
/// `ok` whether or not the process starts, so without this bound a popup that
/// failed to launch would freeze the caller for the full [`WAIT`].
const STARTUP: Duration = Duration::from_secs(3);

/// How often to look for an answer.
const POLL: Duration = Duration::from_millis(100);

/// How often to check the popup is still alive.
///
/// Slower than [`POLL`] because each check spawns a process.
const LIVENESS: Duration = Duration::from_millis(500);

const RESET: &str = "\u{1b}[0m";
const INVERSE: &str = "\u{1b}[7m";
/// The hover mark. It occupies no cells, so the pointer can never move the frame.
const UNDERLINE: &str = "\u{1b}[4m";
/// Clear the pane and put the cursor at its origin.
///
/// 🔑 Every redraw starts here, so the frame always begins at pane-local (0, 0).
/// That is what makes [`Rect`]'s rows line up with the coordinates a click
/// arrives carrying.
const HOME: &str = "\u{1b}[2J\u{1b}[H";

/// Which of the four states a dialog is in.
///
/// 🔑 **Each carries a glyph as well as a colour, and that is the point.** A
/// glyph survives a monochrome terminal, where colour alone cannot separate a
/// warning from a danger.
///
/// All four use the same rounded frame. ⚠️ **Varying the corner shape for
/// `Danger` was proposed and rejected by Mike**, on reasoning worth keeping:
/// the glyph is already the non-colour channel, so a second one is redundant
/// and costs consistency for nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Something the user may want to know.
    Info,
    /// Something finished, and it worked.
    Success,
    /// Something is wrong, and the work continued.
    Warning,
    /// Something is wrong, and it stopped the work or will destroy something.
    Danger,
}

impl State {
    /// How many terminal cells every glyph occupies.
    ///
    /// 🔑 **One, uniformly, which makes it a constant rather than a special
    /// case.** All four glyphs are East Asian Width `Neutral`, so the title line
    /// pays the same width in every state and no state can misalign against the
    /// others.
    ///
    /// It is named rather than inlined because the title line computes its width
    /// instead of measuring it, and a future glyph of a different width has to
    /// change one place. The test suite pins it against the real Unicode data.
    pub const GLYPH_CELLS: usize = 1;

    /// The glyph drawn in the border, on the title line.
    ///
    /// 🔑 **Plain BMP symbols: one codepoint, no variation selector, no
    /// emoji.** That combination is what makes them safe rather than what makes
    /// them pretty, and it took four attempts to land on, each earlier one
    /// failing a width property nobody had checked before choosing.
    ///
    /// Three traps avoided, and each was hit in turn:
    ///
    /// - ⚠️ **A variation selector** turns a text symbol into an emoji and makes
    ///   the glyph two codepoints. It is also the specific trigger for
    ///   Terminal.app drawing a Neutral base character two cells wide while
    ///   advancing the cursor one.
    /// - ⚠️ **Emoji** are East Asian Width `Wide`, so they cost two cells. Any
    ///   set mixing them with narrow characters misaligns one title against the
    ///   rest.
    /// - 🚨 **East Asian Width `Ambiguous` is worse than either**, because the
    ///   *terminal* decides the width from a setting rather than the character
    ///   deciding it. U+24D8, U+2299 and U+25B2 are all Ambiguous, and enabling
    ///   that setting in Terminal.app visibly breaks box drawing, which is
    ///   exactly what this frame is made of.
    ///
    /// Private Use Area codepoints are excluded for a separate reason: they
    /// need a patched font, and this kit is public and feeds three plugins that
    /// other people install. A user without a Nerd Font would see tofu.
    ///
    /// ⚠️ **Not Herdr's own dialog icons.** Herdr's dialogs carry no glyph at
    /// all, which is visible in the screenshots this design was compared
    /// against. These were chosen for this kit. Describing them otherwise would
    /// be the fifth documented claim on this project to be right about a
    /// conclusion and wrong about its source.
    pub fn glyph(self) -> &'static str {
        match self {
            // U+229D CIRCLED DASH
            State::Info => "\u{229d}",
            // U+2713 CHECK MARK
            State::Success => "\u{2713}",
            // U+26A0 WARNING SIGN, bare and deliberately without U+FE0E
            State::Warning => "\u{26a0}",
            // U+2716 HEAVY MULTIPLICATION X
            State::Danger => "\u{2716}",
        }
    }

    /// The SGR foreground parameter for this state.
    ///
    /// ⚠️ **Unmeasured, and a proposal.** The probe that settled everything
    /// else about this dialog captured geometry and characters but not colour,
    /// because the capture library discards SGR attributes. Nobody knows how
    /// Herdr colours its own dialogs. See the module documentation.
    ///
    /// These are the eight basic colours and their bright variants rather than
    /// 256-colour or truecolour values, so a themed terminal maps each one to
    /// the palette the user already chose.
    ///
    /// ⚠️ **Info is the bright variant and the other three are not, and that
    /// asymmetry is deliberate rather than an oversight.** Mike saw the palette
    /// rendered on 2026-09-11 and asked for light blue on info alone; 94 is the
    /// bright form of the same basic blue, so it stays inside the eight-plus-
    /// eight set and keeps the property the whole palette was chosen for. The
    /// other three were not part of that instruction and were left as they are.
    pub fn colour(self) -> u8 {
        match self {
            State::Info => 94,
            State::Success => 32,
            State::Warning => 33,
            State::Danger => 31,
        }
    }

    /// The word that carries this state across the process boundary.
    pub fn as_wire(self) -> &'static str {
        match self {
            State::Info => "info",
            State::Success => "success",
            State::Warning => "warning",
            State::Danger => "danger",
        }
    }

    /// Reads a state back, distinguishing "asked for nothing" from "disagreed".
    ///
    /// 🔑 **The two absences are not the same fact, and fail in different
    /// directions.** A variable that is absent or empty means the caller asked
    /// for no particular state, so [`State::Info`] is right: nothing has
    /// claimed anything is wrong.
    ///
    /// An **unrecognised** word means the two halves of one binary disagree
    /// about the vocabulary, which should be impossible. That answers
    /// [`State::Warning`], because flagging a mismatch beats drawing a dialog
    /// that understates what it is about.
    pub fn from_wire(word: Option<&str>) -> State {
        match word.map(str::trim).filter(|word| !word.is_empty()) {
            None => State::Info,
            Some("info") => State::Info,
            Some("success") => State::Success,
            Some("warning") => State::Warning,
            Some("danger") => State::Danger,
            Some(_) => State::Warning,
        }
    }
}

/// What a dialog says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dialog {
    /// Which of the four states, deciding the glyph and the colour.
    pub state: State,
    /// Drawn in the border, beside the glyph.
    pub title: String,
    /// Drawn inside the frame. Newlines separate paragraphs.
    pub body: String,
}

impl Dialog {
    /// Builds a dialog.
    pub fn new(state: State, title: &str, body: &str) -> Dialog {
        Dialog {
            state,
            title: title.to_string(),
            body: body.to_string(),
        }
    }
}

/// The two affordances an actioned dialog offers.
///
/// **The labels are the caller's. The keys are the kit's.** A button is drawn
/// as its key affordance, a space, then the label, so `Buttons::new("close
/// anyway", "keep")` draws `↵ close anyway` and `esc keep` without the caller
/// typing either. See [`PRIMARY_KEY`] for why that boundary sits there.
///
/// The kit also fixes how they are drawn: the primary inverted in the state's
/// colour, and the cancel as plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Buttons {
    /// The action the dialog is asking about. **Enter chooses this one.**
    pub primary: String,
    /// The way out. Escape and Ctrl-C both choose this one.
    pub cancel: String,
}

impl Buttons {
    /// Builds a button pair, defaulting the cancel label when it is empty.
    pub fn new(primary: &str, cancel: &str) -> Buttons {
        Buttons {
            primary: primary.to_string(),
            cancel: match cancel.trim().is_empty() {
                true => DEFAULT_CANCEL.to_string(),
                false => cancel.to_string(),
            },
        }
    }
}

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
    /// themselves, so the one hand-maintained error code in this module lives
    /// in one place with its measurement beside it.
    pub fn from_error(code: &str, message: &str) -> OpenError {
        match code {
            BUSY_CODE => OpenError::Busy,
            other => OpenError::Failed(format!("{}: {}", other, message)),
        }
    }
}

/// The two socket calls a dialog needs, and deliberately no more.
///
/// 🔑 **Named for what this module requires, not for the transport behind
/// it.** Calling it `Herdr` would overclaim: it holds two of the protocol's
/// hundred and two methods, and a consumer reading `impl Herdr for MyClient`
/// would reasonably expect far more. A name describing the requirement also
/// survives dialogs needing a third call later, where `Herdr` would have been
/// wrong the whole time and never said so.
///
/// The answer file and the process-id marker stay outside this trait. They are
/// filesystem work, and not something a transport owns.
///
/// This shape is not a workaround for the kit's transport being unbuilt. A
/// module that takes its sender as a trait is how this would be designed even
/// with `client.rs` in place, because it is what makes every path here testable
/// without a live server. When SCOPE.md §4.2 lands, the kit's own client
/// implements this trait and no caller changes.
pub trait Transport {
    /// Sends `plugin.pane.open`. `Ok(())` means Herdr accepted the request.
    ///
    /// ⚠️ **An `Ok` is not evidence that a pane appeared.** `plugin.pane.open`
    /// answers `{"type":"ok"}` for a popup whether or not the process starts,
    /// and carries no handle to ask with. The started marker is what decides
    /// that, and [`ask`] is what reads it.
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

/// What became of a notification sent because a popup was unavailable.
///
/// Two cases, because they are different facts. Herdr accepting a notification
/// and declining to display it is not the same as a notification that never
/// reached Herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Explained {
    /// Herdr accepted it, and answered with this delivery reason.
    ///
    /// ⚠️ **Not necessarily seen.** [`NotificationShowReason::Disabled`],
    /// `RateLimited`, `NoForegroundClient` and `Busy` all mean the user was
    /// told nothing. The reason is handed back rather than judged, exactly as
    /// SCOPE.md §7.2 requires.
    Reason(NotificationShowReason),
    /// It could not be sent, so the user was told nothing and nothing knows why.
    Unreachable(String),
}

/// What became of a bare dialog.
///
/// 🔑 **Four states, because four things genuinely differ**: the popup opened,
/// it was busy and the user was told another way, it was busy and the user was
/// told nothing, or the request failed outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shown {
    /// Herdr accepted the request.
    Opened,
    /// ✅ A popup was already open, so the message went to a notification
    /// instead, and Herdr answered with this delivery reason.
    ///
    /// ⚠️ Read the reason. Only [`NotificationShowReason::Shown`] means the
    /// user saw anything.
    Notified(NotificationShowReason),
    /// A popup was already open **and** the notification could not be sent, so
    /// nothing reached the user by either route.
    Unreachable(String),
    /// The request failed for some reason other than a busy popup.
    Failed(String),
}

/// Why nobody chose a button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unanswered {
    /// ✅ A popup was already open, so the question was never asked.
    ///
    /// 🔑 **A notification cannot stand in for a question**, so this is still
    /// an unanswered dialog and the caller still learns it got no answer. The
    /// notification only explains *why* nothing appeared, and what it carries
    /// is whether that explanation reached the user.
    Busy(Explained),
    /// The popup could not be opened, or its channel could not be made.
    Failed(String),
    /// The popup process died without answering, which is what closing it does.
    Dismissed,
    /// The popup never reported that it started, so it never drew.
    ///
    /// ⚠️ This marker is the only evidence either way. `plugin.pane.open`
    /// answers `ok` regardless, and a popup cannot be found in `pane.list`.
    NeverShown,
    /// Nobody answered inside [`WAIT`].
    TimedOut,
    /// The channel carried a word that is neither choice.
    ///
    /// Kept apart from [`Unanswered::Dismissed`] because it means something
    /// different: a popup left over from an older build, or a channel somebody
    /// else wrote into.
    Unrecognised(String),
}

/// What the user chose, or why nobody did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The user chose the primary button.
    Primary,
    /// The user chose the cancel affordance.
    Cancel,
    /// Nobody chose, and this is why.
    Unanswered(Unanswered),
}

impl Answer {
    /// Whether the user explicitly chose the primary button.
    ///
    /// 🔑 **The safety property, as one call.** Every other outcome, including
    /// every failure to open, to start, or to be answered, answers `false`. A
    /// caller acting on an actioned dialog asks this rather than matching,
    /// because a match written the other way round acts on silence.
    pub fn chose_primary(&self) -> bool {
        matches!(self, Answer::Primary)
    }
}

/// Shows a dialog and returns at once, without waiting for the user.
///
/// 🔑 **Never gates the caller, and that is a requirement rather than an
/// optimisation.** SCOPE.md §7.4 states it for the informational case: a
/// cosmetic warning must not make somebody wait on a dialog to get their
/// workspace. So there is no channel here, no marker, and nothing to poll.
///
/// The dialog carries no buttons and dismisses on any key or any click.
///
/// **A busy popup falls back to a notification**, carrying the same title and
/// body. The user sees the message rather than nothing. See [`BUSY_CODE`] for
/// why that fallback is worth an extra socket call.
///
/// ⚠️ **The return says only what Herdr answered.** A [`Shown::Opened`] is not
/// evidence the user saw anything, because a popup hands back no handle to ask
/// with, and a [`Shown::Notified`] carries a reason that may well mean the
/// notification was dropped too.
pub fn notify(transport: &mut impl Transport, plugin_id: &str, dialog: &Dialog) -> Shown {
    match transport.open_pane(open_params(plugin_id, dialog, None, None)) {
        Ok(()) => Shown::Opened,
        Err(OpenError::Failed(why)) => Shown::Failed(why),
        // A dialog that only informs can say the same thing through a
        // notification, because it needs nothing back from the user.
        Err(OpenError::Busy) => match explain(transport, &dialog.title, &dialog.body) {
            Explained::Reason(reason) => Shown::Notified(reason),
            Explained::Unreachable(why) => Shown::Unreachable(why),
        },
    }
}

/// Asks a question in a dialog and waits for which button the user chose.
///
/// This variant necessarily waits, because its answer is the thing the caller
/// asked for. [`notify`] is the variant that must not.
///
/// 🚨 **A busy popup does not become a notification here.** A notification
/// cannot collect an answer, so silently turning a question into a statement
/// would lose the answer and tell the caller nothing about it. Instead the
/// caller still gets [`Unanswered::Busy`], and a notification separately
/// explains to the user why no dialog appeared. Both halves are reported: the
/// caller learns it has no answer, and it learns whether the explanation landed.
///
/// Every way of not being answered is reported rather than swallowed, and none
/// of them answers [`Answer::Primary`]. Use [`Answer::chose_primary`].
pub fn ask(
    transport: &mut impl Transport,
    plugin_id: &str,
    dialog: &Dialog,
    buttons: &Buttons,
) -> Answer {
    let channel = match Channel::new() {
        Ok(channel) => channel,
        Err(e) => {
            return Answer::Unanswered(Unanswered::Failed(format!(
                "cannot make a channel for the question: {}",
                e
            )))
        }
    };

    match transport.open_pane(open_params(
        plugin_id,
        dialog,
        Some(buttons),
        Some(&channel),
    )) {
        Ok(()) => decide(channel.watch()),
        Err(OpenError::Failed(why)) => Answer::Unanswered(Unanswered::Failed(why)),
        Err(OpenError::Busy) => {
            let body = format!(
                "{} This question could not be asked, because another dialog is \
                 already open.",
                dialog.body
            );
            Answer::Unanswered(Unanswered::Busy(explain(transport, &dialog.title, &body)))
        }
    }
}

/// Tells the user something through a notification, and reports what happened.
///
/// 🚨 **Reads the reason rather than discarding it**, which is the whole of
/// SCOPE.md §7.2. A fallback that silently fails is worse than no fallback,
/// because it removes the caller's last signal that anything went wrong.
fn explain(transport: &mut impl Transport, title: &str, body: &str) -> Explained {
    let params = NotificationShowParams {
        body: Some(body.to_string()),
        position: None,
        sound: None,
        title: title.to_string(),
    };
    match transport.show_notification(params) {
        Ok(reason) => Explained::Reason(reason),
        Err(why) => Explained::Unreachable(why),
    }
}

/// Builds the `plugin.pane.open` request.
///
/// ⚠️ **The placement is `popup` and is not a parameter.** An overlay would
/// hand back a `pane_id`, and that is exactly the shortcut this module refuses:
/// ✅ an overlay covers the entire tab with zero rows of any underlying pane
/// surviving, so it is not a dialog. See the module documentation.
///
/// # `workspace_id` is left unset, and that is measured rather than cautious
///
/// ✅ **Measured 2026-09-11.** Sending it is refused: `plugin.pane.open` with
/// `workspace_id` and `popup` placement answers `invalid_params`, with the
/// message that overlay and popup plugin panes target the active pane. A
/// nonexistent workspace id gives the identical error, so the refusal is about
/// the parameter being present, not about the lookup.
///
/// ✅ And it is unnecessary: with the parameter unset, a popup is visible only
/// in the workspace that opened it. It does not follow the user to another
/// workspace, and it is intact on return.
///
/// That is the same shape as `width` and `height`, which are accepted only for
/// `popup` and refused everywhere else. Herdr validates parameters against the
/// effective placement in both directions.
///
fn open_params(
    plugin_id: &str,
    dialog: &Dialog,
    buttons: Option<&Buttons>,
    channel: Option<&Channel>,
) -> PluginPaneOpenParams {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(STATE_VAR.to_string(), dialog.state.as_wire().to_string());
    env.insert(TITLE_VAR.to_string(), dialog.title.clone());
    env.insert(BODY_VAR.to_string(), dialog.body.clone());
    if let Some(buttons) = buttons {
        env.insert(PRIMARY_VAR.to_string(), buttons.primary.clone());
        env.insert(CANCEL_VAR.to_string(), buttons.cancel.clone());
    }
    if let Some(channel) = channel {
        env.insert(
            ANSWER_FILE_VAR.to_string(),
            channel.answer.to_string_lossy().into_owned(),
        );
        env.insert(
            STARTED_FILE_VAR.to_string(),
            channel.started.to_string_lossy().into_owned(),
        );
    }

    PluginPaneOpenParams {
        cwd: None,
        direction: None,
        entrypoint: ENTRYPOINT.to_string(),
        env,
        focus: true,
        height: popup_size(HEIGHT),
        placement: Some(PluginPanePlacement::Popup),
        plugin_id: plugin_id.to_string(),
        target_pane_id: None,
        width: popup_size(WIDTH),
        workspace_id: None,
    }
}

/// Builds a percentage size, or nothing when the text is not one.
///
/// The schema constrains these to `^(100|[1-9][0-9]?)%$`, so a malformed value
/// cannot be represented. Answering `None` drops the size rather than the whole
/// request, and the test suite pins [`WIDTH`] and [`HEIGHT`] as valid so an
/// edit that breaks one reddens instead of silently unsizing every dialog.
fn popup_size(percent: &str) -> Option<PopupSize> {
    PopupSizeString::try_from(percent)
        .ok()
        .map(PopupSize::String)
}

/// How a wait ended, separated from what it means.
///
/// The split exists so every ending is testable. [`WAIT`] is 120 seconds, so a
/// test reaching it by waiting would take 120 seconds. Keeping the decision
/// pure means it is checked in microseconds instead, and the branch cannot rot
/// unnoticed just because it is slow to provoke.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ended {
    Answered(String),
    Dismissed,
    NeverShown,
    TimedOut,
}

/// What an ending means.
///
/// **Only an exact word is acted on.** Everything else is reported as
/// unanswered, and none of it can ever answer [`Answer::Primary`].
fn decide(ended: Ended) -> Answer {
    match ended {
        Ended::Answered(word) => match word.as_str() {
            PRIMARY_WORD => Answer::Primary,
            CANCEL_WORD => Answer::Cancel,
            other => Answer::Unanswered(Unanswered::Unrecognised(other.to_string())),
        },
        Ended::Dismissed => Answer::Unanswered(Unanswered::Dismissed),
        Ended::NeverShown => Answer::Unanswered(Unanswered::NeverShown),
        Ended::TimedOut => Answer::Unanswered(Unanswered::TimedOut),
    }
}

/// The two files the popup writes, in a directory this process makes and removes.
struct Channel {
    answer: PathBuf,
    started: PathBuf,
    dir: PathBuf,
}

impl Channel {
    fn new() -> std::io::Result<Channel> {
        // Unique per call, not merely per process. ✅ SCOPE.md §7.4 records
        // that keying on the pid alone caused a real race which failed a test
        // suite about one run in four: every call shared one path, so a
        // lingering first popup deleted the second question's answer file, and
        // the second then timed out having been answered.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("herdr-plugin-dialog-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir)?;
        Ok(Channel {
            answer: dir.join("answer"),
            started: dir.join("started"),
            dir,
        })
    }

    fn read(path: &Path) -> Option<String> {
        let text = std::fs::read_to_string(path).ok()?;
        let text = text.trim().to_string();
        match text.is_empty() {
            true => None,
            false => Some(text),
        }
    }

    fn watch(&self) -> Ended {
        self.watch_within(STARTUP, WAIT)
    }

    /// The wait, with its two deadlines supplied.
    ///
    /// A test seam and not a knob, for the same reason
    /// [`Environment::from_pairs`] is one: without it, covering
    /// [`Ended::TimedOut`] would cost the suite two minutes of real time, and a
    /// branch nobody can afford to test is a branch that rots.
    fn watch_within(&self, startup: Duration, wait: Duration) -> Ended {
        let pid = match self.await_start(startup) {
            Some(pid) => pid,
            None => {
                // An answer already there means the popup ran and finished
                // inside the startup window, which is the ordinary case for a
                // fast answer.
                return match Self::read(&self.answer) {
                    Some(word) => Ended::Answered(word),
                    None => Ended::NeverShown,
                };
            }
        };

        let deadline = Instant::now() + wait;
        let mut last_liveness_check = Instant::now();
        while Instant::now() < deadline {
            if let Some(word) = Self::read(&self.answer) {
                return Ended::Answered(word);
            }
            if last_liveness_check.elapsed() > LIVENESS {
                if !process_is_alive(&pid) {
                    // It died without answering, which is what closing the
                    // popup does. Read once more first: it may have answered
                    // and exited between the two checks.
                    return match Self::read(&self.answer) {
                        Some(word) => Ended::Answered(word),
                        None => Ended::Dismissed,
                    };
                }
                last_liveness_check = Instant::now();
            }
            std::thread::sleep(POLL);
        }
        Ended::TimedOut
    }

    fn await_start(&self, startup: Duration) -> Option<String> {
        let deadline = Instant::now() + startup;
        while Instant::now() < deadline {
            if let Some(pid) = Self::read(&self.started) {
                return Some(pid);
            }
            if Self::read(&self.answer).is_some() {
                return None;
            }
            std::thread::sleep(POLL);
        }
        None
    }
}

impl Drop for Channel {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Whether a process is still alive.
///
/// `kill -0` is a signal-free existence test. Spawning it is not free, so it
/// runs at a slower cadence than the answer poll. There is no `libc` here on
/// purpose: one crate for one syscall is a supply-chain surface this kit does
/// not need, and `lib.rs` forbids the unsafe block that would call it directly.
///
/// **If the test itself cannot run, absence is not proven**, so it answers
/// alive and the wait falls through to [`WAIT`].
#[cfg(unix)]
fn process_is_alive(pid: &str) -> bool {
    match std::process::Command::new("/bin/kill")
        .arg("-0")
        .arg(pid)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(_) => true,
    }
}

/// ⚠️ **Windows has no equivalent here, so absence is never proven.**
///
/// The consequence is precise and it is safe: a popup the user closes is not
/// noticed at once, and the question runs out [`WAIT`] and answers
/// [`Unanswered::TimedOut`] rather than [`Unanswered::Dismissed`]. Slow rather
/// than wrong, and no outcome becomes [`Answer::Primary`].
///
/// Compile-verified only, like everything else Windows in this kit. Nobody on
/// this project has Windows hardware. See the README.
#[cfg(not(unix))]
fn process_is_alive(_pid: &str) -> bool {
    true
}

// ── The popup half ────────────────────────────────────────────────────────

/// What the popup half was asked to draw, read from its own environment.
///
/// Separated from [`run`] so the whole reading step is testable: it is pure
/// over an [`Environment`], and [`Environment::from_pairs`] builds one without
/// touching process globals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    /// What to draw.
    pub dialog: Dialog,
    /// The buttons, or `None` for a bare dialog that dismisses on any key.
    pub buttons: Option<Buttons>,
    /// Where to write the chosen word.
    pub answer_file: Option<PathBuf>,
    /// Where to write this process's id, before drawing anything.
    pub started_file: Option<PathBuf>,
}

impl Popup {
    /// Reads what to draw out of the environment Herdr launched this pane with.
    ///
    /// 🔑 **The primary label's absence is what makes a dialog bare.** There is
    /// no separate flag, because a bare dialog is exactly one with no button to
    /// label, and two ways of saying so could disagree.
    ///
    /// Every variable that is set but empty counts as absent, which is the
    /// idiom [`crate::env`] documents at nearly every call site.
    pub fn from_env(env: &Environment) -> Popup {
        let value = |key: &str| env.get(key).filter(|text| !text.is_empty());
        Popup {
            dialog: Dialog {
                state: State::from_wire(env.get(STATE_VAR)),
                title: value(TITLE_VAR).unwrap_or_default().to_string(),
                body: value(BODY_VAR).unwrap_or_default().to_string(),
            },
            buttons: value(PRIMARY_VAR)
                .map(|primary| Buttons::new(primary, value(CANCEL_VAR).unwrap_or_default())),
            answer_file: value(ANSWER_FILE_VAR).map(PathBuf::from),
            started_file: value(STARTED_FILE_VAR).map(PathBuf::from),
        }
    }
}

/// Which button the pointer is over, or which one a click landed on.
///
/// One type for hover and for hit-testing, because they ask the same question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hot {
    /// Neither button. A click here resolves nothing.
    #[default]
    None,
    /// The primary button.
    Primary,
    /// The cancel affordance.
    Cancel,
}

/// Where a button was drawn, in the pane's own coordinates.
///
/// 🔑 **Pane-local, and that is what makes hit-testing a rectangle
/// comparison.** ✅ Measured 2026-09-10 on 0.9.0 with a real client in a pty: a
/// click inside a plugin pane arrives at that pane's pty SGR-encoded and
/// **rebased to the pane's own origin**. A click at screen (60, 20) arrived as
/// `\x1b[<0;14;4M`, and one at (70, 22) as `<0;24;6M`, a constant offset. So
/// the coordinates a dialog receives are already in the space it drew in, and
/// nothing has to be translated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Rows from the top of the pane, counting from zero.
    pub row: u16,
    /// Columns from the left of the pane, counting from zero.
    pub column: u16,
    /// How many cells wide, including the button's own padding.
    pub width: u16,
}

impl Rect {
    /// Whether a pane-local position falls inside this rectangle.
    ///
    /// One row tall, because a button is one row tall.
    pub fn contains(&self, column: u16, row: u16) -> bool {
        row == self.row && column >= self.column && column < self.column + self.width
    }
}

/// A drawn dialog: the characters, and where the buttons ended up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The whole dialog, newline-separated, with its escape sequences.
    pub text: String,
    /// Where the primary button was drawn, or `None` on a bare dialog.
    pub primary: Option<Rect>,
    /// Where the cancel affordance was drawn, or `None` on a bare dialog.
    pub cancel: Option<Rect>,
}

impl Frame {
    /// Which button is at this pane-local position.
    ///
    /// ⚠️ **A position on neither button answers [`Hot::None`]**, and an
    /// actioned dialog treats that as no answer at all. Clicking the body text
    /// must not resolve a question whose primary button may be destructive.
    pub fn hit(&self, column: u16, row: u16) -> Hot {
        if self.primary.is_some_and(|r| r.contains(column, row)) {
            return Hot::Primary;
        }
        if self.cancel.is_some_and(|r| r.contains(column, row)) {
            return Hot::Cancel;
        }
        Hot::None
    }
}

/// Draws the dialog, borders, padding, colour and all.
///
/// The layout Mike approved, from the top:
///
/// ```text
/// ╭─ ⚠ Title ──────────────────────────╮
/// │                                    │
/// │                                    │
/// │   The body, wrapped to the width.  │
/// │                                    │
/// │           [ Primary ]  Cancel      │
/// │                                    │
/// ╰────────────────────────────────────╯
/// ```
///
/// Two blank rows inside the border at the top, three columns of padding on
/// each side, and a blank row separating the body from the button row so the
/// buttons do not read as another line of body text.
///
/// 🔑 **The bottom padding is two rows, except below a button row, where it is
/// one.** Decided by Mike: the button row is already visually heavy enough not
/// to need the extra breathing space beneath it. A bare dialog has no button
/// row, so it keeps two rows at the bottom as well as the top.
///
/// A bare dialog has neither the button row nor its separator.
///
/// 🔑 **The buttons stack when a single row cannot hold both labels whole**,
/// one per row, each centred on its own:
///
/// ```text
/// ╭─ ⚠ Title ──────────╮
/// │                    │
/// │                    │
/// │   Body text.       │
/// │                    │
/// │  [ ↵ rebuild ]     │
/// │   esc keep them    │
/// │                    │
/// ╰────────────────────╯
/// ```
///
/// So a dialog's height depends on its width, and [`button_rows`] decides
/// which shape it takes from the drawn widths rather than from a threshold.
///
/// ⚠️ **The button row is centred, and that choice is not from the approved
/// design.** Mike specified how the buttons look, not where they sit. Centred
/// follows TUI convention, and where Herdr puts its own was never measured.
///
/// ⚠️ **The height is not bounded here.** A body longer than the popup scrolls
/// in the pane, which can carry the bottom border off the top. The frame is
/// width-driven only, and the caller sizes the popup with [`HEIGHT`].
///
/// 🔑 **`hot` changes attributes and never geometry.** Every value produces the
/// same characters in the same cells, which is what lets a hover redraw
/// overwrite the previous frame exactly.
pub fn layout(dialog: &Dialog, buttons: Option<&Buttons>, width: usize, hot: Hot) -> Frame {
    let width = width.max(MIN_WIDTH);
    let inner = width - 2;
    let text_width = inner - 2 * SIDE_PADDING;
    let colour = format!("\u{1b}[{}m", dialog.state.colour());

    let mut lines = vec![title_line(dialog, width, &colour)];
    for _ in 0..VERTICAL_PADDING {
        lines.push(blank_line(inner, &colour));
    }
    for line in wrap(&dialog.body, text_width) {
        lines.push(body_line(&line, text_width, &colour));
    }

    let mut primary = None;
    let mut cancel = None;
    if let Some(buttons) = buttons {
        lines.push(blank_line(inner, &colour));
        let (drawn, spans) = button_rows(buttons, text_width, &colour, hot);
        // The row the buttons start on is simply the row the first is pushed
        // to. Each span carries its own offset from there, which is zero for
        // both when they share a row and zero and one when they are stacked.
        let first = lines.len() as u16;
        primary = Some(spans.0.at(first));
        cancel = Some(spans.1.at(first));
        lines.extend(drawn);
    }

    // Two rows below, except under a button row, where one is enough.
    let bottom = match buttons.is_some() {
        true => 1,
        false => VERTICAL_PADDING,
    };
    for _ in 0..bottom {
        lines.push(blank_line(inner, &colour));
    }
    lines.push(bottom_line(width, &colour));

    Frame {
        text: lines.join("\n"),
        primary,
        cancel,
    }
}

/// Draws the dialog with no button highlighted.
///
/// The convenience [`layout`] exists behind. A caller that only displays a
/// dialog wants the characters and nothing else.
pub fn render(dialog: &Dialog, buttons: Option<&Buttons>, width: usize) -> String {
    layout(dialog, buttons, width, Hot::None).text
}

/// A button's extent, before the frame knows which row the buttons start on.
///
/// `row` counts from the first button row rather than from the top of the
/// dialog, because a stacked layout puts the two buttons on different rows and
/// the frame is the only thing that knows where those rows begin.
#[derive(Debug, Clone, Copy)]
struct Span {
    column: u16,
    width: u16,
    row: u16,
}

impl Span {
    fn at(self, first: u16) -> Rect {
        Rect {
            row: first + self.row,
            column: self.column,
            width: self.width,
        }
    }
}

fn title_line(dialog: &Dialog, width: usize, colour: &str) -> String {
    let glyph = dialog.state.glyph();
    // `╭─` + segment + fill + `╮`, so the fill is what is left after three
    // frame characters and the segment itself.
    let room = width - 3;
    // 🚨 **The segment's width is computed, never measured with `cells`.** The
    // glyph is one `char` occupying two cells, so counting the composed string
    // would report one cell too few and push the closing corner out. That is
    // exactly the defect the earlier off-by-one produced, and it would have
    // come back silently the moment the glyph set went wide.
    let glyph_cells = State::GLYPH_CELLS;
    // The budget pays for the segment's own three spaces, the glyph, and **one
    // cell of fill**. Without that last cell a title long enough to use the
    // whole budget pushes the closing corner one cell past the frame.
    let title = truncate(&dialog.title, room.saturating_sub(glyph_cells + 4));
    let (segment, segment_cells) = match title.is_empty() {
        // ` ` + glyph + ` `
        true => (format!(" {} ", glyph), glyph_cells + 2),
        // ` ` + glyph + ` ` + title + ` `
        false => (
            format!(" {} {} ", glyph, title),
            glyph_cells + cells(&title) + 3,
        ),
    };
    let fill = room.saturating_sub(segment_cells).max(1);
    format!("{colour}╭─{segment}{}╮{RESET}", "─".repeat(fill))
}

fn bottom_line(width: usize, colour: &str) -> String {
    format!("{colour}╰{}╯{RESET}", "─".repeat(width - 2))
}

fn blank_line(inner: usize, colour: &str) -> String {
    format!("{colour}│{RESET}{}{colour}│{RESET}", " ".repeat(inner))
}

fn body_line(text: &str, text_width: usize, colour: &str) -> String {
    let pad = " ".repeat(SIDE_PADDING);
    let fill = " ".repeat(text_width.saturating_sub(cells(text)));
    format!("{colour}│{RESET}{pad}{text}{fill}{pad}{colour}│{RESET}")
}

/// The button rows, and where the two buttons landed across them.
///
/// 🔑 **Both buttons share a row only while both fit it whole. Otherwise they
/// stack, one per row.** Decided by Mike on 2026-09-11, after the preview
/// showed what the alternative actually rendered: at the 24-cell floor the
/// labels were being cut to `↵ reb` and `esc kee`, so "rebuild anyway" and
/// "keep them" both became fragments. Truncating rather than pushing the row
/// through the right border was the right instinct, but three characters of a
/// label is not a label.
///
/// Two alternatives were considered and rejected: drawing the keys alone loses
/// the words entirely, and raising [`MIN_WIDTH`] means a narrow pane gets no
/// dialog at all rather than a usable one. Stacking costs one row of height,
/// which is the cheapest thing here to spend.
///
/// ⚠️ **The threshold is measured, not a number.** The kit draws the key
/// affordances itself and the labels are the caller's, so the question is
/// whether these two drawn buttons and the gap between them fit *this* frame —
/// never whether the frame is narrower than some constant.
fn button_rows(
    buttons: &Buttons,
    text_width: usize,
    colour: &str,
    hot: Hot,
) -> (Vec<String>, (Span, Span)) {
    let primary = drawn_button(PRIMARY_KEY, &buttons.primary, true, text_width);
    let cancel = drawn_button(CANCEL_KEY, &buttons.cancel, false, text_width);

    if cells(&primary) + BUTTON_GAP + cells(&cancel) <= text_width {
        let (line, spans) = buttons_row(
            &[
                (primary, true, hot == Hot::Primary),
                (cancel, false, hot == Hot::Cancel),
            ],
            text_width,
            colour,
        );
        return (vec![line], (spans[0], spans[1]));
    }

    // Stacked. The primary goes first, because it names the action the dialog
    // is asking about and reading order should reach it first.
    let (top, above) = buttons_row(&[(primary, true, hot == Hot::Primary)], text_width, colour);
    let (below, under) = buttons_row(&[(cancel, false, hot == Hot::Cancel)], text_width, colour);
    (vec![top, below], (above[0], Span { row: 1, ..under[0] }))
}

/// One row of buttons, centred as a group, and where each one landed on it.
///
/// Takes a slice rather than a pair so that the side-by-side row and each
/// stacked row are the same code. The gap only ever appears *between* items, so
/// a row holding one button is centred on that button alone.
///
/// 🔑 **Inversion rather than an explicit background colour.** Reverse video
/// swaps foreground and background, so setting the foreground to the state's
/// colour and inverting gives that colour as the background with the text
/// inverted against it, which is what Mike asked for. It also keeps the text in
/// the terminal's own background colour, so the pair stays legible in a light
/// theme and a dark one without this module knowing which is in force.
///
/// 🔑 **Hover adds an underline, and nothing else.** A stronger treatment was
/// considered and rejected: inversion already means "this is the default
/// button", so giving a hovered cancel the same treatment would make the two
/// buttons look alike exactly when the user is about to click one. An underline
/// is unambiguous, universally supported, and occupies no cells, so the frame's
/// geometry cannot move when the pointer does.
fn buttons_row(
    items: &[(String, bool, bool)],
    text_width: usize,
    colour: &str,
) -> (String, Vec<Span>) {
    let visible = items.iter().map(|(text, ..)| cells(text)).sum::<usize>()
        + BUTTON_GAP * items.len().saturating_sub(1);
    let left = text_width.saturating_sub(visible) / 2;
    let right = text_width.saturating_sub(left + visible);
    let pad = " ".repeat(SIDE_PADDING);

    let mut middle = String::new();
    let mut spans = Vec::with_capacity(items.len());
    // The border and the side padding sit left of the text column, so a
    // button's pane-local column starts there.
    let mut column = 1 + SIDE_PADDING + left;
    for (index, (text, inverted, hot)) in items.iter().enumerate() {
        if index > 0 {
            middle.push_str(&" ".repeat(BUTTON_GAP));
            column += BUTTON_GAP;
        }
        if *inverted {
            middle.push_str(colour);
            middle.push_str(INVERSE);
        }
        if *hot {
            middle.push_str(UNDERLINE);
        }
        middle.push_str(text);
        middle.push_str(RESET);
        spans.push(Span {
            column: column as u16,
            width: cells(text) as u16,
            row: 0,
        });
        column += cells(text);
    }

    let drawn = format!(
        "{colour}│{RESET}{pad}{}{middle}{}{pad}{colour}│{RESET}",
        " ".repeat(left),
        " ".repeat(right),
    );
    (drawn, spans)
}

/// One button, drawn, with its label shortened only if it alone overflows.
///
/// ⚠️ **Truncation is the last resort rather than the first.** Stacking handles
/// two labels that will not share a row; this handles one label that will not
/// fit a row by itself, which no layout can rescue. Pushing it through the
/// right border instead would break every row's alignment at once.
///
/// The key affordance and the primary's own padding are never shortened,
/// because a button cut to nothing still has to be clickable and still has to
/// say which key answers it.
fn drawn_button(key: &str, label: &str, padded: bool, text_width: usize) -> String {
    let draw = |label: &str| match padded {
        true => format!(" {} ", keyed(key, label)),
        false => keyed(key, label),
    };
    let full = draw(label);
    if cells(&full) <= text_width {
        return full;
    }
    let fixed = cells(&full) - cells(label);
    draw(&truncate(label, text_width.saturating_sub(fixed)))
}

/// Joins a key affordance to its label, or stands alone when there is no label.
///
/// Dropping the separator for an empty label is what keeps [`fit_labels`]
/// honest at the narrowest frames: a button truncated to nothing must not still
/// charge a cell for the space after its key.
fn keyed(key: &str, label: &str) -> String {
    match label.is_empty() {
        true => key.to_string(),
        false => format!("{} {}", key, label),
    }
}

/// How many terminal cells a string occupies.
///
/// One character, one cell. That is **exact for everything this module draws
/// itself**: the four glyphs are single-width BMP characters with no variation
/// selectors, and the frame is box-drawing characters and spaces.
///
/// ⚠️ **Not a general width implementation, and must not be reused as one.**
/// Titles, bodies and button labels are the caller's, and a caller supplying
/// double-width text (CJK, or an emoji) or zero-width text (a combining mark)
/// gets a frame whose right border is out by the difference. Cosmetic rather
/// than a defect, and it is recorded here rather than discovered later.
fn cells(text: &str) -> usize {
    text.chars().count()
}

fn truncate(text: &str, width: usize) -> String {
    match cells(text) <= width {
        true => text.to_string(),
        false => text.chars().take(width).collect(),
    }
}

/// Wraps the body to the frame's inner width.
///
/// Newlines in the body are paragraph breaks and are kept. A word longer than
/// the whole width is split rather than allowed to overflow the border.
fn wrap(body: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for paragraph in body.split('\n') {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let mut word = word.to_string();
            while cells(&word) > width {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                let head: String = word.chars().take(width).collect();
                word = word.chars().skip(width).collect();
                lines.push(head);
            }
            if word.is_empty() {
                continue;
            }
            if line.is_empty() {
                line = word;
            } else if cells(&line) + 1 + cells(&word) <= width {
                line.push(' ');
                line.push_str(&word);
            } else {
                lines.push(std::mem::replace(&mut line, word));
            }
        }
        lines.push(line);
    }
    lines
}

/// The three terminal states a dialog turns on, acquired and released together.
///
/// Raw mode, mouse reporting, and a hidden cursor. 🔑 **One guard for all
/// three, and deliberately not three teardowns.** Three separate releases is
/// how one of them gets missed on the fourth exit route somebody adds later,
/// and a half-restored terminal is worse than any of the three individually.
///
/// A `Drop` guard rather than a call after the loop, because the terminal state
/// is the pane's and outlives this function on every path a plain call would
/// miss: an early return, an error, a panic during unwind. Restoring the cursor
/// matters most of the three, because a terminal left without one stays that
/// way long after the dialog is gone.
///
/// 🔑 **The popup must request mouse reporting itself.** ✅ Measured 2026-09-10
/// on 0.9.0 with a real client in a pty: the client turns reporting on for
/// itself the moment it attaches, but the pane in that measurement had also
/// requested `1000/1002/1003/1006` of its own, and that is the state the
/// finding holds for. `EnableMouseCapture` emits exactly that set plus `1015`.
///
/// Hiding the cursor follows the house pattern rather than inventing one:
/// project-finder's `bin/build` emits `\033[?25l` when its spinner starts and
/// `\033[?25h` when it stops.
///
/// ⚠️ **Two paths are not covered, and neither is coverable here.** `SIGKILL`
/// runs no destructor at all, and an unhandled `SIGTERM` ends the process
/// before `Drop`. Installing handlers was considered and rejected: it needs a
/// signal crate this kit does not otherwise carry, and the dialog runs as its
/// own process in its own plugin pane whose pty dies with it, so nothing
/// survives to be left in a bad state. **That is why this is a small risk, not
/// a reason it was skipped** — every path a destructor can reach is covered.
///
/// ⚠️ Not unit-testable without a pty, so this guard has no test. Its whole job
/// is a side effect on a terminal that a test harness does not have.
struct TerminalState;

impl TerminalState {
    /// Turns all three on in one flushed call, or nothing at all.
    ///
    /// Raw mode first, because it is the one that can fail: without a tty there
    /// is nothing to hide a cursor in, and answering `None` here is what sends
    /// [`interact`] to its line-reading fallback.
    fn enter() -> Option<TerminalState> {
        crossterm::terminal::enable_raw_mode().ok()?;
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::event::EnableMouseCapture,
            crossterm::cursor::Hide,
        );
        Some(TerminalState)
    }
}

impl Drop for TerminalState {
    /// Releases all three, in the reverse of the order they were acquired.
    ///
    /// `execute!` flushes, so the restore reaches the terminal rather than
    /// sitting in a buffer that a hard exit would discard.
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::cursor::Show,
            crossterm::event::DisableMouseCapture,
            crossterm::style::ResetColor,
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

/// Draws the dialog and records the answer. **The popup half's entry point.**
///
/// A consuming plugin calls this from the pane entrypoint named by
/// [`ENTRYPOINT`], which is the same binary Herdr launched into the popup.
///
/// ⚠️ **Writes ANSI escape sequences unconditionally**, because a pane
/// entrypoint always has a real terminal. Do not call it from a `[[startup]]`
/// hook, which is handed no `TERM` at all and would collect the sequences in
/// the server log.
pub fn run(env: &Environment) -> Result<(), String> {
    let popup = Popup::from_env(env);

    // Report that a pane really appeared, before drawing anything. The waiting
    // side cannot learn this from `plugin.pane.open`, which answers `ok` either
    // way, and the process id is what lets a popup the user closes be noticed
    // at once.
    if let Some(started) = &popup.started_file {
        let _ = std::fs::write(started, std::process::id().to_string());
    }

    // A dialog with no buttons is bare, and a dialog with no answer file has
    // nowhere to answer. Either one means nobody is waiting on a choice.
    let actioned = popup.buttons.is_some();
    let answer = interact(&popup.dialog, popup.buttons.as_ref());

    let Some(answer_file) = popup.answer_file.filter(|_| actioned) else {
        return Ok(());
    };
    std::fs::write(&answer_file, answer).map_err(|e| {
        format!(
            "cannot write the answer to {}: {}",
            answer_file.display(),
            e
        )
    })
}

/// The width to draw to, from the pane itself.
fn pane_width() -> usize {
    crossterm::terminal::size()
        .map(|(columns, _)| columns as usize)
        .unwrap_or(FALLBACK_WIDTH)
}

/// Draws, then reads the mouse and the keyboard until one of them answers.
///
/// 🔑 **Mouse first, and keyboard in full.** ✅ Measured 2026-09-10 on 0.9.0: a
/// click inside a plugin pane is forwarded to that pane's own pty, SGR-encoded
/// and rebased to pane-local coordinates, while a click outside it is not
/// forwarded at all. A control in the same run confirmed keystrokes kept
/// arriving throughout, so the two coexist rather than trading off.
///
/// ⚠️ **That measurement was taken on a plugin pane, and this module draws in a
/// popup specifically.** Nobody has confirmed the placement from the source
/// text, so treat click forwarding into a popup as very likely rather than
/// settled. Every answer is therefore reachable from the keyboard alone, and a
/// dialog whose clicks never arrive is fully usable rather than stuck.
fn interact(dialog: &Dialog, buttons: Option<&Buttons>) -> &'static str {
    use crossterm::event::{read, Event, KeyEventKind, MouseButton, MouseEventKind};

    let Some(_terminal) = TerminalState::enter() else {
        // No tty, so no raw mode and no mouse. Draw once and read a line.
        println!("{}", render(dialog, buttons, FALLBACK_WIDTH));
        return match buttons {
            Some(_) => read_line(),
            None => dismiss_on_a_line(),
        };
    };

    let mut width = pane_width();
    let mut hot = Hot::None;
    let mut frame = draw(dialog, buttons, width, hot);

    loop {
        match read() {
            Ok(Event::Key(key)) => {
                // A press and its release both arrive on some terminals, and
                // acting on both would read one keystroke as two.
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match buttons {
                    // A bare dialog dismisses on any key.
                    None => return CANCEL_WORD,
                    Some(_) => {
                        if let Some(answer) = key_answer(key.code, key.modifiers) {
                            return answer;
                        }
                    }
                }
            }
            Ok(Event::Mouse(mouse)) => match mouse.kind {
                MouseEventKind::Down(button) => {
                    // ⚠️ Only the left button activates. A right-click is
                    // forwarded identically, as `<2;`, and treating it as an
                    // activation would put a destructive primary button behind
                    // a menu gesture that means nothing in a dialog.
                    match buttons {
                        // Nothing to aim at, so any button dismisses.
                        None => return CANCEL_WORD,
                        Some(_) if button != MouseButton::Left => continue,
                        Some(_) => match frame.hit(mouse.column, mouse.row) {
                            Hot::Primary => return PRIMARY_WORD,
                            Hot::Cancel => return CANCEL_WORD,
                            // A click on the body is not an answer. Resolving
                            // it would let a misclick fire a primary button
                            // whose action may be destructive.
                            Hot::None => continue,
                        },
                    }
                }
                MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                    let now = frame.hit(mouse.column, mouse.row);
                    if now != hot {
                        hot = now;
                        frame = draw(dialog, buttons, width, hot);
                    }
                }
                // Releases and scrolling are not answers. Mouse capture reports
                // motion as well as clicks, so acting on either would resolve
                // the dialog before it had been read.
                _ => continue,
            },
            Ok(Event::Resize(columns, _)) => {
                width = columns as usize;
                // The pointer's old row may not hold a button any more.
                hot = Hot::None;
                frame = draw(dialog, buttons, width, hot);
            }
            Ok(_) => continue,
            // ⚠️ Fails closed. A terminal that stopped answering cannot be read
            // as the user choosing the primary button.
            Err(_) => return CANCEL_WORD,
        }
    }
}

/// Paints the frame from the pane's origin, and hands back its geometry.
///
/// 🔑 **Always from [`HOME`].** Every redraw replaces the previous frame cell
/// for cell, because `hot` changes attributes and never geometry. That is what
/// makes a hover repaint safe rather than something that can accumulate.
///
/// `\r\n` rather than `\n`: raw mode does no carriage return of its own, so a
/// bare newline would staircase the frame across the pane.
fn draw(dialog: &Dialog, buttons: Option<&Buttons>, width: usize, hot: Hot) -> Frame {
    use std::io::Write;
    let frame = layout(dialog, buttons, width, hot);
    print!("{}{}", HOME, frame.text.replace('\n', "\r\n"));
    let _ = std::io::stdout().flush();
    frame
}

/// What one key means in an actioned dialog, or `None` for "keep waiting".
///
/// 🔑 **Enter chooses the primary button**, which is what makes the inverted
/// button a default rather than decoration. ✅ That was the open question
/// blocking this design, and it is settled: measured 2026-09-11 by logging raw
/// stdin bytes in every pane process and injecting keystrokes, a popup takes
/// keyboard input **exclusively** and the base pane received nothing.
///
/// ⚠️ **The default is the primary button whatever the state**, including
/// [`State::Danger`]. Confirmed by Mike, and it matches Herdr's own
/// delete-worktree dialog. So a caller putting a destructive action on the
/// primary button is putting it one Enter away, and should choose which action
/// is primary with that in mind.
///
/// **An unlisted key is ignored and the dialog stays open.** That is not the
/// same as cancelling, and the difference matters: an unbound key must not
/// resolve the question in either direction.
fn key_answer(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> Option<&'static str> {
    use crossterm::event::{KeyCode, KeyModifiers};
    match code {
        KeyCode::Enter => Some(PRIMARY_WORD),
        KeyCode::Esc => Some(CANCEL_WORD),
        // Ctrl-C in raw mode is a key event rather than a signal, and somebody
        // pressing it means to get out.
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(CANCEL_WORD),
        _ => None,
    }
}

/// One line, where there is no terminal to read a keypress or a click from.
///
/// Raw mode needs a tty. The popup always has one, being a real pane. A test
/// harness piping stdin does not, and neither would a stray invocation from a
/// script. Falling back to a line keeps both choices answerable either way, and
/// means the fallback is exercised rather than being untested code that only
/// runs once something has already gone wrong.
fn read_line() -> &'static str {
    let mut typed = String::new();
    match std::io::stdin().read_line(&mut typed) {
        // End of input is not an answer.
        Ok(0) | Err(_) => CANCEL_WORD,
        // ⚠️ Only an empty line means Enter, matching the raw-mode binding. Any
        // typed word is not one of the two choices and must not resolve the
        // question toward the primary button.
        Ok(_) => match typed.trim().is_empty() {
            true => PRIMARY_WORD,
            false => CANCEL_WORD,
        },
    }
}

/// A bare dialog with no tty. Anything at all dismisses it, including nothing.
fn dismiss_on_a_line() -> &'static str {
    let mut typed = String::new();
    let _ = std::io::stdin().read_line(&mut typed);
    CANCEL_WORD
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The endings that are slow to provoke are provoked in milliseconds here.
    const QUICK: Duration = Duration::from_millis(250);

    #[test]
    fn the_two_words_are_distinct() {
        // They travel through a file between two processes, so a collision
        // would make one choice unreachable.
        assert_ne!(PRIMARY_WORD, CANCEL_WORD);
    }

    #[test]
    fn only_the_exact_word_chooses_the_primary_button() {
        for ended in [
            Ended::Answered(CANCEL_WORD.to_string()),
            Ended::Answered(String::new()),
            Ended::Answered("PRIMARY".to_string()),
            Ended::Answered("prim".to_string()),
            Ended::Answered("primary ok".to_string()),
            Ended::Answered("yes".to_string()),
            Ended::Answered("close".to_string()),
            Ended::Dismissed,
            Ended::NeverShown,
            Ended::TimedOut,
        ] {
            assert!(
                !decide(ended.clone()).chose_primary(),
                "{:?} chose the primary button",
                ended
            );
        }
        assert!(decide(Ended::Answered(PRIMARY_WORD.to_string())).chose_primary());
    }

    #[test]
    fn every_ending_maps_to_its_own_outcome() {
        assert_eq!(
            decide(Ended::Answered(PRIMARY_WORD.to_string())),
            Answer::Primary
        );
        assert_eq!(
            decide(Ended::Answered(CANCEL_WORD.to_string())),
            Answer::Cancel
        );
        assert_eq!(
            decide(Ended::Answered("keep".to_string())),
            Answer::Unanswered(Unanswered::Unrecognised("keep".to_string()))
        );
        assert_eq!(
            decide(Ended::Dismissed),
            Answer::Unanswered(Unanswered::Dismissed)
        );
        assert_eq!(
            decide(Ended::NeverShown),
            Answer::Unanswered(Unanswered::NeverShown)
        );
        assert_eq!(
            decide(Ended::TimedOut),
            Answer::Unanswered(Unanswered::TimedOut)
        );
    }

    #[test]
    fn an_unrecognised_word_is_kept_rather_than_collapsed_into_a_dismissal() {
        // A popup left over from an older build and a popup the user closed are
        // different problems, and the caller reports them differently.
        assert_ne!(
            decide(Ended::Answered("keep".to_string())),
            decide(Ended::Dismissed)
        );
    }

    #[test]
    fn a_channel_with_no_popup_at_all_never_started() {
        let channel = Channel::new().unwrap();
        assert_eq!(channel.watch_within(QUICK, QUICK), Ended::NeverShown);
    }

    #[test]
    fn an_answer_inside_the_startup_window_is_read_without_a_marker() {
        // The ordinary case for a fast answer: the popup ran and finished
        // before it was ever looked for.
        let channel = Channel::new().unwrap();
        std::fs::write(&channel.answer, PRIMARY_WORD).unwrap();
        assert_eq!(
            channel.watch_within(QUICK, QUICK),
            Ended::Answered(PRIMARY_WORD.to_string())
        );
    }

    #[test]
    fn a_started_popup_that_never_answers_times_out() {
        let channel = Channel::new().unwrap();
        // This process is alive, so the liveness check cannot end the wait and
        // only the deadline can.
        std::fs::write(&channel.started, std::process::id().to_string()).unwrap();
        assert_eq!(channel.watch_within(QUICK, QUICK), Ended::TimedOut);
    }

    #[test]
    fn a_started_popup_that_answers_is_read() {
        let channel = Channel::new().unwrap();
        std::fs::write(&channel.started, std::process::id().to_string()).unwrap();
        let answer = channel.answer.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            std::fs::write(&answer, CANCEL_WORD).unwrap();
        });
        assert_eq!(
            channel.watch_within(QUICK, Duration::from_secs(5)),
            Ended::Answered(CANCEL_WORD.to_string())
        );
        writer.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_popup_process_that_died_without_answering_was_dismissed() {
        let channel = Channel::new().unwrap();
        std::fs::write(&channel.started, reaped_pid().to_string()).unwrap();
        assert_eq!(
            channel.watch_within(QUICK, Duration::from_secs(5)),
            Ended::Dismissed
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_live_process_is_alive_and_a_reaped_one_is_not() {
        // The discriminating pair. Without the negative case the liveness check
        // could answer a constant `true` and every test above would still pass.
        assert!(process_is_alive(&std::process::id().to_string()));
        assert!(!process_is_alive(&reaped_pid().to_string()));
    }

    /// The id of a process that has certainly exited and been reaped.
    ///
    /// `/bin/sh -c 'exit 0'` rather than `true`, which lives at `/bin/true` on
    /// Linux and `/usr/bin/true` on macOS. A shell is at `/bin/sh` on both.
    #[cfg(unix)]
    fn reaped_pid() -> u32 {
        let mut dead = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .unwrap();
        let pid = dead.id();
        dead.wait().unwrap();
        pid
    }

    #[test]
    fn an_empty_answer_file_is_not_an_answer() {
        // A file created but not yet written is the state the popup passes
        // through, and reading it as an answer would resolve the question
        // before the user touched a key.
        let channel = Channel::new().unwrap();
        std::fs::write(&channel.answer, "   \n").unwrap();
        assert_eq!(channel.watch_within(QUICK, QUICK), Ended::NeverShown);
    }

    #[test]
    fn a_channel_removes_its_own_directory() {
        let dir = {
            let channel = Channel::new().unwrap();
            assert!(channel.dir.is_dir());
            channel.dir.clone()
        };
        assert!(!dir.exists(), "the channel directory outlived the channel");
    }

    #[test]
    fn two_channels_never_share_a_path() {
        // ✅ SCOPE.md §7.4: keying on the pid alone caused a real race, where a
        // lingering first popup deleted the second question's answer file.
        let first = Channel::new().unwrap();
        let second = Channel::new().unwrap();
        assert_ne!(first.dir, second.dir);
        assert_ne!(first.answer, second.answer);
        assert_ne!(first.started, second.started);
    }

    #[test]
    fn the_requested_size_is_a_shape_the_schema_accepts() {
        // The guard on the two constants. An edit to either that the schema's
        // pattern rejects would otherwise unsize every dialog in silence.
        assert!(popup_size(WIDTH).is_some(), "{} is not a popup size", WIDTH);
        assert!(
            popup_size(HEIGHT).is_some(),
            "{} is not a popup size",
            HEIGHT
        );
        // The pattern is `^(100|[1-9][0-9]?)%$`, so these cannot be represented.
        assert!(popup_size("nonsense").is_none());
        assert!(popup_size("120%").is_none());
        assert!(popup_size("60").is_none());
        assert!(popup_size("0%").is_none());
    }

    #[test]
    fn a_cell_is_a_character() {
        assert_eq!(cells("abc"), 3);
        assert_eq!(cells(""), 0);
        assert_eq!(cells(PRIMARY_KEY), 1);
        assert_eq!(cells(CANCEL_KEY), 3);
    }

    /// The whole width rule, stated rather than enumerated.
    ///
    /// 🔑 **Four glyph sets were tried in one afternoon, and each failed a width
    /// property nobody had checked before choosing it.** A test that asserts the
    /// property means the next person finds out in seconds rather than three
    /// rounds later. That is why this checks the rule against the real Unicode
    /// data instead of pinning four literals, which would only re-state the
    /// choice rather than test it.
    ///
    /// `unicode-width` is a dev-dependency, so no consumer of this crate ever
    /// carries it.
    #[test]
    fn every_glyph_is_one_narrow_unambiguous_codepoint() {
        use unicode_width::UnicodeWidthChar;

        for state in [State::Info, State::Success, State::Warning, State::Danger] {
            let glyph = state.glyph();
            assert_eq!(
                glyph.chars().count(),
                1,
                "{:?}'s glyph is more than one codepoint",
                state
            );
            let point = glyph.chars().next().unwrap();

            // 🚨 A variation selector is the second codepoint the rule above
            // forbids, and the specific trigger for Terminal.app drawing a
            // Neutral character two cells wide while advancing the cursor one.
            assert!(
                !(0xfe00..=0xfe0f).contains(&(point as u32)),
                "{:?}'s glyph carries a variation selector",
                state
            );

            // Not Wide: an emoji costs two cells and misaligns one title
            // against the other three.
            assert_eq!(
                point.width(),
                Some(State::GLYPH_CELLS),
                "{:?}'s glyph is not {} cell(s) wide",
                state,
                State::GLYPH_CELLS
            );

            // 🚨 Not Ambiguous, which is worse than either Neutral or Wide
            // because the terminal decides the width from a setting rather than
            // the character deciding it. `width` treats Ambiguous as narrow and
            // `width_cjk` treats it as wide, so disagreement between the two is
            // an exact test for it.
            assert_eq!(
                point.width(),
                point.width_cjk(),
                "{:?}'s glyph is East Asian Width Ambiguous, so a terminal \
                 setting decides how wide it is",
                state
            );
        }
    }

    #[test]
    fn the_characters_the_glyph_rule_rejects_really_would_fail_it() {
        // The rule is only worth having if it discriminates. These are the
        // three rejected candidates, one per trap.
        use unicode_width::UnicodeWidthChar;

        // U+26D4 NO ENTRY is Wide, so it would have been the only two-cell
        // glyph in a set of four.
        assert_eq!('\u{26d4}'.width(), Some(2));

        // U+24D8, U+2299 and U+25B2 are Ambiguous.
        for point in ['\u{24d8}', '\u{2299}', '\u{25b2}'] {
            assert_ne!(
                point.width(),
                point.width_cjk(),
                "U+{:04X} was expected to be Ambiguous",
                point as u32
            );
        }

        // And a selector makes a one-codepoint glyph two.
        assert_eq!("\u{26a0}\u{fe0f}".chars().count(), 2);
    }

    #[test]
    fn a_word_longer_than_the_width_is_split_rather_than_overflowing() {
        assert_eq!(wrap("aaaaaaaa", 3), vec!["aaa", "aaa", "aa"]);
        assert_eq!(wrap("hi aaaaaa", 3), vec!["hi", "aaa", "aaa"]);
    }

    #[test]
    fn newlines_in_the_body_are_kept_as_paragraph_breaks() {
        assert_eq!(wrap("one\ntwo", 10), vec!["one", "two"]);
        assert_eq!(wrap("one\n\ntwo", 10), vec!["one", "", "two"]);
    }

    #[test]
    fn words_are_packed_up_to_the_width_and_not_past_it() {
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
        assert_eq!(wrap("aa bb cc", 8), vec!["aa bb cc"]);
    }

    #[test]
    fn enter_chooses_the_primary_button_and_escape_cancels() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;

        assert_eq!(key_answer(KeyCode::Enter, NONE), Some(PRIMARY_WORD));
        assert_eq!(key_answer(KeyCode::Esc, NONE), Some(CANCEL_WORD));
        assert_eq!(
            key_answer(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(CANCEL_WORD)
        );
    }

    #[test]
    fn an_unbound_key_is_ignored_rather_than_answering_either_way() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;

        for code in [
            KeyCode::Char(' '),
            KeyCode::Char('y'),
            KeyCode::Char('n'),
            KeyCode::Char('q'),
            KeyCode::Backspace,
            KeyCode::Tab,
            KeyCode::Up,
            KeyCode::F(1),
        ] {
            assert_eq!(key_answer(code, NONE), None, "{:?} answered", code);
        }
        // A bare `c` is not Ctrl-C, and reading it as one would put the
        // modifier check there for nothing.
        assert_eq!(key_answer(KeyCode::Char('c'), NONE), None);
    }
}
