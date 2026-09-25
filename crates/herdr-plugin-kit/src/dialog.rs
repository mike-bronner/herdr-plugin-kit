//! Styled dialogs in four states, drawn in a Herdr popup.
//!
//! Two variants. [`notify`] informs and returns at once. [`ask`] puts a
//! question with any number of buttons and waits for which one the user chose.
//! See SCOPE.md §7.5.
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
//! a popup the user closed be noticed at once rather than waited out. ➕ From
//! 0.5.3 the popup also holds the marker locked while it runs, which is what
//! lets [`ask`] end a popup it gave up on without trusting a bare pid.
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
//!   `TerminalState` does the same rather than trusting the client's setting
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
//! 🔑 **That sentence survives any number of buttons, and holding it is why
//! [`Button::keys`] is shaped the way it is.** A caller names the key on each
//! button, and every button still answers **exactly one** key, so no answer can
//! end up reachable by click alone. A dialog that needed a click was rejected by
//! Mike on 2026-09-14, on exactly this measurement: it would rest an answer on
//! click forwarding nobody has confirmed. SCOPE.md §7.5.8 records it.
//!
//! ⚠️ **Ctrl-C is the exception, and it is the last way out**: it always ends the
//! dialog, no naming can take it away, and it is the only key that answers
//! without being drawn. 🚨 **It chooses no button.** With several buttons every
//! candidate target is a real action the user did not pick, so it resolves the
//! dialog **unanswered** rather than inventing a choice on their behalf.
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
//! So sending it would have meant no dialog at all. `open_params` leaves it
//! unset, and now does so on a measurement rather than on caution.
//!
//! Confinement to the triggering *pane* was explicitly not chosen, and is
//! contradicted by measurement anyway: a popup floats centred over the whole
//! tab and was captured across a pane divider, and there is no position
//! parameter at all.
//!
//! # ✅ Colour is reviewed and approved, and still unmatched
//!
//! ✅ **Approved by Mike on 2026-09-11**, from the `preview` example rendered
//! in Terminal.app under his own theme: the four colours read correctly and
//! tell the four states apart. A palette is a design question, and its author
//! looking at it is the right authority for one — which is precisely what the
//! original caveat was missing.
//!
//! ⚠️ **Approved is not matched, and the unexamined half survives the
//! approval.** Nobody has put the kit's blue beside Herdr's own blue in one
//! frame. The probe captured **geometry and characters, not colour** — the
//! capture library discards SGR attributes, so borders and titles were
//! confirmed present and never confirmed coloured — and **nobody has
//! established how Herdr colours its own dialogs.** That was true before the
//! review and it is true after it.
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
//! three-state teardown in `TerminalState`. Every character of the frame is
//! this module's own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::api::generated::{
    ClientWindowTitleReason, NotificationShowParams, NotificationShowReason, PluginPaneOpenParams,
    PluginPanePlacement, PopupSize, PopupSizeString,
};
use crate::env::Environment;

/// The seam this module sends through, shared with `report`.
///
/// 🔑 **Re-exported rather than referenced, so `dialog::Transport` still
/// resolves.** The trait was declared here first and is documented from here in
/// SCOPE.md §7.5.6. It moved to [`crate::surface`] when `report` turned out to
/// need the same two calls for the same stated reason: `show_notification`'s
/// contract is §7.2's, which is `report`'s own section. A consumer that already
/// writes `impl dialog::Transport for MyClient` is unaffected.
pub use crate::surface::{OpenError, Transport, BUSY_CODE};

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

/// The prefix of the variable carrying one button. See [`button_var`].
pub const BUTTON_VAR: &str = "HERDR_PLUGIN_DIALOG_BUTTON_";

/// Where the popup writes which button the user chose.
pub const ANSWER_FILE_VAR: &str = "HERDR_PLUGIN_DIALOG_ANSWER_FILE";

/// Where the popup writes its own process id, before drawing anything.
pub const STARTED_FILE_VAR: &str = "HERDR_PLUGIN_DIALOG_STARTED_FILE";

/// The variable carrying the button at `index`, counting from zero.
///
/// 🔑 **One variable per button, and the list ends at the first absent one.**
/// A single variable holding every button would need a separator, and a label is
/// the caller's string: any separator a label can contain turns one button into
/// two, which changes the set of answers rather than merely the drawing.
///
/// 🔑 **Each one carries exactly what its button draws**: the resolved key, a
/// space, then the label. The key vocabulary is [`Key::drawn`]'s
/// — the Enter glyph, the Escape word, or one ASCII graphic character — and none
/// of those contains a space, so the first space is an unambiguous split and
/// every label survives verbatim, spaces and all.
///
/// ⚠️ **`BUTTON_0`'s absence is what makes a dialog bare**, which is the rule the
/// primary label's absence used to carry.
pub fn button_var(index: usize) -> String {
    format!("{}{}", BUTTON_VAR, index)
}

/// The word the popup writes when nobody chose a button.
///
/// 🚨 **Ctrl-C writes this, and so does every other way of ending without a
/// choice.** It is not a button and it can never become one: the waiting half
/// reads it as [`Unanswered::Dismissed`], which is what the popup dying without
/// answering already meant. Writing it rather than exiting silently is what
/// makes the outcome immediate on every platform, including Windows, where the
/// liveness check cannot prove a death.
///
/// It is not a number, so no button index can collide with it.
pub const DISMISSED_WORD: &str = "dismissed";

/// The glyph for Enter, drawn by the kit and never by the caller.
///
/// 🔑 **The kit draws this, so three plugins cannot disagree about it.** A
/// caller writing its own `↵` into a label is how the symbol, the spacing, or
/// whether to bother at all drift apart across repositories, which is the exact
/// class of divergence this crate exists to end. ➕ **That is why a caller *names*
/// the key on each button and never draws one** ([`Key`]): the choice is the
/// consumer's, and the character on the frame stays the kit's.
///
/// It matters more now the dialogs are mouse first: a button somebody clicks
/// still has to advertise the key for somebody who will not.
///
/// 🚨 **It is drawn on whichever button Enter answers, and nowhere else.** The
/// frame never draws a key that answers nothing, which is [`Button::keys`]'s rule
/// and SCOPE.md §7.5.8's record.
///
/// U+21B5 rather than U+23CE, and neither has an emoji presentation, so no
/// text-presentation selector is needed on either.
pub const ENTER_KEY: &str = "\u{21b5}";

/// The key affordance for Escape. See [`ENTER_KEY`].
///
/// A word rather than a glyph, because no single character means Escape and an
/// invented one would have to be learned.
///
/// 🔑 **Drawn on whichever button Escape answers.** ⚠️ **Never the first**, which
/// is [`Button::keys`]'s one safety rule: the way out must not fire the action
/// the dialog leads with. A list where no button answers Escape leaves it undrawn
/// and answering nothing, and **Ctrl-C is then the only undrawn way out**.
pub const ESCAPE_KEY: &str = "esc";

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

/// Blank columns between two buttons sharing a row.
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
    /// ✅ **All four reviewed and approved by Mike on 2026-09-11**, from the
    /// `preview` example rendered in Terminal.app under his own theme. The
    /// judgement was that they read correctly and tell the four states apart,
    /// which is the question a palette actually has to answer.
    ///
    /// ⚠️ **Approved is not matched.** Nobody has put these beside Herdr's own
    /// dialog colours in one frame, and nobody has established how Herdr
    /// colours its own. See the module documentation.
    ///
    /// These are the eight basic colours and their bright variants rather than
    /// 256-colour or truecolour values, so a themed terminal maps each one to
    /// the palette the user already chose.
    ///
    /// ✅ **Info is the bright variant and the other three are not, and that
    /// asymmetry was reviewed with the rest and stands.** Mike asked for light
    /// blue on info alone; 94 is the bright form of the same basic blue, so it
    /// stays inside the eight-plus-eight set and keeps the property the whole
    /// palette was chosen for. Bright info against three normal siblings
    /// looked correct to the person who asked for it.
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

/// A key a caller can put on a button, and the kit draws.
///
/// 🔑 **The caller chooses which key, and the kit still draws it.** Only the
/// caller knows what its buttons do; only the kit can keep three plugins drawing
/// one symbol at one spacing ([`ENTER_KEY`]). So a caller **names** a key and
/// never types a glyph into a label, which is the boundary this type exists to
/// keep.
///
/// 🔑 **Keys do not move, and that is the whole rule.** A key answers the button
/// it is named on, and nothing else. There is no displacement to reason about:
/// [`Key::Enter`] answers whichever button names it, and answers nothing at all
/// where no button does.
///
/// | Button 0 | Button 1 | 0 draws | 1 draws | Enter | Escape |
/// |---|---|---|---|---|---|
/// | unnamed | unnamed | `↵` | `esc` | button 0 | button 1 |
/// | `Char('y')` | unnamed | `y` | `esc` | nothing | button 1 |
/// | unnamed | `Char('n')` | `↵` | `n` | button 0 | nothing |
/// | `Char('y')` | `Char('n')` | `y` | `n` | nothing | nothing |
/// | `Char('y')` | `Enter` | `y` | `↵` | button 1 | nothing |
///
/// 🔑 **Every key the frame draws answers the button it is drawn on, and every
/// key that answers is drawn.** Ctrl-C is the single exception and is never
/// configurable: it is the last way out of a raw-mode dialog, a popup carries no
/// pane id to close, and a caller must not be able to take the exit away. It
/// **understates** the frame rather than contradicting it, and it is what makes
/// naming a key over Escape safe. See [`Button::keys`], which is where a naming
/// that cannot be honoured is resolved.
///
/// ⚠️ **Naming replaces rather than adds.** A button that names a character is a
/// button Enter no longer reaches. That is Mike's instruction of 2026-09-14, and
/// SCOPE.md §7.5.8 records the cost beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// The Enter key, drawn as [`ENTER_KEY`].
    ///
    /// ⚠️ **It answers whatever the state is**, [`State::Danger`] included.
    /// Confirmed by Mike, and it matches Herdr's own delete-worktree dialog. So a
    /// caller putting a destructive action on the button Enter answers is putting
    /// it one Enter away, and either orders the list with that in mind or names
    /// another key.
    ///
    /// 🔑 **Naming it on a later button is how the safe answer goes under the key
    /// people press to dismiss what they have not read.** [`Button::keys`] then
    /// gives the first button something else, because one key cannot answer two
    /// buttons.
    Enter,
    /// The Escape key, drawn as [`ESCAPE_KEY`].
    ///
    /// ⚠️ **Never answers the first button.** A dialog whose way out fires the
    /// action it leads with is not a dialog, so naming it there is resolved to
    /// another key. It is the same reasoning that keeps Ctrl-C unconfigurable.
    Escape,
    /// This character, drawn as itself, matched case-insensitively so `'y'`
    /// accepts `Y`.
    ///
    /// 🚨 **It must be an ASCII graphic character, and one predicate carries
    /// three rules.** Such a character is **exactly one cell wide**, which is
    /// what `cells` assumes when it counts characters rather than measuring
    /// width; none of them is East Asian Width `Ambiguous`, so no terminal
    /// *setting* can decide the width and break the frame, which is §7.5.4's
    /// worst trap; and none of them is whitespace, so an affordance cannot
    /// advertise an invisible key. Measuring real width instead would cost a
    /// runtime dependency §7.5.7 argues against.
    ///
    /// ⚠️ **Anything else takes the first free key instead** rather than leaving
    /// a button no key answers. It is a caller's mistake, it is visible the first
    /// time the frame is drawn, and [`Button::keys`] reports what it became.
    Char(char),
}

