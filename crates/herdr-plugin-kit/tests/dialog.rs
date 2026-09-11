//! What the `dialog` module promises.
//!
//! Everything here runs without a Herdr server. The module takes its sender as
//! a trait, so the opener half is exercised against fakes that answer exactly
//! what a measured Herdr answers, including `ui_busy`.
//!
//! [`a_popup_that_answers_is_heard_across_the_channel`] is the end-to-end one:
//! a fake opener writes the started marker and then the answer, exactly as the
//! real popup process does, and the whole channel is driven for real. A thread
//! stands in for the process, which is the only part that is not real.

#![cfg(feature = "dialog")]

use std::time::{Duration, Instant};

use herdr_plugin_kit::api::generated::{PluginPaneOpenParams, PluginPanePlacement};
use herdr_plugin_kit::dialog::{
    ask, layout, notify, render, Answer, Buttons, Dialog, Hot, OpenError, PaneOpener, Popup, Rect,
    Shown, State, Unanswered, ANSWER_FILE_VAR, BODY_VAR, BUSY_CODE, CANCEL_KEY, CANCEL_VAR,
    CANCEL_WORD, DEFAULT_CANCEL, ENTRYPOINT, HEIGHT, PRIMARY_KEY, PRIMARY_VAR, PRIMARY_WORD,
    STARTED_FILE_VAR, STATE_VAR, TITLE_VAR, WIDTH,
};
use herdr_plugin_kit::env::Environment;

const PLUGIN: &str = "mikebronner.test-plugin";

/// The literal Herdr answers this module has to tell apart.
///
/// ✅ Reproduced 2026-09-11 on 0.9.0 by opening a second popup while one was up.
const BUSY_MESSAGE: &str = "a popup pane is already open";

/// An opener that records what it was handed and answers what it was told to.
struct Fake {
    answer: Result<(), OpenError>,
    seen: Vec<PluginPaneOpenParams>,
}

impl Fake {
    fn ok() -> Fake {
        Fake {
            answer: Ok(()),
            seen: Vec::new(),
        }
    }

    fn busy() -> Fake {
        Fake {
            answer: Err(OpenError::from_error(BUSY_CODE, BUSY_MESSAGE)),
            seen: Vec::new(),
        }
    }

    fn failing() -> Fake {
        Fake {
            answer: Err(OpenError::Failed("socket is gone".to_string())),
            seen: Vec::new(),
        }
    }

    fn only(&self) -> &PluginPaneOpenParams {
        assert_eq!(self.seen.len(), 1, "expected exactly one request");
        &self.seen[0]
    }
}

impl PaneOpener for Fake {
    fn open(&mut self, params: PluginPaneOpenParams) -> Result<(), OpenError> {
        self.seen.push(params);
        self.answer.clone()
    }
}

/// An opener that behaves like the popup process: marker first, then an answer.
struct Answering {
    word: String,
    /// `false` skips the marker, which is what a popup that never drew looks like.
    marks_started: bool,
    delay: Duration,
}

impl PaneOpener for Answering {
    fn open(&mut self, params: PluginPaneOpenParams) -> Result<(), OpenError> {
        let started = params.env.get(STARTED_FILE_VAR).cloned();
        let answer = params
            .env
            .get(ANSWER_FILE_VAR)
            .cloned()
            .expect("an actioned dialog must carry an answer file");
        let word = self.word.clone();
        let marks_started = self.marks_started;
        let delay = self.delay;
        std::thread::spawn(move || {
            if marks_started {
                let started = started.expect("an actioned dialog must carry a started file");
                std::fs::write(started, std::process::id().to_string()).unwrap();
            }
            std::thread::sleep(delay);
            std::fs::write(answer, word).unwrap();
        });
        Ok(())
    }
}

fn dialog() -> Dialog {
    Dialog::new(State::Warning, "Careful", "Two panes are running agents.")
}

fn buttons() -> Buttons {
    Buttons::new("Rebuild", "Leave it")
}

/// Strips CSI sequences, so a layout assertion reads the characters only.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // `ESC [ ... <letter>` is the whole of what this module emits.
        for c in chars.by_ref() {
            if c.is_ascii_alphabetic() {
                break;
            }
        }
    }
    out
}

/// Counts cells without reusing the module's own counter.
///
/// `unicode-width` rather than a re-implementation, so a layout assertion is
/// checked against the real Unicode data. A dev-dependency, so no consumer of
/// this crate carries it.
fn width_of(line: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(line)
}

// ── The request the opener is handed ──────────────────────────────────────