impl Key {
    /// What the frame draws for this key, and what the wire carries.
    ///
    /// 🔑 **One string doing both jobs, so the two cannot drift.**
    /// [`button_var`] carries exactly this, and [`Key::from_wire`] reads it back,
    /// so a key the popup half draws is a key the asking half chose.
    pub fn drawn(self) -> String {
        match self {
            Key::Enter => ENTER_KEY.to_string(),
            Key::Escape => ESCAPE_KEY.to_string(),
            Key::Char(key) => key.to_string(),
        }
    }

    /// Reads a key back, or `None` for a value that names no key at all.
    ///
    /// Absent, empty, and anything outside the vocabulary all answer `None`,
    /// which is exactly what a button naming no key at all says, so an unreadable
    /// value takes the first free key like any other unnamed button. ⚠️ **One rule
    /// for every unusable value, pointing the same way at every button** — a rule
    /// that read one garbled key differently from another would be two rules
    /// wearing one name.
    ///
    /// 🔑 The vocabulary is [`Key::drawn`]'s, so every value this crate writes
    /// round-trips: the Enter glyph, the Escape word, or one character.
    pub fn from_wire(word: Option<&str>) -> Option<Key> {
        let word = word.map(str::trim).filter(|word| !word.is_empty())?;
        match word {
            ENTER_KEY => Some(Key::Enter),
            ESCAPE_KEY => Some(Key::Escape),
            _ => {
                let mut characters = word.chars();
                match (characters.next(), characters.next()) {
                    (Some(key), None) => Key::named(key),
                    _ => None,
                }
            }
        }
    }

    /// The key a character makes, or `None` when a character cannot be one.
    ///
    /// 🔑 **The single place the character rule lives**, so a codepoint that
    /// cannot be drawn as one cell cannot be a key on one path and a fallback on
    /// another. [`Key::Char`] carries the three rules this predicate holds.
    fn named(key: char) -> Option<Key> {
        match key.is_ascii_graphic() {
            true => Some(Key::Char(key)),
            false => None,
        }
    }

    /// Whether this keypress is this key.
    ///
    /// ⚠️ **Shift is the only modifier a character tolerates**, because it is how
    /// an uppercase character arrives at all. `Alt-y` is a different chord. Enter
    /// and Escape are matched whatever the modifiers, which is what they did
    /// before any of this was configurable.
    fn answers(
        self,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> bool {
        use crossterm::event::{KeyCode, KeyModifiers};
        match (self, code) {
            (Key::Enter, KeyCode::Enter) => true,
            (Key::Escape, KeyCode::Esc) => true,
            (Key::Char(key), KeyCode::Char(typed)) => {
                modifiers.difference(KeyModifiers::SHIFT).is_empty()
                    && typed.eq_ignore_ascii_case(&key)
            }
            _ => false,
        }
    }

    /// Whether these two settings are the same keypress.
    ///
    /// Case-insensitive for characters, for the reason [`Key::answers`] is: `'y'`
    /// and `'Y'` are one key, so naming one on each of two buttons is a collision
    /// rather than two keys.
    fn same_as(self, other: Key) -> bool {
        match (self, other) {
            (Key::Enter, Key::Enter) | (Key::Escape, Key::Escape) => true,
            (Key::Char(one), Key::Char(other)) => one.eq_ignore_ascii_case(&other),
            _ => false,
        }
    }
}

/// One button an actioned dialog offers.
///
/// **The label is the caller's. The key is the caller's. Drawing both is the
/// kit's.** A button is drawn as its key, a space, then the label, so
/// `Button::new("close anyway")` draws `↵ close anyway` without the caller typing
/// the glyph. See [`ENTER_KEY`] for why that boundary sits there, and [`Key`] for
/// what a caller may put on a button.
///
/// The kit also fixes how a list is drawn: **the first button is inverted in the
/// state's colour and every other one is plain text.** Emphasis is positional, so
/// a caller chooses it by choosing the order — the same lever it already uses for
/// reading order.
///
/// ➕ **The keys ride on the buttons rather than on [`ask`]**, because they are a
/// property of the answers, and because the list is the one value that already
/// crosses to the popup half — [`ask`] sends it and [`Popup::from_env`] reads it
/// back, so both halves cannot disagree about which key answers what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    /// What the button says. Drawn after its key, and never truncated while the
    /// row can hold it whole.
    pub label: String,
    /// The key this button asks for, or `None` to take whatever is free.
    ///
    /// ⚠️ Read through [`Button::keys`] rather than directly, which is where a
    /// naming that cannot be honoured is resolved.
    pub key: Option<Key>,
}

impl Button {
    /// Builds a button that takes whatever key is free. See [`Button::keys`].
    pub fn new(label: &str) -> Button {
        Button {
            label: label.to_string(),
            key: None,
        }
    }

    /// Names the key that answers this button, **and leaves that key answering
    /// nothing anywhere else**.
    ///
    /// `Button::new("close anyway").on_key(Key::Char('y'))` draws
    /// `y close anyway`. Naming [`Key::Enter`] on a later button is how the safe
    /// answer goes under the key people press to dismiss what they have not read.
    ///
    /// ⚠️ **Ctrl-C still ends the dialog**, and it is the only way out that
    /// survives every naming. That is deliberate rather than an oversight, and it
    /// chooses no button: see [`Key`].
    pub fn on_key(self, key: Key) -> Button {
        Button {
            key: Some(key),
            ..self
        }
    }

    /// The key each button actually answers, in the buttons' own order.
    ///
    /// 🔑 **The single place a naming is interpreted.** Every reader — both input
    /// paths, the drawing, and the wire — asks this one question, so a naming that
    /// cannot be honoured cannot be honoured on one path and dropped on another.
    /// It is idempotent, and both halves of the binary run it.
    ///
    /// 🔑 **One rule, walked in order, with no special case per button.** Each
    /// button takes the key it named, or **the first key still free** when it
    /// cannot — and free runs [`Key::Escape`], then [`Key::Enter`], then the ASCII
    /// graphic characters in codepoint order, skipping every key an earlier button
    /// took and skipping Escape on the first button.
    ///
    /// A naming cannot be honoured in three cases, and all three simply fall
    /// through to that ladder rather than leaving a button no key answers:
    ///
    /// - [`Key::Escape`] on the **first** button, because the way out must not
    ///   fire the action the dialog leads with.
    /// - A [`Key::Char`] outside the character rule, because the frame could not
    ///   draw it at a width it can measure.
    /// - A key an **earlier** button already took, case included, because one
    ///   keypress cannot answer two buttons and the frame would have to lie about
    ///   one of them.
    ///
    /// ✅ **Escape first in the ladder is what reproduces the pair this module
    /// shipped with.** Two buttons naming nothing resolve to Enter then Escape,
    /// and a first button naming a character leaves Escape on the second, which is
    /// what a caller that names nothing has always drawn and answered.
    ///
    /// 🚨 **A button no key answers is the one thing that stays unrepresentable.**
    /// The mouse-only dialog Mike rejected on 2026-09-14 cannot be built, and the
    /// ladder can never produce one while the list is no longer than the keys the
    /// ladder can tell apart.
    ///
    /// ⚠️ **A list longer than that loses its tail, and the limit is 70, not
    /// 96.** The ladder yields 96 keys, but a letter and its capital are one
    /// keypress, so 26 of them collide with a key already held and at most 70
    /// buttons are keyed. Buttons past the 70th get none — this answers shorter
    /// than the list it was given, and everything that draws, sends or hit-tests
    /// a button walks *this* answer. A button with no key is drawn nowhere and
    /// answers nothing rather than existing unreachably.
    ///
    /// ⚠️ **Resolved rather than refused**, and that is a judgement recorded in
    /// SCOPE.md §7.5.8: each case is a cross-field or value-level condition, and
    /// refusing them would need fallible builders or private fields, neither of
    /// which this module uses. This is how a caller checks what its naming became.
    pub fn keys(buttons: &[Button]) -> Vec<Key> {
        let mut taken: Vec<Key> = Vec::with_capacity(buttons.len());
        for (index, button) in buttons.iter().enumerate() {
            // One predicate for a naming and for the ladder, so a key a naming
            // may not have is a key the ladder may not hand out either.
            let allowed = |key: Key| {
                let usable = match key {
                    Key::Char(character) => Key::named(character).is_some(),
                    _ => true,
                };
                // Escape is the way out, so the first button never answers it —
                // whether it asked for it or merely reached it down the ladder.
                usable
                    && !(index == 0 && key == Key::Escape)
                    && !taken.iter().any(|held| held.same_as(key))
            };
            let resolved = button
                .key
                .filter(|key| allowed(*key))
                .or_else(|| ladder().find(|key| allowed(*key)));
            match resolved {
                Some(key) => taken.push(key),
                // The ladder is exhausted, so no later button can be keyed
                // either: stopping here is what keeps the answer a prefix of the
                // list rather than a list with holes in it.
                None => break,
            }
        }
        taken
    }
}

/// Every key a button can fall back to, in the order they are offered.
///
/// 🔑 **Escape before Enter, which is not arbitrary.** It is what makes a list of
/// two unnamed buttons resolve to Enter then Escape: the first button skips
/// Escape and takes Enter, and the second finds Escape free. Enter first would
/// hand Enter to the second button of any list whose first names a character,
/// which is not what this module has ever drawn.
///
/// The ASCII graphic characters are exactly what [`Key::named`] admits, so every
/// key this yields can be drawn at a width [`cells`] can count.
fn ladder() -> impl Iterator<Item = Key> {
    [Key::Escape, Key::Enter]
        .into_iter()
        .chain((b'!'..=b'~').map(|point| Key::Char(point as char)))
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
    /// **Nobody chose.** The popup was closed, Ctrl-C ended it, or there was no
    /// way to read a choice at all.
    ///
    /// 🚨 **Ctrl-C answers no button, and that is a decision rather than an
    /// omission.** With several buttons every candidate target is a real action
    /// the user did not pick, and the kit never invents a choice on their behalf —
    /// the same rule it already applies to a click on the body and to an unbound
    /// key. SCOPE.md §7.5.8 records it.
    Dismissed,
    /// The popup never reported that it started, so it never drew. ➕ Or, from
    /// 0.5.3, Herdr answered that no client is attached, so no popup was opened.
    ///
    /// ⚠️ This marker is the only evidence either way. `plugin.pane.open`
    /// answers `ok` regardless, and a popup cannot be found in `pane.list`.
    NeverShown,
    /// Nobody answered inside `WAIT`.
    ///
    /// ➕ From 0.5.3 the popup is ended before this is answered, where the kit
    /// can prove the process is this dialog's own. See [`ask`].
    TimedOut,
    /// The channel carried a word that names no button of this dialog.
    ///
    /// Kept apart from [`Unanswered::Dismissed`] because it means something
    /// different: a popup left over from an older build, or a channel somebody
    /// else wrote into. ⚠️ An index past the last button lands here too, rather
    /// than becoming a choice.
    Unrecognised(String),
}

/// Which button the user chose, or why nobody did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The user chose the button at this index, counting from zero.
    Chose(usize),
    /// Nobody chose, and this is why.
    Unanswered(Unanswered),
}