#[test]
fn the_placement_is_always_popup_and_never_overlay() {
    // 🚨 The safety property of the whole module. ✅ An overlay would hand back
    // a pane_id, and it covers the entire tab with zero rows of any underlying
    // pane surviving, so it is not a dialog.
    let mut opener = Fake::ok();
    notify(&mut opener, PLUGIN, &dialog());
    assert_eq!(
        opener.only().placement,
        Some(PluginPanePlacement::Popup),
        "a dialog asked for something other than a popup"
    );

    let mut opener = Fake::ok();
    ask(&mut opener, PLUGIN, &dialog(), &buttons());
    assert_eq!(opener.only().placement, Some(PluginPanePlacement::Popup));
}

#[test]
fn the_request_carries_the_size_the_schema_accepts() {
    // ✅ width and height are popup-only. Any other placement answers
    // `invalid_params`, so these two travelling together with `popup` is the
    // whole of what makes them legal.
    //
    // Asserted on the serialized form rather than the typed one, because that
    // is what Herdr actually receives. `PopupSize` derives no `PartialEq`, and
    // the generated file is never hand-edited to add one.
    let mut opener = Fake::ok();
    notify(&mut opener, PLUGIN, &dialog());
    let wire = serde_json::to_value(opener.only()).unwrap();
    assert_eq!(wire["placement"], serde_json::json!("popup"));
    assert_eq!(wire["width"], serde_json::json!(WIDTH));
    assert_eq!(wire["height"], serde_json::json!(HEIGHT));
    // A size the schema rejects would serialize as a bare number, and Herdr
    // reads a number as cells rather than a percentage.
    assert!(wire["width"].is_string(), "the width lost its percentage");
    assert!(wire["height"].is_string(), "the height lost its percentage");
}

#[test]
fn the_request_names_the_kit_entrypoint_the_plugin_id_and_takes_focus() {
    let mut opener = Fake::ok();
    ask(&mut opener, PLUGIN, &dialog(), &buttons());
    let params = opener.only();
    assert_eq!(params.entrypoint, ENTRYPOINT);
    assert_eq!(params.plugin_id, PLUGIN);
    assert!(params.focus, "a dialog nobody focused takes no keys");
}

#[test]
fn a_bare_dialog_carries_no_channel_and_no_buttons() {
    // 🔑 The bare variant waits for nothing, so it has nothing to learn. Files
    // it never reads would be files nobody removes.
    let mut opener = Fake::ok();
    notify(&mut opener, PLUGIN, &dialog());
    let env = &opener.only().env;
    for absent in [ANSWER_FILE_VAR, STARTED_FILE_VAR, PRIMARY_VAR, CANCEL_VAR] {
        assert!(
            !env.contains_key(absent),
            "{} was sent to a bare dialog",
            absent
        );
    }
    assert_eq!(env.get(STATE_VAR).map(String::as_str), Some("warning"));
    assert_eq!(env.get(TITLE_VAR).map(String::as_str), Some("Careful"));
    assert_eq!(
        env.get(BODY_VAR).map(String::as_str),
        Some("Two panes are running agents.")
    );
}

#[test]
fn an_actioned_dialog_carries_both_files_and_both_labels() {
    let mut opener = Fake::ok();
    ask(&mut opener, PLUGIN, &dialog(), &buttons());
    let env = &opener.only().env;
    for present in [ANSWER_FILE_VAR, STARTED_FILE_VAR] {
        assert!(
            env.get(present).is_some_and(|path| !path.is_empty()),
            "{} is missing, so the answer has nowhere to travel",
            present
        );
    }
    assert_ne!(
        env[ANSWER_FILE_VAR], env[STARTED_FILE_VAR],
        "one file cannot be both the marker and the answer"
    );
    assert_eq!(env.get(PRIMARY_VAR).map(String::as_str), Some("Rebuild"));
    assert_eq!(env.get(CANCEL_VAR).map(String::as_str), Some("Leave it"));
}

#[test]
fn every_state_travels_as_its_own_word() {
    for (state, word) in [
        (State::Info, "info"),
        (State::Success, "success"),
        (State::Warning, "warning"),
        (State::Danger, "danger"),
    ] {
        let mut opener = Fake::ok();
        notify(&mut opener, PLUGIN, &Dialog::new(state, "t", "b"));
        assert_eq!(
            opener.only().env.get(STATE_VAR).map(String::as_str),
            Some(word),
            "{:?} did not travel as {}",
            state,
            word
        );
    }
}

// ── ui_busy, which is an outcome rather than an error ─────────────────────