impl Answer {
    /// Whether the user explicitly chose the button at `index`.
    ///
    /// 🔑 **The safety property, as one call.** Every other outcome answers
    /// `false`: another button, every failure to open, to start, or to be
    /// answered, and **an index this dialog has no button for**. A caller acting
    /// on an actioned dialog asks this rather than matching, because a match
    /// written the other way round acts on silence.
    ///
    /// Buttons are identified by index rather than by label, because labels are
    /// the caller's: they can repeat, and they can be empty.
    pub fn chose(&self, index: usize) -> bool {
        *self == Answer::Chose(index)
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
    match transport.open_pane(open_params(plugin_id, dialog, &[], None)) {
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
/// of them answers [`Answer::Chose`]. Use [`Answer::chose`].
///
/// ➕ **Which key answers each button rides on the buttons.** A caller that names
/// nothing gets Enter on the first and Escape on the second, which is what this
/// module has always drawn. A caller whose leading action destroys something
/// unrecoverable names a key with [`Button::on_key`], and can put the safe answer
/// under Enter by naming it on a later button.
///
/// 🚨 **A list with no buttons is refused rather than opened.** Nobody could
/// answer it, and the single-popup limit is global (§7.5.3), so an unanswerable
/// question would block every dialog in every workspace until it timed out.
///
/// 🚨 **No popup outlives this call, and none opens where nobody can see it.**
/// ✅ Measured 2026-09-24 on Herdr 0.9.1: with no client attached, a popup opens
/// on a pty nobody sees and holds the global slot. So two guards, SCOPE.md
/// §7.5.9:
///
/// - **The pre-check is the fast path.** `ask` first sends
///   `client.window_title.clear` through [`Transport::clear_window_title`]. The
///   reason `no_foreground_client` answers [`Unanswered::NeverShown`] at once,
///   and no popup is opened. ⚠️ With a client attached, that call re-emits
///   Herdr's default window title and drops any title override. And the reason
///   code is undocumented: it is what 0.9.1 was measured to answer.
/// - **The timeout kill is the backstop.** A client can detach between the
///   pre-check and the open, and a transport may not implement the pre-check at
///   all. When the wait runs out, `ask` ends the popup process before it
///   answers [`Unanswered::TimedOut`], but only a process it can prove is this
///   dialog's own popup. ⚠️ On Windows nothing is signalled, so a timed-out
///   popup stays open until somebody answers it or Herdr stops, as in 0.5.2.
pub fn ask(
    transport: &mut impl Transport,
    plugin_id: &str,
    dialog: &Dialog,
    buttons: &[Button],
) -> Answer {
    if buttons.is_empty() {
        return Answer::Unanswered(Unanswered::Failed(
            "a question with no buttons cannot be answered".to_string(),
        ));
    }

    // 🚨 Asked before anything is opened, because a popup nobody can see still
    // holds the global slot for the whole WAIT. Only the one measured reason
    // stops here. Any other answer, an error included, opens as 0.5.2 did, and
    // the timeout kill is the backstop for that path.
    if transport.clear_window_title() == Ok(ClientWindowTitleReason::NoForegroundClient) {
        return Answer::Unanswered(Unanswered::NeverShown);
    }

    let channel = match Channel::new() {
        Ok(channel) => channel,
        Err(e) => {
            return Answer::Unanswered(Unanswered::Failed(format!(
                "cannot make a channel for the question: {}",
                e
            )))
        }
    };

    match transport.open_pane(open_params(plugin_id, dialog, buttons, Some(&channel))) {
        Ok(()) => decide(channel.watch(), Button::keys(buttons).len()),
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
    buttons: &[Button],
    channel: Option<&Channel>,
) -> PluginPaneOpenParams {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(STATE_VAR.to_string(), dialog.state.as_wire().to_string());
    env.insert(TITLE_VAR.to_string(), dialog.title.clone());
    env.insert(BODY_VAR.to_string(), dialog.body.clone());
    // 🔑 Each button travels as exactly what it draws, carrying the **resolved**
    // key rather than the named one, so the popup half is told what a caller's
    // naming actually became. Walking the resolved keys rather than the buttons is
    // also what drops a tail the ladder could not key: a button the frame would
    // never draw is a button the popup is never told about.
    for (index, (key, button)) in Button::keys(buttons).iter().zip(buttons).enumerate() {
        env.insert(button_var(index), keyed(&key.drawn(), &button.label));
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

/// What an ending means, given how many buttons the dialog actually had.
///
/// **Only an exact index is acted on.** Everything else is reported as
/// unanswered, and none of it can ever answer [`Answer::Chose`].
///
/// 🔑 **The index is matched by rebuilding what the popup writes**, rather than by
/// parsing. The two halves share one vocabulary that way, and `00`, `+0` and any
/// other spelling a parser would accept stays unrecognised — which is the same
/// discipline [`Key::drawn`] and [`Key::from_wire`] already keep.
///
/// ⚠️ **`keys` is the count, never the caller's list.** A button the ladder could
/// not key is drawn nowhere and sent nowhere, so an index reaching it names no
/// button of this dialog.
fn decide(ended: Ended, keys: usize) -> Answer {
    match ended {
        Ended::Answered(word) => match (0..keys).find(|index| index.to_string() == word) {
            Some(index) => Answer::Chose(index),
            // Includes DISMISSED_WORD, which is how Ctrl-C and every other
            // choiceless ending arrive immediately rather than being waited out.
            None if word == DISMISSED_WORD => Answer::Unanswered(Unanswered::Dismissed),
            None => Answer::Unanswered(Unanswered::Unrecognised(word)),
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
        // 🚨 Giving up on a popup ends it. Left running, it holds the global
        // slot, and an answer given later is lost, because this channel's
        // directory goes when the channel drops.
        self.end_popup();
        Ended::TimedOut
    }

    /// Ends the popup this channel was waiting on, if it can be proven to be it.
    ///
    /// 🚨 **A pid alone proves nothing.** A process that exits frees its pid, and
    /// a later process of Mike's can be given the same one. Signalling that
    /// process is worse than leaving an orphan popup. So the marker is trusted
    /// only while the popup holds it locked: [`run`] takes the lock before it
    /// writes its pid, and keeps it until the process exits. A pid cannot be
    /// reused while its process lives, so a held lock ties the pid to this
    /// dialog's own popup.
    ///
    /// Signals nothing unless all of these hold:
    ///
    /// - The marker is locked by somebody else, which is the popup being alive.
    /// - The marker names a pid exactly as a number, and not `0`, which names a
    ///   process group.
    /// - The pid is not this process.
    ///
    /// ⚠️ **One window is left, and it is microseconds wide.** The popup could
    /// exit between the lock check and the signal, and its pid could be reused
    /// in that gap. Herdr must reap it first, and the pid space must come round
    /// to it. Closing this would need a pidfd or a process handle, which `std`
    /// does not offer on every Unix and `lib.rs` forbids reaching with `unsafe`.
    ///
    /// Answers whether a signal was sent, which is what the tests read.
    fn end_popup(&self) -> bool {
        if !popup_holds(&self.started) {
            return false;
        }
        match Self::read(&self.started).as_deref().and_then(popup_pid) {
            Some(pid) => terminate(pid),
            None => false,
        }
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
/// than wrong, and no outcome becomes [`Answer::Chose`].
///
/// Compile-verified only, like everything else Windows in this kit. Nobody on
/// this project has Windows hardware. See the README.
#[cfg(not(unix))]
fn process_is_alive(_pid: &str) -> bool {
    true
}

/// Writes this process's id to the started marker, and holds the marker locked.
///
/// 🔑 **The lock is what lets [`Channel::end_popup`] trust the pid.** It is taken
/// before the pid is written, so a marker with a pid in it is already locked,
/// and it is released only when the returned file drops or the process exits.
///
/// A lock that cannot be taken still writes the pid, so the liveness check
/// works as before. The only thing lost is the kill, which then never fires.
/// That fails towards an orphan popup rather than towards a wrong signal.
fn mark_started(path: &Path) -> Option<std::fs::File> {
    use std::io::Write;
    let mut file = std::fs::File::create(path).ok()?;
    let _ = file.lock();
    file.write_all(std::process::id().to_string().as_bytes())
        .ok()?;
    Some(file)
}

/// Whether some other open handle holds the marker locked.
///
/// Only a live popup does. Anything short of a clear "held", a missing file or
/// a lock call that failed included, answers `false`, so no signal is sent.
fn popup_holds(marker: &Path) -> bool {
    let Ok(file) = std::fs::File::open(marker) else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// The pid a marker names, if it names one this kit may ever signal.
///
/// 🔑 **Matched by rebuilding it, as [`decide`] matches an index.** `+12`, `012`
/// and ` 12` stay unread. `0` is refused, because `kill 0` signals the whole
/// process group, and this process's own pid is refused, because a popup is
/// never the process that asked.
fn popup_pid(text: &str) -> Option<u32> {
    let pid: u32 = text.parse().ok()?;
    (pid > 0 && pid.to_string() == text && pid != std::process::id()).then_some(pid)
}

/// Sends `SIGTERM`. `/bin/kill` for the reason [`process_is_alive`] gives.
#[cfg(unix)]
fn terminate(pid: u32) -> bool {
    std::process::Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// ⚠️ **Windows signals nothing.** A timed-out popup stays open until somebody
/// answers it or Herdr stops, exactly as in 0.5.2, and it holds the slot that
/// long. The pre-check in [`ask`] still keeps a popup from opening headless,
/// because that is a socket call and works on every platform.
///
/// Compile-verified only, like [`process_is_alive`] above.
#[cfg(not(unix))]
fn terminate(_pid: u32) -> bool {
    false
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
    /// The buttons, in order. **Empty is a bare dialog** that dismisses on any
    /// key, because a dialog with nothing to choose is exactly that.
    pub buttons: Vec<Button>,
    /// Where to write the chosen word.
    pub answer_file: Option<PathBuf>,
    /// Where to write this process's id, before drawing anything.
    pub started_file: Option<PathBuf>,
}

impl Popup {
    /// Reads what to draw out of the environment Herdr launched this pane with.
    ///
    /// 🔑 **`BUTTON_0`'s absence is what makes a dialog bare**, and the list ends
    /// at the first index that is not there. There is no separate count and no
    /// separate flag, because two ways of saying how many buttons there are could
    /// disagree.
    ///
    /// Every variable that is set but empty counts as absent, which is the
    /// idiom [`crate::env`] documents at nearly every call site.
    ///
    /// 🔑 **Each value is split at its first space**, which is [`button_var`]'s
    /// encoding read back: no key is ever drawn with a space in it, so the label
    /// on the right survives verbatim and may itself hold spaces or be empty.
    ///
    /// A value naming no key leaves that button unnamed, which is
    /// [`Key::from_wire`]'s rule and the same one the asking half applied.
    pub fn from_env(env: &Environment) -> Popup {
        let value = |key: &str| env.get(key).filter(|text| !text.is_empty());
        let mut buttons = Vec::new();
        while let Some(drawn) = value(&button_var(buttons.len())) {
            let (key, label) = drawn.split_once(' ').unwrap_or((drawn, ""));
            buttons.push(Button {
                label: label.to_string(),
                key: Key::from_wire(Some(key)),
            });
        }
        Popup {
            dialog: Dialog {
                state: State::from_wire(env.get(STATE_VAR)),
                title: value(TITLE_VAR).unwrap_or_default().to_string(),
                body: value(BODY_VAR).unwrap_or_default().to_string(),
            },
            buttons,
            answer_file: value(ANSWER_FILE_VAR).map(PathBuf::from),
            started_file: value(STARTED_FILE_VAR).map(PathBuf::from),
        }
    }
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
    /// Where each button was drawn, in the buttons' own order. **Empty on a bare
    /// dialog**, and shorter than the caller's list where the key ladder ran out.
    pub buttons: Vec<Rect>,
}

impl Frame {
    /// Which button is at this pane-local position, or `None` for no button.
    ///
    /// ⚠️ **A position on no button answers `None`**, and an actioned dialog
    /// treats that as no answer at all. Clicking the body text must not resolve a
    /// question whose leading button may be destructive.
    pub fn hit(&self, column: u16, row: u16) -> Option<usize> {
        self.buttons
            .iter()
            .position(|button| button.contains(column, row))
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
/// │           [ ↵ First ]  esc Next    │
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
/// 🔑 **The buttons share one row while they all fit it whole, and otherwise take
/// one row each**, each centred on its own:
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
/// So a dialog's height depends on its width, and `button_rows` decides
/// which shape it takes from the drawn widths rather than from a threshold.
///
/// ⚠️ **The button row is centred, and that choice is not from the approved
/// design.** Mike specified how the buttons look, not where they sit. Centred
/// follows TUI convention, and where Herdr puts its own was never measured.
///
/// ⚠️ **The height is not bounded here.** A body longer than the popup scrolls
/// in the pane, which can carry the bottom border off the top. The frame is
/// width-driven only, and the caller sizes the popup with [`HEIGHT`]. A long list
/// of stacked buttons costs height the same way.
///
/// 🔑 **`hot` changes attributes and never geometry.** Every value produces the
/// same characters in the same cells, which is what lets a hover redraw
/// overwrite the previous frame exactly. `None` is nothing hovered.
pub fn layout(dialog: &Dialog, buttons: &[Button], width: usize, hot: Option<usize>) -> Frame {
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

    let mut rects = Vec::new();
    if !buttons.is_empty() {
        lines.push(blank_line(inner, &colour));
        let (drawn, spans) = button_rows(buttons, text_width, &colour, hot);
        // The row the buttons start on is simply the row the first is pushed
        // to. Each span carries its own offset from there, which is zero for
        // every button when they share a row and its own index when they stack.
        let first = lines.len() as u16;
        rects = spans.into_iter().map(|span| span.at(first)).collect();
        lines.extend(drawn);
    }

    // Two rows below, except under a button row, where one is enough.
    let bottom = match buttons.is_empty() {
        false => 1,
        true => VERTICAL_PADDING,
    };
    for _ in 0..bottom {
        lines.push(blank_line(inner, &colour));
    }
    lines.push(bottom_line(width, &colour));

    Frame {
        text: lines.join("\n"),
        buttons: rects,
    }
}

/// Draws the dialog with no button highlighted.
///
/// The convenience [`layout`] exists behind. A caller that only displays a
/// dialog wants the characters and nothing else.
pub fn render(dialog: &Dialog, buttons: &[Button], width: usize) -> String {
    layout(dialog, buttons, width, None).text
}

/// A button's extent, before the frame knows which row the buttons start on.
///
/// `row` counts from the first button row rather than from the top of the
/// dialog, because a stacked layout puts each button on its own row and the
/// frame is the only thing that knows where those rows begin.
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

/// The button rows, and where each button landed across them.
///
/// 🔑 **They share one row only while they all fit it whole. Otherwise they
/// stack, one per row.** Decided by Mike on 2026-09-11 for a pair, after the
/// preview showed what the alternative actually rendered: at the 24-cell floor
/// the labels were being cut to `↵ reb` and `esc kee`, so "rebuild anyway" and
/// "keep them" both became fragments. Truncating rather than pushing the row
/// through the right border was the right instinct, but three characters of a
/// label is not a label.
///
/// ➕ **A sum in place of a pair generalises it rather than replacing it**, and
/// the `n = 2` answer is unchanged. Greedy packing that fills each row was
/// rejected: it makes the emphasised button's position depend on label lengths,
/// and it produces ragged rows the approved design never showed.
///
/// Two alternatives were considered and rejected: drawing the keys alone loses
/// the words entirely, and raising [`MIN_WIDTH`] means a narrow pane gets no
/// dialog at all rather than a usable one. Stacking costs one row of height per
/// button, which is the cheapest thing here to spend.
///
/// ⚠️ **The threshold is measured, not a number.** The kit draws the key
/// affordances itself and the labels are the caller's, so the question is
/// whether these drawn buttons and the gaps between them fit *this* frame —
/// never whether the frame is narrower than some constant.
///
/// 🔑 **Only the first button is emphasised**, inverted and padded; every other
/// one is plain text. Emphasis is positional, so the caller chooses it by
/// ordering the list. SCOPE.md §7.5.4 records why a button carries no emphasis
/// field.
fn button_rows(
    buttons: &[Button],
    text_width: usize,
    colour: &str,
    hot: Option<usize>,
) -> (Vec<String>, Vec<Span>) {
    // 🔑 Each button draws the key that answers it, read through the one resolver
    // both input paths read. So the frame cannot advertise a key that answers
    // nothing, whatever a caller named — and a button the ladder could not key
    // falls off this walk, so it is never drawn at all. See [`Button::keys`].
    let drawn: Vec<(String, bool, bool)> = Button::keys(buttons)
        .iter()
        .zip(buttons)
        .enumerate()
        .map(|(index, (key, button))| {
            let first = index == 0;
            (
                drawn_button(&key.drawn(), &button.label, first, text_width),
                first,
                hot == Some(index),
            )
        })
        .collect();

    let together = drawn.iter().map(|(text, ..)| cells(text)).sum::<usize>()
        + BUTTON_GAP * drawn.len().saturating_sub(1);
    if together <= text_width {
        let (line, spans) = buttons_row(&drawn, text_width, colour);
        return (vec![line], spans);
    }

    // Stacked, in the caller's order, so reading order reaches the emphasised
    // button first.
    let mut lines = Vec::with_capacity(drawn.len());
    let mut spans = Vec::with_capacity(drawn.len());
    for (row, item) in drawn.into_iter().enumerate() {
        let (line, alone) = buttons_row(&[item], text_width, colour);
        lines.push(line);
        spans.push(Span {
            row: row as u16,
            ..alone[0]
        });
    }
    (lines, spans)
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
/// the terminal's own background colour, so the row stays legible in a light
/// theme and a dark one without this module knowing which is in force.
///
/// 🔑 **Hover adds an underline, and nothing else.** A stronger treatment was
/// considered and rejected: inversion already means "this is the button the
/// dialog leads with", so giving a hovered sibling the same treatment would make
/// the buttons look alike exactly when the user is about to click one. An underline
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
/// labels that will not share a row; this handles one label that will not fit a
/// row by itself, which no layout can rescue. Pushing it through the right
/// border instead would break every row's alignment at once.
///
/// The key affordance and the first button's own padding are never shortened,
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
/// Dropping the separator for an empty label is what keeps `drawn_button`
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
    // at once. 🔑 The marker stays open and locked until this returns, which is
    // what lets the waiting side end this process when it gives up.
    let _marker = popup.started_file.as_deref().and_then(mark_started);

    // A dialog with no buttons is bare, and a dialog with no answer file has
    // nowhere to answer. Either one means nobody is waiting on a choice.
    let actioned = !popup.buttons.is_empty();
    let answer = interact(&popup.dialog, &popup.buttons);

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
/// settled. Every answer is therefore reachable from the keyboard alone, in every
/// configuration, and a dialog whose clicks never arrive is fully usable rather
/// than stuck. 🔑 **[`Button::keys`] holds that property rather than spending
/// it**: every button always answers exactly one key, and the frame draws it.
///
/// 🔑 **A click on a button means that button's answer, whatever key is named.**
/// The mouse aims at a labelled button, so what it resolves is never in question.
///
/// 🚨 **Every ending that is not a choice writes [`DISMISSED_WORD`]**, which
/// includes Ctrl-C, a terminal that stopped answering, and a line naming nothing.
/// "Fail closed to the cancel button" has no meaning once there is no cancel
/// button, and choosing the last button instead would only be safe if every
/// caller put its safe answer last — which the kit can neither enforce nor check.
fn interact(dialog: &Dialog, buttons: &[Button]) -> String {
    use crossterm::event::{read, Event, KeyEventKind, MouseButton, MouseEventKind};

    let keys = Button::keys(buttons);
    let Some(_terminal) = TerminalState::enter() else {
        // No tty, so no raw mode and no mouse. Draw once and read a line.
        println!("{}", render(dialog, buttons, FALLBACK_WIDTH));
        return match buttons.is_empty() {
            // 🔑 The same keys as the raw-mode path, from the same resolver. A
            // dialog that answered differently depending on which input path the
            // terminal took would be harder to diagnose than one simply bound
            // the wrong way.
            false => read_line(&keys),
            true => dismiss_on_a_line(),
        };
    };

    let mut width = pane_width();
    let mut hot = None;
    let mut frame = draw(dialog, buttons, width, hot);

    loop {
        match read() {
            Ok(Event::Key(key)) => {
                // A press and its release both arrive on some terminals, and
                // acting on both would read one keystroke as two.
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match buttons.is_empty() {
                    // A bare dialog dismisses on any key.
                    true => return DISMISSED_WORD.to_string(),
                    false => {
                        if let Some(answer) = key_answer(key.code, key.modifiers, &keys) {
                            return answer;
                        }
                    }
                }
            }
            Ok(Event::Mouse(mouse)) => match mouse.kind {
                MouseEventKind::Down(button) => {
                    // ⚠️ Only the left button activates. A right-click is
                    // forwarded identically, as `<2;`, and treating it as an
                    // activation would put a destructive first button behind a
                    // menu gesture that means nothing in a dialog.
                    match buttons.is_empty() {
                        // Nothing to aim at, so any button dismisses.
                        true => return DISMISSED_WORD.to_string(),
                        false if button != MouseButton::Left => continue,
                        false => match frame.hit(mouse.column, mouse.row) {
                            Some(index) => return index.to_string(),
                            // A click on the body is not an answer. Resolving
                            // it would let a misclick fire a leading button
                            // whose action may be destructive.
                            None => continue,
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
                hot = None;
                frame = draw(dialog, buttons, width, hot);
            }
            Ok(_) => continue,
            // ⚠️ Fails closed. A terminal that stopped answering cannot be read
            // as the user choosing anything at all.
            Err(_) => return DISMISSED_WORD.to_string(),
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
fn draw(dialog: &Dialog, buttons: &[Button], width: usize, hot: Option<usize>) -> Frame {
    use std::io::Write;
    let frame = layout(dialog, buttons, width, hot);
    print!("{}{}", HOME, frame.text.replace('\n', "\r\n"));
    let _ = std::io::stdout().flush();
    frame
}

/// The word one keypress writes, or `None` for "keep waiting".
///
/// 🔑 **Every key here answers the button the frame drew it on**, which is the
/// property [`Button::keys`] exists to hold. ✅ The measurement that settled the
/// design holds for all of them: taken 2026-09-11 by logging raw stdin bytes in
/// every pane process and injecting keystrokes, a popup takes keyboard input
/// **exclusively** and the base pane received nothing.
///
/// 🚨 **Ctrl-C always ends the dialog, is never configurable, and is decided
/// first.** It is the last way out of a raw-mode dialog: a popup carries no pane
/// id to close, clicks into one are unproven, and a caller must not be able to
/// take the exit away. Deciding it before anything else is what stops a named
/// `'c'` from shadowing it, and it is what makes naming a key over Escape safe.
/// 🚨 **It chooses no button**, because with several buttons every candidate is a
/// real action the user did not pick.
///
/// ⚠️ **The buttons are asked in reverse order.** With collisions already
/// resolved, no keypress can answer two, so the order changes no answer. It is
/// this way round so that a collision which somehow survived resolution would
/// answer **away from** the emphasised button rather than towards it.
///
/// **An unlisted key is ignored and the dialog stays open.** That is not the same
/// as ending it, and the difference matters: an unbound key must not resolve the
/// question in any direction. ⚠️ Enter and Escape are unbound like any other key
/// where no button names them.
fn key_answer(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
    keys: &[Key],
) -> Option<String> {
    use crossterm::event::{KeyCode, KeyModifiers};
    if matches!(code, KeyCode::Char('c')) && modifiers.contains(KeyModifiers::CONTROL) {
        return Some(DISMISSED_WORD.to_string());
    }
    keys.iter()
        .rposition(|key| key.answers(code, modifiers))
        .map(|index| index.to_string())
}

/// One line, where there is no terminal to read a keypress or a click from.
///
/// Raw mode needs a tty. The popup always has one, being a real pane. A test
/// harness piping stdin does not, and neither would a stray invocation from a
/// script. Falling back to a line keeps both choices answerable either way, and
/// means the fallback is exercised rather than being untested code that only
/// runs once something has already gone wrong.
fn read_line(keys: &[Key]) -> String {
    let mut typed = String::new();
    let read = std::io::stdin().read_line(&mut typed);
    line_answer(
        match read {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(typed.as_str()),
        },
        keys,
    )
}

/// What a typed line means, with the reading of stdin taken out.
///
/// The split is [`Ended`]'s, for [`Ended`]'s reason: a decision nobody can reach
/// without a pty is a decision that rots. `None` is end of input or a stdin that
/// could not be read at all.
///
/// 🔑 **It turns the line into a keypress and asks [`key_answer`], rather than
/// deciding anything itself.** An empty line is Enter and a single character is
/// that character, so the two input paths cannot disagree **by construction**
/// rather than by two implementations being kept in step. A test asserts the
/// agreement as well, because construction is only an argument until it is
/// measured.
///
/// ⚠️ **Escape cannot be typed as a line**, so a dialog with a button on Escape
/// has no line that reaches it. That is what the choiceless default below is for.
///
/// ⚠️ Chooses nothing everywhere else, and has to: this path gets one line rather
/// than a loop, so "keep waiting" is not available to it. No input at all,
/// several characters, and a keypress that answers nothing all end the dialog
/// unanswered — including a word that merely *starts* with a named key, which is
/// not that keypress.
fn line_answer(typed: Option<&str>, keys: &[Key]) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};
    let Some(text) = typed.map(str::trim) else {
        // End of input is not an answer.
        return DISMISSED_WORD.to_string();
    };
    let mut characters = text.chars();
    let pressed = match (characters.next(), characters.next()) {
        (None, _) => Some(KeyCode::Enter),
        (Some(typed), None) => Some(KeyCode::Char(typed)),
        // Several characters are no keypress at all.
        _ => None,
    };
    pressed
        .and_then(|code| key_answer(code, KeyModifiers::NONE, keys))
        .unwrap_or_else(|| DISMISSED_WORD.to_string())
}

/// A bare dialog with no tty. Anything at all dismisses it, including nothing.
fn dismiss_on_a_line() -> String {
    let mut typed = String::new();
    let _ = std::io::stdin().read_line(&mut typed);
    DISMISSED_WORD.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The endings that are slow to provoke are provoked in milliseconds here.
    const QUICK: Duration = Duration::from_millis(250);

    #[test]
    fn the_dismissal_word_can_never_be_a_button() {
        // It travels through the same file as the indices, so a word a button
        // could also write would make one outcome unreachable.
        assert!(
            DISMISSED_WORD.parse::<usize>().is_err(),
            "{} reads as an index",
            DISMISSED_WORD
        );
        assert_eq!(
            decide(Ended::Answered(DISMISSED_WORD.to_string()), KEYABLE),
            Answer::Unanswered(Unanswered::Dismissed),
            "the widest dialog there can be still read it as a dismissal"
        );
    }

    #[test]
    fn only_an_exact_index_chooses_a_button() {
        // 🔑 The safety property, swept over every ending and every spelling a
        // parser would have accepted.
        for ended in [
            Ended::Answered(String::new()),
            Ended::Answered("00".to_string()),
            Ended::Answered("+0".to_string()),
            Ended::Answered("0.0".to_string()),
            Ended::Answered("-1".to_string()),
            // Past the last button, which is a word naming no button here.
            Ended::Answered("2".to_string()),
            Ended::Answered("primary".to_string()),
            Ended::Answered("0 ok".to_string()),
            Ended::Answered(DISMISSED_WORD.to_string()),
            Ended::Dismissed,
            Ended::NeverShown,
            Ended::TimedOut,
        ] {
            for index in 0..2 {
                assert!(
                    !decide(ended.clone(), 2).chose(index),
                    "{:?} chose button {}",
                    ended,
                    index
                );
            }
        }
        assert!(decide(Ended::Answered("0".to_string()), 2).chose(0));
        assert!(decide(Ended::Answered("1".to_string()), 2).chose(1));
        // And a choice of one button is never a choice of another.
        assert!(!decide(Ended::Answered("1".to_string()), 2).chose(0));
        // An index no button has answers false rather than panicking.
        assert!(!decide(Ended::Answered("1".to_string()), 2).chose(99));
    }

    #[test]
    fn every_index_the_popup_writes_reads_back_as_that_button() {
        // 🔑 The two halves share one vocabulary: `key_answer` writes the index
        // and `decide` reads it. This measures the round trip rather than
        // trusting two calls to `to_string` to stay in step.
        use crossterm::event::{KeyCode, KeyModifiers};
        let buttons: Vec<Button> = (0..12).map(|n| Button::new(&format!("b{}", n))).collect();
        let keys = Button::keys(&buttons);
        assert_eq!(keys.len(), buttons.len());

        for (index, key) in keys.iter().enumerate() {
            let code = match key {
                Key::Enter => KeyCode::Enter,
                Key::Escape => KeyCode::Esc,
                Key::Char(character) => KeyCode::Char(*character),
            };
            let word = key_answer(code, KeyModifiers::NONE, &keys)
                .unwrap_or_else(|| panic!("{:?} answered nothing", key));
            assert_eq!(
                decide(Ended::Answered(word.clone()), keys.len()),
                Answer::Chose(index),
                "{:?} wrote {:?}, which did not read back as button {}",
                key,
                word,
                index
            );
        }
    }

    #[test]
    fn every_ending_maps_to_its_own_outcome() {
        assert_eq!(
            decide(Ended::Answered("0".to_string()), 2),
            Answer::Chose(0)
        );
        assert_eq!(
            decide(Ended::Answered("1".to_string()), 2),
            Answer::Chose(1)
        );
        assert_eq!(
            decide(Ended::Answered("keep".to_string()), 2),
            Answer::Unanswered(Unanswered::Unrecognised("keep".to_string()))
        );
        assert_eq!(
            decide(Ended::Answered(DISMISSED_WORD.to_string()), 2),
            Answer::Unanswered(Unanswered::Dismissed)
        );
        assert_eq!(
            decide(Ended::Dismissed, 2),
            Answer::Unanswered(Unanswered::Dismissed)
        );
        assert_eq!(
            decide(Ended::NeverShown, 2),
            Answer::Unanswered(Unanswered::NeverShown)
        );
        assert_eq!(
            decide(Ended::TimedOut, 2),
            Answer::Unanswered(Unanswered::TimedOut)
        );
    }

    #[test]
    fn an_index_past_the_last_button_is_unrecognised_rather_than_a_choice() {
        // 🚨 The count is the dialog's own. A popup left over from a build with
        // more buttons must not reach one this dialog does not have.
        assert_eq!(
            decide(Ended::Answered("2".to_string()), 2),
            Answer::Unanswered(Unanswered::Unrecognised("2".to_string()))
        );
        assert_eq!(
            decide(Ended::Answered("2".to_string()), 3),
            Answer::Chose(2)
        );
        // A dialog with no buttons at all recognises no index.
        assert_eq!(
            decide(Ended::Answered("0".to_string()), 0),
            Answer::Unanswered(Unanswered::Unrecognised("0".to_string()))
        );
    }

    #[test]
    fn an_unrecognised_word_is_kept_rather_than_collapsed_into_a_dismissal() {
        // A popup left over from an older build and a popup the user closed are
        // different problems, and the caller reports them differently.
        assert_ne!(
            decide(Ended::Answered("keep".to_string()), 2),
            decide(Ended::Dismissed, 2)
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
        std::fs::write(&channel.answer, "0").unwrap();
        assert_eq!(
            channel.watch_within(QUICK, QUICK),
            Ended::Answered("0".to_string())
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
            std::fs::write(&answer, "1").unwrap();
        });
        assert_eq!(
            channel.watch_within(QUICK, Duration::from_secs(5)),
            Ended::Answered("1".to_string())
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

    /// A process standing in for a popup: alive, not this process, and ours to
    /// reap. Anything that outlives a test is killed by the test itself.
    #[cfg(unix)]
    fn stand_in() -> std::process::Child {
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap()
    }

    /// Writes `pid` to `marker` holding the lock, exactly as [`run`] leaves it.
    fn hold(marker: &Path, pid: u32) -> std::fs::File {
        use std::io::Write;
        let mut file = std::fs::File::create(marker).unwrap();
        file.lock().unwrap();
        file.write_all(pid.to_string().as_bytes()).unwrap();
        file
    }

    /// Whether `child` exits within two seconds, reaping it if it does.
    #[cfg(unix)]
    fn exits(child: &mut std::process::Child) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn a_popup_the_wait_gave_up_on_is_ended() {
        // 🚨 ✅ Measured 2026-09-24 on 0.9.1: a timed-out popup stayed alive
        // and held the global slot until somebody answered it or Herdr stopped.
        let channel = Channel::new().unwrap();
        let mut popup = stand_in();
        let _held = hold(&channel.started, popup.id());

        assert_eq!(channel.watch_within(QUICK, QUICK), Ended::TimedOut);

        let ended = exits(&mut popup);
        let _ = popup.kill();
        assert!(ended, "the popup outlived the wait that gave up on it");
    }

    #[cfg(unix)]
    #[test]
    fn a_marker_nobody_holds_is_never_signalled() {
        // 🚨 The pid-reuse case. The popup exited, which released its lock,
        // and another process now carries its pid. That process is alive, so
        // the liveness check cannot tell it apart, and only the lock can.
        let channel = Channel::new().unwrap();
        let mut unrelated = stand_in();
        std::fs::write(&channel.started, unrelated.id().to_string()).unwrap();

        assert_eq!(channel.watch_within(QUICK, QUICK), Ended::TimedOut);
        assert!(!channel.end_popup());

        let ended = exits(&mut unrelated);
        let _ = unrelated.kill();
        let _ = unrelated.wait();
        assert!(
            !ended,
            "a process the kit could not tie to the popup was signalled"
        );
    }

    #[test]
    fn only_a_held_marker_counts_as_the_popup() {
        let channel = Channel::new().unwrap();
        assert!(!popup_holds(&channel.started), "a missing marker was held");

        std::fs::write(&channel.started, "12").unwrap();
        assert!(
            !popup_holds(&channel.started),
            "an unlocked marker was held"
        );

        let held = hold(&channel.started, 12);
        assert!(popup_holds(&channel.started));
        drop(held);
        assert!(
            !popup_holds(&channel.started),
            "the lock outlived the popup's handle"
        );
    }

    #[test]
    fn the_popup_half_locks_its_marker_before_the_pid_can_be_read() {
        // 🔑 `run` goes through this, so the kill works only if this holds.
        let channel = Channel::new().unwrap();
        let marker = mark_started(&channel.started).expect("the marker was not written");
        assert!(popup_holds(&channel.started));
        assert_eq!(
            Channel::read(&channel.started),
            Some(std::process::id().to_string())
        );
        drop(marker);
        assert!(!popup_holds(&channel.started));
    }

    #[test]
    fn only_a_plain_pid_that_is_not_this_process_may_be_signalled() {
        assert_eq!(popup_pid("12"), Some(12));
        let own = std::process::id().to_string();
        for text in [
            "0",
            "+12",
            "012",
            " 12",
            "12 ",
            "-1",
            "",
            "twelve",
            own.as_str(),
        ] {
            assert_eq!(popup_pid(text), None, "{:?} could be signalled", text);
        }
    }

    #[test]
    fn a_held_marker_naming_this_process_is_never_signalled() {
        // Reached through `end_popup`, and not only through `popup_pid`, so a
        // guard moved out of the path that signals is noticed. If it failed,
        // this test process would receive the signal itself.
        let channel = Channel::new().unwrap();
        let _held = hold(&channel.started, std::process::id());
        assert!(!channel.end_popup());
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
        assert_eq!(cells(ENTER_KEY), 1);
        assert_eq!(cells(ESCAPE_KEY), 3);
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

    /// A button named `label` that asks for `key`, or for nothing.
    fn button(label: &str, key: Option<Key>) -> Button {
        Button {
            label: label.to_string(),
            key,
        }
    }

    /// The word a keypress writes, as the owned string `key_answer` hands back.
    fn word(index: usize) -> Option<String> {
        Some(index.to_string())
    }

    /// The configurations every keyboard test sweeps, named once.
    ///
    /// 🔑 **A closed set, swept rather than sampled.** Each row is a list a caller
    /// can actually build: every pair the round-3 design could express, and lists
    /// of three and four where the ladder reaches past Escape. A button no key
    /// answers is not in the list because it cannot be built.
    fn configurations() -> Vec<Vec<Button>> {
        let y = Some(Key::Char('y'));
        let n = Some(Key::Char('n'));
        vec![
            vec![button("Rebuild", None), button("Leave it", None)],
            vec![button("Rebuild", y), button("Leave it", None)],
            vec![button("Rebuild", None), button("Leave it", n)],
            vec![button("Rebuild", y), button("Leave it", n)],
            vec![button("Rebuild", y), button("Leave it", Some(Key::Enter))],
            vec![
                button("Delete", None),
                button("Keep", None),
                button("Archive", None),
            ],
            vec![
                button("Delete", Some(Key::Char('d'))),
                button("Keep", Some(Key::Enter)),
                button("Archive", None),
                button("Later", None),
            ],
        ]
    }

    #[test]
    fn a_caller_that_names_nothing_gets_enter_then_escape_then_characters() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;
        let pair = Button::keys(&[button("Rebuild", None), button("Leave it", None)]);

        // 🔑 The pair this module has always drawn, reproduced by the ladder.
        assert_eq!(pair, vec![Key::Enter, Key::Escape]);
        assert_eq!(key_answer(KeyCode::Enter, NONE, &pair), word(0));
        assert_eq!(key_answer(KeyCode::Esc, NONE, &pair), word(1));
        // Nothing else answers, which is what it did before any key was nameable.
        assert_eq!(key_answer(KeyCode::Char('y'), NONE, &pair), None);

        // Past the pair, the ladder hands out characters in codepoint order.
        let four: Vec<Button> = (0..4).map(|_| button("b", None)).collect();
        assert_eq!(
            Button::keys(&four),
            vec![Key::Enter, Key::Escape, Key::Char('!'), Key::Char('"')]
        );
    }

    #[test]
    fn the_ladder_runs_escape_then_enter_then_every_ascii_graphic_character() {
        // 🔑 Escape first is what gives the second of two unnamed buttons Escape,
        // and a button after a named first one Escape rather than Enter.
        let ladder: Vec<Key> = ladder().collect();
        assert_eq!(ladder.len(), 96, "the ladder is not 96 keys long");
        assert_eq!(ladder[0], Key::Escape);
        assert_eq!(ladder[1], Key::Enter);
        assert_eq!(ladder[2], Key::Char('!'));
        assert_eq!(ladder[95], Key::Char('~'));
        assert_eq!(
            Button::keys(&[
                button("Rebuild", Some(Key::Char('y'))),
                button("Leave", None)
            ]),
            vec![Key::Char('y'), Key::Escape],
            "a named first button moved Enter onto the second"
        );
    }

    /// How many buttons the ladder can key: 96 keys, less the 26 lowercase
    /// letters that are the same keypress as a capital already handed out.
    const KEYABLE: usize = 70;

    #[test]
    fn a_list_longer_than_the_ladder_can_tell_apart_loses_its_tail() {
        // ⚠️ 70 buttons keyed, so the 71st gets none. The answer stays a prefix
        // of the list rather than a list with a hole in it.
        let exactly: Vec<Button> = (0..KEYABLE).map(|_| button("b", None)).collect();
        let keys = Button::keys(&exactly);
        assert_eq!(
            keys.len(),
            KEYABLE,
            "a list the ladder can hold lost a button"
        );
        for (index, key) in keys.iter().enumerate() {
            assert!(
                !keys[..index].iter().any(|held| held.same_as(*key)),
                "{:?} answers two buttons",
                key
            );
        }

        // Far past the limit, and still exactly the limit.
        let over: Vec<Button> = (0..100).map(|_| button("b", None)).collect();
        assert_eq!(
            Button::keys(&over).len(),
            KEYABLE,
            "the 71st button got a key"
        );

        // A naming on the tail cannot rescue it: every key is already held.
        let mut named_tail = over.clone();
        named_tail[KEYABLE].key = Some(Key::Char('z'));
        assert_eq!(Button::keys(&named_tail).len(), KEYABLE);

        // And everything that walks the answer walks the prefix: the frame draws
        // 70 buttons, the popup is told about 70, and an index reaching the 71st
        // names no button.
        let dialog = Dialog::new(State::Info, "t", "b");
        let frame = layout(&dialog, &over, 60, None);
        assert_eq!(
            frame.buttons.len(),
            KEYABLE,
            "the frame drew an unkeyed button"
        );
        let env = open_params("p", &dialog, &over, None).env;
        assert!(env.contains_key(&button_var(KEYABLE - 1)));
        assert!(
            !env.contains_key(&button_var(KEYABLE)),
            "the popup was told about a button no key answers"
        );
        let past = KEYABLE.to_string();
        assert_eq!(
            decide(Ended::Answered(past.clone()), Button::keys(&over).len()),
            Answer::Unanswered(Unanswered::Unrecognised(past))
        );
    }

    #[test]
    fn a_named_key_answers_its_own_button_and_the_default_stops_answering() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;

        // 🔑 Naming replaces rather than adds, on every button alike.
        let first_named = Button::keys(&[
            button("Rebuild", Some(Key::Char('y'))),
            button("Leave it", None),
        ]);
        assert_eq!(key_answer(KeyCode::Char('y'), NONE, &first_named), word(0));
        assert_eq!(
            key_answer(KeyCode::Enter, NONE, &first_named),
            None,
            "Enter still answered a button it was named off"
        );
        assert_eq!(
            key_answer(KeyCode::Esc, NONE, &first_named),
            word(1),
            "the untouched button lost its default"
        );

        let second_named = Button::keys(&[
            button("Rebuild", None),
            button("Leave it", Some(Key::Char('n'))),
        ]);
        assert_eq!(key_answer(KeyCode::Char('n'), NONE, &second_named), word(1));
        assert_eq!(
            key_answer(KeyCode::Esc, NONE, &second_named),
            None,
            "Escape still answered a button it was named off"
        );
        assert_eq!(
            key_answer(KeyCode::Enter, NONE, &second_named),
            word(0),
            "the untouched button lost its default"
        );
    }

    #[test]
    fn enter_answers_a_later_button_when_it_is_named_there() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;

        // 🔑 The whole point of the first round, expressed with no special case:
        // the caller names Enter where it wants it.
        let keys = Button::keys(&[
            button("Rebuild", Some(Key::Char('y'))),
            button("Leave it", Some(Key::Enter)),
        ]);
        assert_eq!(keys, vec![Key::Char('y'), Key::Enter]);
        assert_eq!(key_answer(KeyCode::Enter, NONE, &keys), word(1));
        assert_eq!(key_answer(KeyCode::Char('y'), NONE, &keys), word(0));
        assert_eq!(key_answer(KeyCode::Esc, NONE, &keys), None);

        // And on the third of three, where Escape lands on the one between.
        let keys = Button::keys(&[
            button("Delete", Some(Key::Char('d'))),
            button("Archive", None),
            button("Keep", Some(Key::Enter)),
        ]);
        assert_eq!(keys, vec![Key::Char('d'), Key::Escape, Key::Enter]);
        assert_eq!(key_answer(KeyCode::Enter, NONE, &keys), word(2));
        assert_eq!(key_answer(KeyCode::Esc, NONE, &keys), word(1));
    }

    #[test]
    fn ctrl_c_dismisses_whatever_anybody_named() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;

        // 🚨 The last way out, in every configuration a caller can build, and it
        // chooses no button: the word it writes reads back as a dismissal.
        for buttons in configurations() {
            let keys = Button::keys(&buttons);
            let written = key_answer(KeyCode::Char('c'), KeyModifiers::CONTROL, &keys);
            assert_eq!(
                written.as_deref(),
                Some(DISMISSED_WORD),
                "{:?} took Ctrl-C away",
                keys
            );
            assert_eq!(
                decide(Ended::Answered(written.unwrap()), keys.len()),
                Answer::Unanswered(Unanswered::Dismissed),
                "{:?} read Ctrl-C as a choice",
                keys
            );
        }

        // And naming `c` cannot shadow it, which is why it is decided first.
        let keys = Button::keys(&[
            button("Rebuild", Some(Key::Char('c'))),
            button("Leave it", None),
        ]);
        assert_eq!(
            key_answer(KeyCode::Char('c'), KeyModifiers::CONTROL, &keys).as_deref(),
            Some(DISMISSED_WORD)
        );
        assert_eq!(
            key_answer(KeyCode::Char('c'), NONE, &keys),
            word(0),
            "a bare c stopped being the named key"
        );
    }

    #[test]
    fn a_named_character_needs_its_own_chord_and_not_a_modified_one() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let keys = Button::keys(&[
            button("Rebuild", Some(Key::Char('y'))),
            button("Leave it", None),
        ]);
        let keys = keys.as_slice();

        // Shift is how an uppercase character arrives, so it is the same key.
        for modifiers in [KeyModifiers::NONE, KeyModifiers::SHIFT] {
            assert_eq!(
                key_answer(KeyCode::Char('Y'), modifiers, keys),
                word(0),
                "{:?} was not the named key",
                modifiers
            );
        }
        // Every other chord is a different keypress, and resolving one would let a
        // stray Alt-y destroy something.
        for modifiers in [
            KeyModifiers::ALT,
            KeyModifiers::CONTROL,
            KeyModifiers::SUPER,
            KeyModifiers::ALT | KeyModifiers::SHIFT,
        ] {
            assert_eq!(
                key_answer(KeyCode::Char('y'), modifiers, keys),
                None,
                "{:?} answered as the named key",
                modifiers
            );
        }
    }

    #[test]
    fn an_unbound_key_is_ignored_rather_than_answering_either_way() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;

        // Every configuration: naming a key must not turn some other key into an
        // answer, in either direction.
        for buttons in configurations() {
            let keys = Button::keys(&buttons);
            let keys = keys.as_slice();
            for code in [
                KeyCode::Char(' '),
                KeyCode::Char('q'),
                KeyCode::Backspace,
                KeyCode::Tab,
                KeyCode::Up,
                KeyCode::F(1),
            ] {
                assert_eq!(
                    key_answer(code, NONE, keys),
                    None,
                    "{:?} answered under {:?}",
                    code,
                    keys
                );
            }
            // A bare `c` is not Ctrl-C, and reading it as one would put the
            // modifier check there for nothing.
            assert_eq!(key_answer(KeyCode::Char('c'), NONE, keys), None);
        }
    }

    #[test]
    fn each_button_draws_exactly_the_key_that_answers_it() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;

        // 🔑 The frame's claim, checked rather than restated, in every
        // configuration. What the frame draws is read back from the drawn text,
        // fed through the decoder, and has to answer the button it was drawn on.
        // This is what reddens if a key is ever left drawn on a button it no
        // longer reaches.
        let from_drawing = |drawn: &str| match drawn {
            ENTER_KEY => KeyCode::Enter,
            ESCAPE_KEY => KeyCode::Esc,
            other => KeyCode::Char(other.chars().next().expect("a drawn key is never empty")),
        };

        for buttons in configurations() {
            let keys = Button::keys(&buttons);
            assert_eq!(keys.len(), buttons.len(), "{:?} lost a key", keys);
            for (index, key) in keys.iter().enumerate() {
                assert_eq!(
                    key_answer(from_drawing(&key.drawn()), NONE, &keys),
                    word(index),
                    "{:?} draws {:?} on button {}, which it does not answer",
                    keys,
                    key.drawn(),
                    index
                );
                // And no two are the same keypress, which is what would make one
                // of the claims a lie whichever way the decoder resolved it.
                assert!(
                    !keys[..index].iter().any(|held| held.same_as(*key)),
                    "{:?} draws one key on two buttons",
                    keys
                );
            }
        }
    }

    #[test]
    fn a_key_is_drawn_as_itself() {
        assert_eq!(Key::Enter.drawn(), ENTER_KEY);
        assert_eq!(Key::Escape.drawn(), ESCAPE_KEY);
        assert_eq!(Key::Char('y').drawn(), "y");
    }

    #[test]
    fn a_line_answers_exactly_as_the_keypress_it_names_does() {
        use crossterm::event::{KeyCode, KeyModifiers};
        const NONE: KeyModifiers = KeyModifiers::NONE;

        // 🔑 The two input paths are reached under different terminal conditions.
        // The line path routes through the same decoder, and this measures that
        // rather than trusting it.
        for buttons in configurations() {
            let keys = Button::keys(&buttons);
            let keys = keys.as_slice();
            let expected =
                |code| key_answer(code, NONE, keys).unwrap_or_else(|| DISMISSED_WORD.to_string());
            for line in ["", "\n", "   \n"] {
                assert_eq!(
                    line_answer(Some(line), keys),
                    expected(KeyCode::Enter),
                    "{:?} disagreed with raw mode on an empty line {:?}",
                    keys,
                    line
                );
            }
            for (line, typed) in [("y", 'y'), ("Y\n", 'Y'), (" n \n", 'n'), ("q", 'q')] {
                assert_eq!(
                    line_answer(Some(line), keys),
                    expected(KeyCode::Char(typed)),
                    "{:?} disagreed with raw mode on {:?}",
                    keys,
                    line
                );
            }
        }
    }

    #[test]
    fn a_line_naming_no_keypress_fails_closed() {
        // ⚠️ This path gets one line rather than a loop, so "keep waiting" is not
        // available to it and anything unrecognised has to end the dialog
        // unanswered rather than choose any button.
        for buttons in configurations() {
            let keys = Button::keys(&buttons);
            let keys = keys.as_slice();
            assert_eq!(line_answer(None, keys), DISMISSED_WORD, "{:?}", keys);
            // A word that merely starts with a named key is not that keypress.
            for typed in ["yes", "y y", "no", "primary", "  xx  ", "0", "1"] {
                assert_eq!(
                    line_answer(Some(typed), keys),
                    DISMISSED_WORD,
                    "{:?} resolved {:?}",
                    keys,
                    typed
                );
            }
        }
    }

    #[test]
    fn every_key_survives_the_wire_it_is_drawn_on() {
        // 🔑 One vocabulary: what the frame draws is what the variable carries, so
        // every value this crate can write reads back as itself.
        for key in [Key::Enter, Key::Escape, Key::Char('y'), Key::Char('?')] {
            assert_eq!(
                Key::from_wire(Some(&key.drawn())),
                Some(key),
                "{:?} did not survive its own drawing",
                key
            );
        }
        assert_eq!(Key::from_wire(Some(" y ")), Some(Key::Char('y')));
    }

    #[test]
    fn a_wire_value_naming_no_key_falls_back_to_the_default() {
        // Absent, empty, several characters, and a character that cannot be drawn
        // at a measurable width all name no key. ⚠️ One rule pointing one way at
        // every button: a garbled key leaves its button unnamed, so it takes the
        // first free key down the ladder like any other.
        for word in [None, Some(""), Some("   "), Some("yes"), Some("\u{4f60}")] {
            assert_eq!(Key::from_wire(word), None, "{:?} named a key", word);
        }
    }

    #[test]
    fn a_naming_that_cannot_be_honoured_falls_down_the_ladder() {
        let pair = |first: Option<Key>, second: Option<Key>| {
            Button::keys(&[button("Rebuild", first), button("Leave it", second)])
        };
        let default = vec![Key::Enter, Key::Escape];

        // Escape never fires the action the dialog leads with.
        assert_eq!(pair(Some(Key::Escape), None), default);
        // A character the frame cannot measure names nothing, on any button.
        assert_eq!(pair(Some(Key::Char('\u{4f60}')), None), default);
        assert_eq!(pair(None, Some(Key::Char(' '))), default);
        // One keypress cannot answer two buttons, case included.
        assert_eq!(
            pair(Some(Key::Char('y')), Some(Key::Char('Y'))),
            vec![Key::Char('y'), Key::Escape]
        );
        // Enter on the second is that same collision until the first names a key.
        assert_eq!(pair(None, Some(Key::Enter)), default);
        assert_eq!(
            pair(Some(Key::Char('y')), Some(Key::Enter)),
            vec![Key::Char('y'), Key::Enter]
        );

        // Past two buttons the same rule holds: a collision on the third falls to
        // the first key still free, which skips everything already held.
        assert_eq!(
            Button::keys(&[
                button("a", None),
                button("b", None),
                button("c", Some(Key::Escape)),
                button("d", Some(Key::Char('!'))),
                button("e", Some(Key::Char('!'))),
            ]),
            vec![
                Key::Enter,
                Key::Escape,
                Key::Char('!'),
                Key::Char('"'),
                Key::Char('#'),
            ]
        );
    }

    #[test]
    fn resolving_a_resolved_list_changes_nothing() {
        // 🔑 Both halves run the resolver: the asking half on what the caller
        // built, and the popup half on what crossed the wire, which is already
        // resolved. So it has to be idempotent, or the halves would disagree.
        for buttons in configurations() {
            let keys = Button::keys(&buttons);
            let resolved: Vec<Button> = buttons
                .iter()
                .zip(&keys)
                .map(|(button, key)| Button {
                    key: Some(*key),
                    ..button.clone()
                })
                .collect();
            assert_eq!(Button::keys(&resolved), keys);
        }
    }

    #[test]
    fn no_naming_can_leave_a_button_without_a_key() {
        // 🚨 The one thing that stays unrepresentable. Every triple of namings,
        // wire values and hazards included, resolves to three keys that are not
        // the same keypress, and the first is never Escape.
        let namings = [
            None,
            Some(Key::Enter),
            Some(Key::Escape),
            Some(Key::Char('y')),
            Some(Key::Char('Y')),
            Some(Key::Char(' ')),
            Some(Key::Char('\u{4f60}')),
            Some(Key::Char('c')),
        ];
        for first in namings {
            for second in namings {
                for third in namings {
                    let keys = Button::keys(&[
                        button("a", first),
                        button("b", second),
                        button("c", third),
                    ]);
                    assert_eq!(keys.len(), 3, "{:?} stranded a button", keys);
                    for (index, key) in keys.iter().enumerate() {
                        assert!(
                            !keys[..index].iter().any(|held| held.same_as(*key)),
                            "{:?}, {:?}, {:?} resolved two buttons to one key",
                            first,
                            second,
                            third
                        );
                    }
                    assert_ne!(keys[0], Key::Escape, "Escape answered the first button");
                }
            }
        }
    }

    #[test]
    fn every_key_a_caller_can_name_costs_exactly_one_cell() {
        use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

        // 🔑 §7.5.4's rule, applied to the caller's keys rather than the kit's
        // glyphs, and checked against the real Unicode data for the same reason:
        // `cells` counts characters, so a key wider than one cell would draw a
        // frame one cell too wide, and an `Ambiguous` one would let a terminal
        // setting decide the width.
        for point in 0x21u8..=0x7e {
            let key = point as char;
            let drawn = Key::Char(key).drawn();
            assert_eq!(
                Key::from_wire(Some(&drawn)),
                Some(Key::Char(key)),
                "U+{:04X} cannot be named",
                point
            );
            assert_eq!(
                UnicodeWidthChar::width(key),
                Some(1),
                "{:?} is not one cell",
                key
            );
            assert_eq!(
                UnicodeWidthStr::width(drawn.as_str()),
                UnicodeWidthStr::width_cjk(drawn.as_str()),
                "{:?} is East Asian Width Ambiguous",
                key
            );
            assert_eq!(cells(&drawn), 1, "{:?} is not counted as one cell", key);
        }

        // And everything else names nothing, so no frame can be drawn wrong: a
        // wide character, a whitespace one, two Ambiguous ones, and a control.
        for key in ['\u{4f60}', ' ', '\t', '\u{26a0}', '\u{24d8}', '\u{7f}'] {
            assert_eq!(
                Button::keys(&[button("a", Some(Key::Char(key))), button("b", None)]),
                vec![Key::Enter, Key::Escape],
                "{:?} was nameable",
                key
            );
        }

        // The kit's own two keys keep the widths every threshold assumes.
        assert_eq!(cells(&Key::Enter.drawn()), 1);
        assert_eq!(cells(&Key::Escape.drawn()), 3);
    }
}