#[test]
fn the_measured_busy_code_is_classified_and_nothing_else_is() {
    assert_eq!(
        OpenError::from_error(BUSY_CODE, BUSY_MESSAGE),
        OpenError::Busy
    );
    assert_ne!(
        OpenError::from_error("plugin_not_found", "plugin not found"),
        OpenError::Busy
    );
    assert_ne!(
        OpenError::from_error("invalid_params", "bad"),
        OpenError::Busy
    );
    // Near misses, because a prefix test is not a name test.
    for near in ["ui_bus", "ui_busyy", "UI_BUSY", "busy"] {
        assert_ne!(
            OpenError::from_error(near, BUSY_MESSAGE),
            OpenError::Busy,
            "{} was read as the busy code",
            near
        );
    }
}

#[test]
fn a_failed_open_keeps_what_herdr_said_about_it() {
    let error = OpenError::from_error("plugin_not_found", "plugin not found");
    let OpenError::Failed(why) = error else {
        panic!("a non-busy code should not classify as busy");
    };
    assert!(
        why.contains("plugin_not_found"),
        "the code was dropped: {}",
        why
    );
    assert!(
        why.contains("plugin not found"),
        "the message was dropped: {}",
        why
    );
}

#[test]
fn a_bare_dialog_that_lost_the_race_says_so_rather_than_showing_nothing() {
    // 🔑 The defect SCOPE.md §7.2 was written to fix, in its dialog form: a
    // dropped message and a delivered one must not be indistinguishable to the
    // caller, from information the caller already received.
    assert_eq!(notify(&mut Fake::busy(), PLUGIN, &dialog()), Shown::Busy);
    assert_eq!(notify(&mut Fake::ok(), PLUGIN, &dialog()), Shown::Opened);
    assert_eq!(
        notify(&mut Fake::failing(), PLUGIN, &dialog()),
        Shown::Failed("socket is gone".to_string())
    );
}

#[test]
fn a_question_that_lost_the_race_is_reported_as_busy_and_not_as_a_refusal() {
    let answer = ask(&mut Fake::busy(), PLUGIN, &dialog(), &buttons());
    assert_eq!(answer, Answer::Unanswered(Unanswered::Busy));
    // The distinction that matters: busy is not the user cancelling.
    assert_ne!(answer, Answer::Cancel);
    assert!(!answer.chose_primary());
}

#[test]
fn a_busy_dialog_is_never_confused_with_one_that_failed_to_open() {
    assert_ne!(
        ask(&mut Fake::busy(), PLUGIN, &dialog(), &buttons()),
        ask(&mut Fake::failing(), PLUGIN, &dialog(), &buttons())
    );
    assert_ne!(
        notify(&mut Fake::busy(), PLUGIN, &dialog()),
        notify(&mut Fake::failing(), PLUGIN, &dialog())
    );
}

// ── Waiting, and refusing to wait ─────────────────────────────────────────

#[test]
fn a_bare_dialog_never_waits_for_anything() {
    // 🔑 SCOPE.md §7.4: a cosmetic warning must not make somebody wait on a
    // dialog to get their workspace. A notify that built a channel would sit
    // through the three-second startup window before answering.
    let started = Instant::now();
    notify(&mut Fake::ok(), PLUGIN, &dialog());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a bare dialog waited {:?}",
        started.elapsed()
    );
}

#[test]
fn a_question_that_could_not_be_opened_gives_up_at_once() {
    // The failure paths must not wait either. Only a popup that really opened
    // is worth waiting on.
    for mut opener in [Fake::busy(), Fake::failing()] {
        let started = Instant::now();
        ask(&mut opener, PLUGIN, &dialog(), &buttons());
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "a dialog that never opened waited {:?}",
            started.elapsed()
        );
    }
}

#[test]
fn a_popup_that_answers_is_heard_across_the_channel() {
    // End to end through the real files: the marker, the poll, and the answer.
    for (word, expected) in [
        (PRIMARY_WORD, Answer::Primary),
        (CANCEL_WORD, Answer::Cancel),
    ] {
        let mut opener = Answering {
            word: word.to_string(),
            marks_started: true,
            delay: Duration::from_millis(50),
        };
        assert_eq!(ask(&mut opener, PLUGIN, &dialog(), &buttons()), expected);
    }
}

#[test]
fn a_popup_answering_a_word_neither_button_uses_does_not_choose_either() {
    // A popup left over from an older build is the real case. It must not fall
    // through to the primary button.
    let mut opener = Answering {
        word: "rebuild".to_string(),
        marks_started: true,
        delay: Duration::from_millis(20),
    };
    let answer = ask(&mut opener, PLUGIN, &dialog(), &buttons());
    assert_eq!(
        answer,
        Answer::Unanswered(Unanswered::Unrecognised("rebuild".to_string()))
    );
    assert!(!answer.chose_primary());
}

#[test]
fn an_answer_arriving_before_the_marker_is_still_heard() {
    // ✅ `plugin.pane.open` answers `ok` whether or not a process starts, so
    // the marker is the only evidence a popup drew. A fast popup that answers
    // and exits inside the startup window writes both, and the answer is what
    // decides.
    let mut opener = Answering {
        word: PRIMARY_WORD.to_string(),
        marks_started: false,
        delay: Duration::from_millis(20),
    };
    assert_eq!(
        ask(&mut opener, PLUGIN, &dialog(), &buttons()),
        Answer::Primary
    );
}

#[test]
fn only_an_explicit_primary_answer_ever_chooses_the_primary_button() {
    // The safety property gathered into one place. Every outcome except the
    // one exact word must answer false, because the primary button is where a
    // caller puts the destructive action.
    assert!(!ask(&mut Fake::busy(), PLUGIN, &dialog(), &buttons()).chose_primary());
    assert!(!ask(&mut Fake::failing(), PLUGIN, &dialog(), &buttons()).chose_primary());
    assert!(!Answer::Cancel.chose_primary());
    for why in [
        Unanswered::Busy,
        Unanswered::Failed("x".to_string()),
        Unanswered::Dismissed,
        Unanswered::NeverShown,
        Unanswered::TimedOut,
        Unanswered::Unrecognised(PRIMARY_WORD.to_string()),
    ] {
        assert!(
            !Answer::Unanswered(why.clone()).chose_primary(),
            "{:?} chose the primary button",
            why
        );
    }
    assert!(Answer::Primary.chose_primary());
}

// ── The popup half reading its own environment ────────────────────────────

#[test]
fn the_primary_labels_absence_is_what_makes_a_dialog_bare() {
    let bare = Popup::from_env(&Environment::from_pairs(&[
        (STATE_VAR, "danger"),
        (TITLE_VAR, "Stop"),
        (BODY_VAR, "It broke."),
    ]));
    assert_eq!(bare.buttons, None);
    assert_eq!(bare.dialog.state, State::Danger);
    assert_eq!(bare.dialog.title, "Stop");
    assert_eq!(bare.dialog.body, "It broke.");

    let actioned = Popup::from_env(&Environment::from_pairs(&[
        (STATE_VAR, "danger"),
        (PRIMARY_VAR, "Delete"),
        (CANCEL_VAR, "Keep"),
    ]));
    assert_eq!(
        actioned.buttons,
        Some(Buttons::new("Delete", "Keep")),
        "a primary label should have produced buttons"
    );
}

#[test]
fn a_variable_set_but_empty_counts_as_absent() {
    // The idiom `crate::env` documents at nearly every call site. Herdr does
    // inject empty values.
    let popup = Popup::from_env(&Environment::from_pairs(&[
        (PRIMARY_VAR, ""),
        (TITLE_VAR, ""),
        (ANSWER_FILE_VAR, ""),
        (STARTED_FILE_VAR, ""),
    ]));
    assert_eq!(popup.buttons, None, "an empty label made a button");
    assert!(popup.dialog.title.is_empty());
    assert_eq!(popup.answer_file, None);
    assert_eq!(popup.started_file, None);
}

#[test]
fn a_cancel_label_defaults_rather_than_drawing_an_empty_button() {
    let popup = Popup::from_env(&Environment::from_pairs(&[(PRIMARY_VAR, "Go")]));
    assert_eq!(
        popup.buttons.map(|b| b.cancel),
        Some(DEFAULT_CANCEL.to_string())
    );
    assert_eq!(Buttons::new("Go", "   ").cancel, DEFAULT_CANCEL);
}

#[test]
fn an_absent_state_informs_and_an_unrecognised_one_warns() {
    // 🔑 The two absences fail in different directions. Nothing asked for means
    // nothing is wrong. A word neither half recognises means the two halves
    // disagree, and understating a dialog is the worse error.
    assert_eq!(State::from_wire(None), State::Info);
    assert_eq!(State::from_wire(Some("")), State::Info);
    assert_eq!(State::from_wire(Some("   ")), State::Info);
    assert_eq!(State::from_wire(Some("catastrophe")), State::Warning);
    assert_eq!(State::from_wire(Some("INFO")), State::Warning);
}

#[test]
fn every_state_survives_the_round_trip_through_the_environment() {
    for state in [State::Info, State::Success, State::Warning, State::Danger] {
        assert_eq!(State::from_wire(Some(state.as_wire())), state);
    }
}

// ── What the frame looks like ─────────────────────────────────────────────

#[test]
fn every_row_is_exactly_as_wide_as_the_frame() {
    // The alignment property. A mis-counted glyph, a mis-padded body line, or a
    // mis-centred button row each break exactly this.
    for width in [24, 30, 53, 80] {
        for state in [State::Info, State::Success, State::Warning, State::Danger] {
            let dialog = Dialog::new(state, "A title", "Some body text that wraps a little.");
            for buttons in [None, Some(buttons())] {
                let drawn = render(&dialog, buttons.as_ref(), width);
                for line in plain(&drawn).lines() {
                    assert_eq!(
                        width_of(line),
                        width,
                        "{:?} at {} drew a {}-cell row: {:?}",
                        state,
                        width,
                        width_of(line),
                        line
                    );
                }
            }
        }
    }
}

#[test]
fn the_frame_is_rounded_on_all_four_states() {
    // ⚠️ Varying the corner for danger was proposed and rejected: the glyph is
    // already the non-colour channel, so a second one costs consistency.
    for state in [State::Info, State::Success, State::Warning, State::Danger] {
        let drawn = plain(&render(&Dialog::new(state, "t", "b"), None, 30));
        let lines: Vec<&str> = drawn.lines().collect();
        assert!(lines[0].starts_with('╭'), "{:?} top-left", state);
        assert!(lines[0].ends_with('╮'), "{:?} top-right", state);
        assert!(
            lines[lines.len() - 1].starts_with('╰'),
            "{:?} bottom-left",
            state
        );
        assert!(
            lines[lines.len() - 1].ends_with('╯'),
            "{:?} bottom-right",
            state
        );
    }
}

#[test]
fn each_state_draws_its_own_glyph_in_the_title_line() {
    // 🔑 The glyph is what survives a monochrome terminal, where colour alone
    // cannot separate a warning from a danger.
    let mut seen = Vec::new();
    for state in [State::Info, State::Success, State::Warning, State::Danger] {
        let drawn = plain(&render(&Dialog::new(state, "Title", "b"), None, 40));
        let title = drawn.lines().next().unwrap();
        assert!(
            title.contains(state.glyph()),
            "{:?} drew no glyph: {:?}",
            state,
            title
        );
        assert!(title.contains("Title"), "{:?} dropped the title", state);
        seen.push(state.glyph());
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        4,
        "two states share a glyph, so one is unreadable"
    );
}

#[test]
fn each_state_draws_its_own_colour() {
    let mut seen = Vec::new();
    for state in [State::Info, State::Success, State::Warning, State::Danger] {
        let drawn = render(&Dialog::new(state, "t", "b"), None, 30);
        let code = format!("\u{1b}[{}m", state.colour());
        assert!(drawn.contains(&code), "{:?} drew no {} colour", state, code);
        seen.push(state.colour());
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 4, "two states share a colour");
}

#[test]
fn the_padding_is_two_blank_rows_top_and_three_columns_each_side() {
    let dialog = Dialog::new(State::Info, "T", "Body");
    let drawn = plain(&render(&dialog, None, 40));
    let lines: Vec<&str> = drawn.lines().collect();

    let blank = format!("│{}│", " ".repeat(38));
    assert_eq!(lines[1], blank, "first top padding row");
    assert_eq!(lines[2], blank, "second top padding row");

    assert!(
        lines[3].starts_with("│   Body"),
        "three columns of side padding: {:?}",
        lines[3]
    );
    assert!(lines[3].ends_with("   │"), "right padding: {:?}", lines[3]);
}

#[test]
fn the_bottom_padding_is_two_rows_except_below_a_button_row() {
    // 🔑 Decided by Mike: the button row is already visually heavy enough not to
    // need the extra breathing space beneath it. Nothing else changes, so the
    // top of both variants and the bottom of the bare one keep two rows.
    let dialog = Dialog::new(State::Info, "T", "Body");
    let blank = format!("│{}│", " ".repeat(38));

    let bare = plain(&render(&dialog, None, 40));
    let bare: Vec<&str> = bare.lines().collect();
    assert_eq!(bare[bare.len() - 2], blank, "bare, second bottom row");
    assert_eq!(bare[bare.len() - 3], blank, "bare, first bottom row");

    let actioned = plain(&render(&dialog, Some(&buttons()), 40));
    let actioned: Vec<&str> = actioned.lines().collect();
    assert_eq!(
        actioned[actioned.len() - 2],
        blank,
        "actioned, the one bottom row"
    );
    assert!(
        actioned[actioned.len() - 3].contains("Rebuild"),
        "a second blank row survives below the buttons: {:?}",
        actioned[actioned.len() - 3]
    );
}

#[test]
fn a_blank_row_separates_the_body_from_the_buttons() {
    // Without it the buttons read as another line of body text.
    let dialog = Dialog::new(State::Info, "T", "Body");
    let drawn = plain(&render(&dialog, Some(&buttons()), 40));
    let lines: Vec<&str> = drawn.lines().collect();
    let body = lines.iter().position(|l| l.contains("Body")).unwrap();
    let row = lines.iter().position(|l| l.contains("Rebuild")).unwrap();
    assert_eq!(row, body + 2, "the buttons sit against the body");
    assert_eq!(lines[body + 1].trim_matches(['│', ' ']), "", "no separator");
}

#[test]
fn a_bare_dialog_draws_no_buttons_and_no_separator() {
    let dialog = Dialog::new(State::Info, "T", "Body");
    let bare = plain(&render(&dialog, None, 40));
    let actioned = plain(&render(&dialog, Some(&buttons()), 40));
    assert!(!bare.contains("Rebuild"), "a bare dialog drew a button");
    assert!(!bare.contains("Leave it"), "a bare dialog drew a cancel");
    // The separator and the button row add two rows, and the bottom padding
    // gives one back, so an actioned dialog is exactly one row taller.
    assert_eq!(
        actioned.lines().count(),
        bare.lines().count() + 1,
        "the button row and its separator are not both present"
    );
}

#[test]
fn the_primary_button_is_inverted_and_the_cancel_is_plain() {
    // Mike's design: the state's colour as the background with the text
    // inverted against it, and the cancel as plain text.
    let drawn = render(&Dialog::new(State::Danger, "T", "B"), Some(&buttons()), 40);
    let row = drawn
        .lines()
        .find(|line| line.contains("Rebuild"))
        .expect("no button row");
    let inverted = row
        .split("\u{1b}[7m")
        .nth(1)
        .expect("the primary button is not inverted");
    assert!(
        inverted.starts_with(&format!(" {} Rebuild ", PRIMARY_KEY)),
        "the inversion does not cover the primary key and label: {:?}",
        inverted
    );
    assert!(
        !row.split("Leave it").nth(1).unwrap().contains("\u{1b}[7m"),
        "inversion leaked past the primary button"
    );
}

#[test]
fn a_long_body_wraps_inside_the_padding_rather_than_overflowing() {
    let body = "one two three four five six seven eight nine ten eleven twelve";
    let dialog = Dialog::new(State::Info, "T", body);
    let drawn = plain(&render(&dialog, None, 30));
    assert!(
        drawn.lines().count() > 6,
        "a body that cannot fit one row did not wrap"
    );
    for word in body.split_whitespace() {
        assert!(drawn.contains(word), "{} was dropped by the wrap", word);
    }
}

#[test]
fn a_title_too_long_for_the_border_is_truncated_rather_than_breaking_the_frame() {
    let long = "a title far longer than any frame this narrow could ever hold";
    let drawn = plain(&render(&Dialog::new(State::Info, long, "b"), None, 30));
    let title = drawn.lines().next().unwrap();
    assert_eq!(width_of(title), 30);
    assert!(
        title.ends_with('╮'),
        "the frame lost its corner: {:?}",
        title
    );
    assert!(!title.contains("ever hold"), "the title was not truncated");
}

#[test]
fn a_width_below_the_floor_is_raised_rather_than_drawn_wrong() {
    // Below the floor the borders, the padding and a glyph collide. A frame
    // that cannot be drawn correctly is worse than one wider than asked.
    for asked in [0, 1, 10, 23] {
        let drawn = plain(&render(&dialog(), Some(&buttons()), asked));
        for line in drawn.lines() {
            assert_eq!(
                width_of(line),
                24,
                "a {}-cell request drew {:?}",
                asked,
                line
            );
        }
    }
}

#[test]
fn labels_too_long_for_the_frame_are_shortened_rather_than_breaking_it() {
    // ⚠️ Labels are the caller's, so no frame width makes them safe. A long
    // action name in a narrow terminal would push the button row through the
    // right border and break every row's alignment at once.
    let long = Buttons::new(
        "Rebuild every tab in this workspace",
        "Leave everything exactly as it is",
    );
    for width in [24, 30, 40, 60] {
        let drawn = plain(&render(&dialog(), Some(&long), width));
        for line in drawn.lines() {
            assert_eq!(width_of(line), width, "at {}: {:?}", width, line);
        }
    }

    // Both survive as something readable rather than one being erased.
    let drawn = plain(&render(&dialog(), Some(&long), 40));
    let row = drawn
        .lines()
        .find(|line| line.contains("Rebuild"))
        .expect("the primary label was erased");
    assert!(
        row.contains("Leave"),
        "the cancel label was erased: {:?}",
        row
    );

    // A pair that already fits is left exactly as the caller wrote it.
    let short = Buttons::new("Go", "Stop");
    let fits = plain(&render(&dialog(), Some(&short), 40));
    assert!(fits.contains(" Go "), "a fitting label was truncated");
    assert!(fits.contains("Stop"), "a fitting label was truncated");
}

// ── The mouse ─────────────────────────────────────────────────────────────

#[test]
fn the_buttons_report_where_they_were_actually_drawn() {
    // ✅ The coordinates a click arrives carrying are pane-local, and the frame
    // is painted from the pane's origin, so a reported rectangle has to cover
    // exactly the cells the button really occupies, key affordance included.
    for width in [24, 40, 60, 80] {
        let frame = layout(&dialog(), Some(&buttons()), width, Hot::None);
        let drawn = plain(&frame.text);
        let lines: Vec<&str> = drawn.lines().collect();
        let primary = frame.primary.expect("no primary rectangle");
        let cancel = frame.cancel.expect("no cancel rectangle");

        assert_eq!(primary.row, cancel.row, "the buttons are on different rows");
        let row: Vec<char> = lines[primary.row as usize].chars().collect();
        let span = |r: Rect| -> String {
            row[r.column as usize..(r.column + r.width) as usize]
                .iter()
                .collect()
        };

        assert!(
            span(primary).starts_with(&format!(" {}", PRIMARY_KEY)),
            "the primary rectangle covers {:?} at width {}",
            span(primary),
            width
        );
        assert!(
            span(cancel).starts_with(CANCEL_KEY),
            "the cancel rectangle covers {:?} at width {}",
            span(cancel),
            width
        );
        // The rectangles must not overlap, or one button would swallow clicks
        // meant for the other.
        assert!(
            primary.column + primary.width <= cancel.column,
            "the rectangles overlap at width {}",
            width
        );
    }

    // Exactly, where the frame is wide enough for both labels in full.
    let frame = layout(&dialog(), Some(&buttons()), 60, Hot::None);
    let drawn = plain(&frame.text);
    let row: Vec<char> = drawn
        .lines()
        .nth(frame.primary.unwrap().row as usize)
        .unwrap()
        .chars()
        .collect();
    let span = |r: Rect| -> String {
        row[r.column as usize..(r.column + r.width) as usize]
            .iter()
            .collect()
    };
    assert_eq!(
        span(frame.primary.unwrap()),
        format!(" {} Rebuild ", PRIMARY_KEY)
    );
    assert_eq!(
        span(frame.cancel.unwrap()),
        format!("{} Leave it", CANCEL_KEY)
    );
}

#[test]
fn the_kit_draws_each_key_and_the_caller_never_types_one() {
    // 🔑 Drift, not convenience. Three plugins typing their own glyph would
    // eventually disagree about the symbol, the spacing, or whether to bother.
    let drawn = plain(&render(
        &dialog(),
        Some(&Buttons::new("close anyway", "keep")),
        60,
    ));
    assert!(
        drawn.contains(&format!("{} close anyway", PRIMARY_KEY)),
        "the primary key was not drawn: {:?}",
        drawn
    );
    assert!(
        drawn.contains(&format!("{} keep", CANCEL_KEY)),
        "the cancel key was not drawn: {:?}",
        drawn
    );
    // The caller's label is kept verbatim, so only the prefix is the kit's.
    assert!(drawn.contains("close anyway"));
    assert!(drawn.contains("keep"));
}

#[test]
fn a_click_on_a_button_is_that_button_and_a_click_anywhere_else_is_neither() {
    // ⚠️ Clicking the body must resolve nothing. A misclick that fired a
    // primary button whose action is destructive is the failure this prevents.
    let frame = layout(&dialog(), Some(&buttons()), 60, Hot::None);
    let primary = frame.primary.unwrap();
    let cancel = frame.cancel.unwrap();

    assert_eq!(frame.hit(primary.column, primary.row), Hot::Primary);
    assert_eq!(
        frame.hit(primary.column + primary.width - 1, primary.row),
        Hot::Primary
    );
    assert_eq!(frame.hit(cancel.column, cancel.row), Hot::Cancel);
    assert_eq!(
        frame.hit(cancel.column + cancel.width - 1, cancel.row),
        Hot::Cancel
    );

    // One cell outside each edge, and the gap between the two.
    assert_eq!(frame.hit(primary.column - 1, primary.row), Hot::None);
    assert_eq!(
        frame.hit(primary.column + primary.width, primary.row),
        Hot::None
    );
    assert_eq!(
        frame.hit(cancel.column + cancel.width, cancel.row),
        Hot::None
    );
    // The row above and the row below hold no buttons at all.
    assert_eq!(frame.hit(primary.column, primary.row - 1), Hot::None);
    assert_eq!(frame.hit(primary.column, primary.row + 1), Hot::None);
    // The title, the body, and the far corner.
    assert_eq!(frame.hit(0, 0), Hot::None);
    assert_eq!(frame.hit(5, 3), Hot::None);
    assert_eq!(frame.hit(59, 0), Hot::None);
}

#[test]
fn a_bare_dialog_offers_nothing_to_click() {
    let frame = layout(&dialog(), None, 40, Hot::None);
    assert_eq!(frame.primary, None);
    assert_eq!(frame.cancel, None);
    // Every position on a bare dialog is neither button, so the caller's
    // "any click dismisses" rule can never be overridden by a hit.
    for row in 0..frame.text.lines().count() as u16 {
        for column in [0, 5, 20, 39] {
            assert_eq!(frame.hit(column, row), Hot::None);
        }
    }
}

#[test]
fn hover_changes_attributes_and_never_geometry() {
    // 🔑 The invariant that makes a hover repaint safe. Every redraw paints from
    // the pane's origin over the previous frame, so if the pointer could move a
    // cell the frame would accumulate rather than replace.
    let reference = layout(&dialog(), Some(&buttons()), 60, Hot::None);
    for hot in [Hot::None, Hot::Primary, Hot::Cancel] {
        let frame = layout(&dialog(), Some(&buttons()), 60, hot);
        assert_eq!(
            plain(&frame.text),
            plain(&reference.text),
            "{:?} moved a character",
            hot
        );
        assert_eq!(
            frame.primary, reference.primary,
            "{:?} moved the primary",
            hot
        );
        assert_eq!(frame.cancel, reference.cancel, "{:?} moved the cancel", hot);
    }
}

#[test]
fn only_the_hovered_button_is_underlined() {
    let underline = "\u{1b}[4m";

    let none = layout(&dialog(), Some(&buttons()), 60, Hot::None);
    assert!(
        !none.text.contains(underline),
        "nothing hovered, yet underlined"
    );

    let primary = layout(&dialog(), Some(&buttons()), 60, Hot::Primary);
    let row = primary
        .text
        .lines()
        .find(|line| line.contains("Rebuild"))
        .unwrap();
    assert_eq!(
        row.matches(underline).count(),
        1,
        "not exactly one underline"
    );
    assert!(
        row.split("Rebuild").next().unwrap().contains(underline),
        "the underline is not on the primary button"
    );

    let cancel = layout(&dialog(), Some(&buttons()), 60, Hot::Cancel);
    let row = cancel
        .text
        .lines()
        .find(|line| line.contains("Leave it"))
        .unwrap();
    assert_eq!(
        row.matches(underline).count(),
        1,
        "not exactly one underline"
    );
    assert!(
        row.split(CANCEL_KEY).next().unwrap().ends_with(underline),
        "the underline is not on the cancel affordance: {:?}",
        row
    );
}

#[test]
fn an_empty_primary_label_draws_its_key_alone_and_keeps_the_frame_square() {
    // Reachable from the caller rather than from truncation: `Buttons::new`
    // defaults an empty cancel label but not an empty primary one, so a caller
    // can hand one in. The key must then stand alone, because a separator with
    // nothing after it pads the inversion with a cell of nothing.
    let buttons = Buttons::new("", "keep");
    for width in [24, 40, 60] {
        let drawn = plain(&render(&dialog(), Some(&buttons), width));
        for line in drawn.lines() {
            assert_eq!(width_of(line), width, "at {}: {:?}", width, line);
        }
    }
    // The reported width is what discriminates. A separator that survived an
    // empty label would make the button one cell wider, and the surrounding
    // gap makes that invisible to any search of the drawn text.
    let frame = layout(&dialog(), Some(&buttons), 60, Hot::None);
    let primary = frame.primary.expect("no primary rectangle");
    assert_eq!(
        primary.width,
        (cells_of(PRIMARY_KEY) + 2) as u16,
        "a separator survived an empty label"
    );

    let drawn = plain(&frame.text);
    let row: Vec<char> = drawn
        .lines()
        .nth(primary.row as usize)
        .unwrap()
        .chars()
        .collect();
    let span: String = row[primary.column as usize..(primary.column + primary.width) as usize]
        .iter()
        .collect();
    assert_eq!(
        span,
        format!(" {} ", PRIMARY_KEY),
        "the key did not stand alone"
    );
}

/// Counts characters, independently of the module's own counter.
fn cells_of(text: &str) -> usize {
    text.chars().count()
}

#[test]
fn an_empty_body_still_draws_a_complete_frame() {
    let drawn = plain(&render(&Dialog::new(State::Info, "", ""), None, 24));
    let lines: Vec<&str> = drawn.lines().collect();
    assert_eq!(
        lines.len(),
        7,
        "top, two blanks, one body row, two blanks, bottom"
    );
    for line in &lines {
        assert_eq!(width_of(line), 24, "{:?}", line);
    }
}
