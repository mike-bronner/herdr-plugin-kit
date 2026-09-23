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

use herdr_plugin_kit::api::generated::{
    NotificationShowParams, NotificationShowReason, PluginPaneOpenParams, PluginPanePlacement,
};
use herdr_plugin_kit::dialog::{
    ask, button_var, layout, notify, render, Answer, Button, Dialog, Explained, Key, OpenError,
    Popup, Rect, Shown, State, Transport, Unanswered, ANSWER_FILE_VAR, BODY_VAR, BUSY_CODE,
    BUTTON_VAR, DISMISSED_WORD, ENTER_KEY, ENTRYPOINT, ESCAPE_KEY, HEIGHT, STARTED_FILE_VAR,
    STATE_VAR, TITLE_VAR, WIDTH,
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
    /// What `notification.show` answers, or `None` to make it unreachable.
    notification: Option<NotificationShowReason>,
    seen: Vec<PluginPaneOpenParams>,
    notified: Vec<NotificationShowParams>,
}

impl Fake {
    fn new(answer: Result<(), OpenError>) -> Fake {
        Fake {
            answer,
            notification: Some(NotificationShowReason::Shown),
            seen: Vec::new(),
            notified: Vec::new(),
        }
    }

    fn ok() -> Fake {
        Fake::new(Ok(()))
    }

    fn busy() -> Fake {
        Fake::new(Err(OpenError::from_error(BUSY_CODE, BUSY_MESSAGE)))
    }

    fn failing() -> Fake {
        Fake::new(Err(OpenError::Failed("socket is gone".to_string())))
    }

    /// A busy popup whose notification answers `reason`.
    fn busy_notifying(reason: NotificationShowReason) -> Fake {
        let mut fake = Fake::busy();
        fake.notification = Some(reason);
        fake
    }

    /// A busy popup whose notification cannot be sent at all.
    fn busy_unreachable() -> Fake {
        let mut fake = Fake::busy();
        fake.notification = None;
        fake
    }

    fn only(&self) -> &PluginPaneOpenParams {
        assert_eq!(self.seen.len(), 1, "expected exactly one request");
        &self.seen[0]
    }
}

impl Transport for Fake {
    fn open_pane(&mut self, params: PluginPaneOpenParams) -> Result<(), OpenError> {
        self.seen.push(params);
        self.answer.clone()
    }

    fn show_notification(
        &mut self,
        params: NotificationShowParams,
    ) -> Result<NotificationShowReason, String> {
        self.notified.push(params);
        self.notification
            .ok_or_else(|| "the socket is gone".to_string())
    }
}

/// An opener that behaves like the popup process: marker first, then an answer.
struct Answering {
    word: String,
    /// `false` skips the marker, which is what a popup that never drew looks like.
    marks_started: bool,
    delay: Duration,
}

impl Transport for Answering {
    fn open_pane(&mut self, params: PluginPaneOpenParams) -> Result<(), OpenError> {
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

    fn show_notification(
        &mut self,
        _params: NotificationShowParams,
    ) -> Result<NotificationShowReason, String> {
        panic!("a popup that opened must never fall back to a notification");
    }
}

fn dialog() -> Dialog {
    Dialog::new(State::Warning, "Careful", "Two panes are running agents.")
}

/// The two buttons most tests ask with, naming no key.
fn buttons() -> Vec<Button> {
    vec![Button::new("Rebuild"), Button::new("Leave it")]
}

/// Three buttons, for the behaviour a pair cannot show.
fn three() -> Vec<Button> {
    vec![
        Button::new("Delete"),
        Button::new("Keep"),
        Button::new("Archive"),
    ]
}

/// The lists every wire and drawing test sweeps.
///
/// Every pair the round-3 design could express, then lists of three and four
/// where the ladder reaches past Escape and a later button names Enter.
fn configurations() -> Vec<Vec<Button>> {
    let pair = |first: Button, second: Button| vec![first, second];
    vec![
        buttons(),
        pair(
            Button::new("Rebuild").on_key(Key::Char('y')),
            Button::new("Leave it"),
        ),
        pair(
            Button::new("Rebuild"),
            Button::new("Leave it").on_key(Key::Char('n')),
        ),
        pair(
            Button::new("Rebuild").on_key(Key::Char('y')),
            Button::new("Leave it").on_key(Key::Char('n')),
        ),
        pair(
            Button::new("Rebuild").on_key(Key::Char('y')),
            Button::new("Leave it").on_key(Key::Enter),
        ),
        three(),
        vec![
            Button::new("Delete").on_key(Key::Char('d')),
            Button::new("Keep them all").on_key(Key::Enter),
            Button::new("Archive"),
            Button::new(""),
        ],
    ]
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

/// The characters a rectangle covers on the frame it was reported for.
fn span(frame: &str, rect: Rect) -> String {
    plain(frame)
        .lines()
        .nth(rect.row as usize)
        .expect("a rectangle on a row the frame does not have")
        .chars()
        .skip(rect.column as usize)
        .take(rect.width as usize)
        .collect()
}

/// Reads an environment back the way the popup half does.
fn read_back(env: &std::collections::HashMap<String, String>) -> Popup {
    let pairs: Vec<(&str, &str)> = env
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    Popup::from_env(&Environment::from_pairs(&pairs))
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
    for absent in [ANSWER_FILE_VAR, STARTED_FILE_VAR] {
        assert!(
            !env.contains_key(absent),
            "{} was sent to a bare dialog",
            absent
        );
    }
    // A bare dialog dismisses on any key, so it sends no button at all.
    assert!(
        !env.keys().any(|key| key.starts_with(BUTTON_VAR)),
        "a button was sent to a bare dialog: {:?}",
        env.keys().collect::<Vec<_>>()
    );
    assert_eq!(env.get(STATE_VAR).map(String::as_str), Some("warning"));
    assert_eq!(env.get(TITLE_VAR).map(String::as_str), Some("Careful"));
    assert_eq!(
        env.get(BODY_VAR).map(String::as_str),
        Some("Two panes are running agents.")
    );
}

#[test]
fn an_actioned_dialog_carries_both_files_and_every_button() {
    let mut opener = Fake::ok();
    ask(&mut opener, PLUGIN, &dialog(), &three());
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
    // One variable per button, each carrying exactly what that button draws.
    assert_eq!(
        env.get(&button_var(0)),
        Some(&format!("{} Delete", ENTER_KEY))
    );
    assert_eq!(
        env.get(&button_var(1)),
        Some(&format!("{} Keep", ESCAPE_KEY))
    );
    assert_eq!(
        env.get(&button_var(2)).map(String::as_str),
        Some("! Archive")
    );
    // 🔑 The list ends at the first absent index, so nothing past the last
    // button may be sent.
    assert!(
        !env.contains_key(&button_var(3)),
        "a fourth button was sent"
    );
}

#[test]
fn a_button_variable_is_the_prefix_and_its_index() {
    assert_eq!(button_var(0), format!("{}0", BUTTON_VAR));
    assert_eq!(button_var(12), "HERDR_PLUGIN_DIALOG_BUTTON_12");
}

#[test]
fn a_named_key_travels_as_what_the_frame_draws() {
    let mut opener = Fake::ok();
    ask(
        &mut opener,
        PLUGIN,
        &dialog(),
        &[
            Button::new("Rebuild").on_key(Key::Char('y')),
            Button::new("Leave it").on_key(Key::Enter),
        ],
    );
    let env = &opener.only().env;
    assert_eq!(
        env.get(&button_var(0)).map(String::as_str),
        Some("y Rebuild")
    );
    assert_eq!(
        env.get(&button_var(1)),
        Some(&format!("{} Leave it", ENTER_KEY)),
        "the popup was not told that Enter answers the second button"
    );
}

#[test]
fn a_naming_that_cannot_be_honoured_travels_as_what_it_became() {
    // 🔑 The wire carries the resolved key, never the named one, so the popup
    // half is told what the naming actually became rather than resolving it a
    // second time on its own.
    let mut opener = Fake::ok();
    ask(
        &mut opener,
        PLUGIN,
        &dialog(),
        &[
            Button::new("Delete").on_key(Key::Escape),
            Button::new("Keep"),
        ],
    );
    let env = &opener.only().env;
    assert_eq!(
        env.get(&button_var(0)),
        Some(&format!("{} Delete", ENTER_KEY)),
        "Escape travelled on the first button"
    );
    assert_eq!(
        env.get(&button_var(1)),
        Some(&format!("{} Keep", ESCAPE_KEY))
    );
}

#[test]
fn every_configuration_survives_the_round_trip_through_the_environment() {
    // The popup half is a separate process, so a naming that does not survive the
    // environment is one only the asking half believes in. Labels with spaces
    // and an empty label are in the sweep, because the first space is the split.
    for buttons in configurations() {
        let mut opener = Fake::ok();
        ask(&mut opener, PLUGIN, &dialog(), &buttons);
        let popup = read_back(&opener.only().env);
        assert_eq!(
            Button::keys(&popup.buttons),
            Button::keys(&buttons),
            "{:?} did not reach the popup half",
            Button::keys(&buttons)
        );
        let labels = |list: &[Button]| -> Vec<String> {
            list.iter().map(|button| button.label.clone()).collect()
        };
        assert_eq!(labels(&popup.buttons), labels(&buttons));
    }
}

#[test]
fn a_list_longer_than_the_ladder_sends_only_the_buttons_it_could_key() {
    // ⚠️ A button no key answers is drawn nowhere, so the popup is never told
    // about it either. 70 is how many keys the ladder can tell apart.
    let many: Vec<Button> = (0..80).map(|n| Button::new(&format!("b{}", n))).collect();
    let mut opener = Fake::ok();
    ask(&mut opener, PLUGIN, &dialog(), &many);
    let env = &opener.only().env;
    assert!(
        env.contains_key(&button_var(69)),
        "a keyed button was dropped"
    );
    assert!(
        !env.contains_key(&button_var(70)),
        "a button no key answers was sent"
    );
    assert_eq!(read_back(env).buttons.len(), 70);
}

#[test]
fn a_variable_naming_no_key_leaves_that_button_to_the_ladder() {
    let read = |pairs: &[(&str, &str)]| {
        Button::keys(&Popup::from_env(&Environment::from_pairs(pairs)).buttons)
    };
    let (first, second) = (button_var(0), button_var(1));

    // ⚠️ Only the asking half writes these, and it writes what the frame draws. A
    // value outside that vocabulary is a foreign environment, and one rule points
    // the same way at every button rather than two rules wearing one name.
    assert_eq!(
        read(&[(&first, "yes Delete"), (&second, "\u{4f60} Keep")]),
        vec![Key::Enter, Key::Escape]
    );
    // And a value that names a key is honoured, so the assertion above is about
    // the vocabulary rather than about the key being ignored.
    assert_eq!(
        read(&[(&first, "d Delete"), (&second, "k Keep")]),
        vec![Key::Char('d'), Key::Char('k')]
    );
    // The label is everything after the first space, spaces and all.
    let popup = Popup::from_env(&Environment::from_pairs(&[(&first, "d close  it now ")]));
    assert_eq!(popup.buttons[0].label, "close  it now ");
}

#[test]
fn the_frame_draws_each_button_with_the_key_that_answers_it() {
    let drawn = |buttons: &[Button]| plain(&render(&dialog(), buttons, 40));

    // The default frame is exactly what it was before any key was nameable.
    let default = drawn(&buttons());
    assert!(default.contains(&format!("{} Rebuild", ENTER_KEY)));
    assert!(default.contains(&format!("{} Leave it", ESCAPE_KEY)));

    // A named key on the first replaces the Enter glyph, and the untouched
    // button keeps its own default.
    let first_named = drawn(&[
        Button::new("Rebuild").on_key(Key::Char('y')),
        Button::new("Leave it"),
    ]);
    assert!(first_named.contains("y Rebuild"), "{}", first_named);
    assert!(
        !first_named.contains(&format!("{} Rebuild", ENTER_KEY)),
        "the frame still claims Enter reaches the first button:\n{}",
        first_named
    );
    assert!(first_named.contains(&format!("{} Leave it", ESCAPE_KEY)));

    // The second takes the same treatment, Escape included.
    let second_named = drawn(&[
        Button::new("Rebuild"),
        Button::new("Leave it").on_key(Key::Char('n')),
    ]);
    assert!(second_named.contains("n Leave it"), "{}", second_named);
    assert!(
        !second_named.contains(&format!("{} Leave it", ESCAPE_KEY)),
        "the frame still claims Escape reaches the second button:\n{}",
        second_named
    );

    // And Enter named on the second is drawn there, which is the first round's
    // whole requirement with no special case left in the drawing.
    let enter_later = drawn(&[
        Button::new("Rebuild").on_key(Key::Char('y')),
        Button::new("Leave it").on_key(Key::Enter),
    ]);
    assert!(
        enter_later.contains(&format!("{} Leave it", ENTER_KEY)),
        "Enter is not drawn where it answers:\n{}",
        enter_later
    );
    assert!(
        !enter_later.contains(ESCAPE_KEY),
        "Escape is drawn where it answers nothing:\n{}",
        enter_later
    );

    // Past the pair, the third draws the first free character.
    let three = plain(&render(&dialog(), &three(), 60));
    assert!(three.contains("! Archive"), "{}", three);
}

#[test]
fn a_key_is_paid_for_in_cells_rather_than_assumed_free() {
    // ⚠️ Naming a key changes what a button is worth in cells, so the width at
    // which the buttons stop sharing a row has to move with it. A threshold read
    // off a constant would not move at all. ✅ Measured: `↵ Go now` / `esc Stop`
    // share a row from 28 cells, and naming `s` on the second saves the two cells
    // that `esc` cost over `s`, so the same pair shares a row from 26.
    let default = vec![Button::new("Go now"), Button::new("Stop")];
    let named = vec![
        Button::new("Go now"),
        Button::new("Stop").on_key(Key::Char('s')),
    ];
    let shares_a_row = |buttons: &[Button], width| {
        plain(&render(&dialog(), buttons, width))
            .lines()
            .filter(|line| line.contains("Go") || line.contains("Stop"))
            .count()
            == 1
    };

    assert!(
        !shares_a_row(&default, 27),
        "the default pair fits 27 cells"
    );
    assert!(shares_a_row(&default, 28), "the default pair lost 28 cells");
    assert!(
        shares_a_row(&named, 26),
        "a one-cell key was not cheaper than `esc`"
    );
    assert!(!shares_a_row(&named, 25), "the named pair fits 25 cells");

    // Whatever it costs, every row is still exactly as wide as the frame. That is
    // what breaks if a drawn key is counted as fewer cells than it occupies.
    for buttons in [default, named] {
        for width in [26, 28, 40] {
            for line in plain(&render(&dialog(), &buttons, width)).lines() {
                assert_eq!(width_of(line), width, "{:?} drew {:?}", buttons, line);
            }
        }
    }
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
fn a_bare_dialog_that_lost_the_race_falls_back_to_a_notification() {
    // 🔑 A dialog that only informs needs nothing back from the user, so the
    // same message can go by another route. 🚨 That matters more than it looks:
    // the single-popup limit is global rather than per workspace, so a user can
    // sit behind one unanswered dialog indefinitely with nothing on screen to
    // explain it.
    let mut opener = Fake::busy();
    assert_eq!(
        notify(&mut opener, PLUGIN, &dialog()),
        Shown::Notified(NotificationShowReason::Shown)
    );
    assert_eq!(opener.notified.len(), 1, "no notification was sent");
    // The same message, not a summary of it.
    assert_eq!(opener.notified[0].title, dialog().title);
    assert_eq!(
        opener.notified[0].body.as_deref(),
        Some(dialog().body.as_str())
    );
}

#[test]
fn a_bare_dialog_reports_the_notification_reason_rather_than_assuming_it_landed() {
    // 🚨 SCOPE.md §7.2 in its dialog form. A fallback that silently fails is
    // worse than no fallback, because it removes the caller's last signal that
    // anything went wrong. Four of the five reasons mean the user saw nothing.
    for reason in [
        NotificationShowReason::Shown,
        NotificationShowReason::Disabled,
        NotificationShowReason::RateLimited,
        NotificationShowReason::NoForegroundClient,
        NotificationShowReason::Busy,
    ] {
        assert_eq!(
            notify(&mut Fake::busy_notifying(reason), PLUGIN, &dialog()),
            Shown::Notified(reason),
            "{:?} was not reported back",
            reason
        );
    }
}

#[test]
fn a_bare_dialog_says_so_when_neither_route_reached_the_user() {
    let shown = notify(&mut Fake::busy_unreachable(), PLUGIN, &dialog());
    let Shown::Unreachable(why) = shown else {
        panic!("a failed notification was not reported: {:?}", shown);
    };
    assert!(!why.is_empty(), "the failure carries no reason");
    // Distinct from a notification that was accepted, because nothing reached
    // the user at all.
    assert_ne!(
        notify(&mut Fake::busy(), PLUGIN, &dialog()),
        notify(&mut Fake::busy_unreachable(), PLUGIN, &dialog())
    );
}

#[test]
fn a_bare_dialog_that_opened_or_failed_sends_no_notification() {
    // The fallback is for a busy popup and nothing else. A notification beside
    // a dialog that opened would say everything twice.
    let mut opened = Fake::ok();
    assert_eq!(notify(&mut opened, PLUGIN, &dialog()), Shown::Opened);
    assert!(opened.notified.is_empty(), "a shown dialog also notified");

    let mut failed = Fake::failing();
    assert_eq!(
        notify(&mut failed, PLUGIN, &dialog()),
        Shown::Failed("socket is gone".to_string())
    );
    assert!(failed.notified.is_empty(), "a failed open also notified");
}

#[test]
fn a_question_that_lost_the_race_is_reported_as_busy_and_not_as_a_choice() {
    let answer = ask(&mut Fake::busy(), PLUGIN, &dialog(), &buttons());
    assert_eq!(
        answer,
        Answer::Unanswered(Unanswered::Busy(Explained::Reason(
            NotificationShowReason::Shown
        )))
    );
    // The distinction that matters: busy is not the user choosing any button.
    for index in 0..buttons().len() {
        assert!(!answer.chose(index), "busy chose button {}", index);
    }
}

#[test]
fn a_busy_question_explains_itself_without_becoming_a_statement() {
    // 🚨 A notification cannot collect an answer, so turning the question into
    // one would lose the answer and tell the caller nothing about it. The
    // notification only explains why nothing appeared.
    let mut opener = Fake::busy();
    let answer = ask(&mut opener, PLUGIN, &dialog(), &buttons());

    assert!(
        matches!(answer, Answer::Unanswered(Unanswered::Busy(_))),
        "the caller was not told the question went unasked"
    );
    assert_eq!(opener.notified.len(), 1, "the user was told nothing");
    let body = opener.notified[0].body.clone().unwrap();
    assert!(
        body.contains("could not be asked"),
        "the notification does not say the question went unasked: {:?}",
        body
    );
    assert!(
        body.contains("already open"),
        "the notification does not say why: {:?}",
        body
    );
    // No button label appears, because there is nothing to press.
    assert!(!body.contains("Rebuild"), "a button leaked into a toast");
    assert!(!body.contains("Leave it"), "a button leaked into a toast");
}

#[test]
fn a_busy_question_reports_whether_its_explanation_reached_anyone() {
    // The caller still learns it has no answer either way. What changes is
    // whether it can tell the user was left wondering.
    for reason in [
        NotificationShowReason::Shown,
        NotificationShowReason::Disabled,
        NotificationShowReason::NoForegroundClient,
    ] {
        let answer = ask(
            &mut Fake::busy_notifying(reason),
            PLUGIN,
            &dialog(),
            &buttons(),
        );
        assert_eq!(
            answer,
            Answer::Unanswered(Unanswered::Busy(Explained::Reason(reason)))
        );
        assert!(!answer.chose(0));
    }

    let answer = ask(&mut Fake::busy_unreachable(), PLUGIN, &dialog(), &buttons());
    let Answer::Unanswered(Unanswered::Busy(Explained::Unreachable(why))) = answer else {
        panic!("an unsendable explanation was not reported");
    };
    assert!(!why.is_empty());
}

#[test]
fn a_question_that_failed_to_open_sends_no_notification() {
    // Only a busy popup earns an explanation. A broken socket could not send
    // one anyway, and claiming otherwise would invent a second failure.
    let mut opener = Fake::failing();
    ask(&mut opener, PLUGIN, &dialog(), &buttons());
    assert!(opener.notified.is_empty(), "a failed open also notified");
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

#[test]
fn a_question_with_no_buttons_is_refused_rather_than_opened() {
    // 🚨 Nobody could answer it, and the single-popup limit is global, so an
    // unanswerable question would block every dialog in every workspace until
    // it timed out. It must never reach Herdr at all.
    let mut opener = Fake::ok();
    let started = Instant::now();
    let answer = ask(&mut opener, PLUGIN, &dialog(), &[]);
    let Answer::Unanswered(Unanswered::Failed(why)) = &answer else {
        panic!("an empty list was not refused: {:?}", answer);
    };
    assert!(!why.is_empty(), "the refusal carries no reason");
    assert!(opener.seen.is_empty(), "an unanswerable popup was opened");
    assert!(opener.notified.is_empty(), "the refusal also notified");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "a refused question waited {:?}",
        started.elapsed()
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
        ("0", Answer::Chose(0)),
        ("1", Answer::Chose(1)),
        ("2", Answer::Chose(2)),
        // 🚨 What Ctrl-C and every other choiceless ending write.
        (DISMISSED_WORD, Answer::Unanswered(Unanswered::Dismissed)),
    ] {
        let mut opener = Answering {
            word: word.to_string(),
            marks_started: true,
            delay: Duration::from_millis(50),
        };
        assert_eq!(ask(&mut opener, PLUGIN, &dialog(), &three()), expected);
    }
}

#[test]
fn a_popup_answering_a_word_no_button_uses_does_not_choose_any() {
    // A popup left over from an older build is the real case: 0.4.4 wrote
    // `primary` and `cancel`. Neither may fall through to a button, and nor may
    // an index this dialog has no button for.
    for word in ["rebuild", "primary", "cancel", "2"] {
        let mut opener = Answering {
            word: word.to_string(),
            marks_started: true,
            delay: Duration::from_millis(20),
        };
        let answer = ask(&mut opener, PLUGIN, &dialog(), &buttons());
        assert_eq!(
            answer,
            Answer::Unanswered(Unanswered::Unrecognised(word.to_string()))
        );
        for index in 0..3 {
            assert!(!answer.chose(index), "{:?} chose button {}", word, index);
        }
    }
}

#[test]
fn an_index_reaching_a_button_no_key_answers_is_not_a_choice() {
    // ⚠️ The caller handed in 80 buttons and only 70 were drawn and sent, so a
    // popup writing the 71st names no button of this dialog. Counting the
    // caller's list rather than the keyed one would act on a button nobody saw.
    let many: Vec<Button> = (0..80).map(|n| Button::new(&format!("b{}", n))).collect();
    for (word, expected) in [
        ("69", Answer::Chose(69)),
        (
            "70",
            Answer::Unanswered(Unanswered::Unrecognised("70".to_string())),
        ),
    ] {
        let mut opener = Answering {
            word: word.to_string(),
            marks_started: true,
            delay: Duration::from_millis(20),
        };
        assert_eq!(ask(&mut opener, PLUGIN, &dialog(), &many), expected);
    }
}

#[test]
fn an_answer_arriving_before_the_marker_is_still_heard() {
    // ✅ `plugin.pane.open` answers `ok` whether or not a process starts, so
    // the marker is the only evidence a popup drew. A fast popup that answers
    // and exits inside the startup window writes both, and the answer is what
    // decides.
    let mut opener = Answering {
        word: "0".to_string(),
        marks_started: false,
        delay: Duration::from_millis(20),
    };
    assert_eq!(
        ask(&mut opener, PLUGIN, &dialog(), &buttons()),
        Answer::Chose(0)
    );
}

#[test]
fn only_an_explicit_choice_ever_chooses_a_button() {
    // The safety property gathered into one place. Every outcome except the one
    // exact index must answer false, because the first button is where a caller
    // usually puts the action.
    assert!(!ask(&mut Fake::busy(), PLUGIN, &dialog(), &buttons()).chose(0));
    assert!(!ask(&mut Fake::failing(), PLUGIN, &dialog(), &buttons()).chose(0));
    assert!(!ask(&mut Fake::ok(), PLUGIN, &dialog(), &[]).chose(0));
    for why in [
        Unanswered::Busy(Explained::Reason(NotificationShowReason::Shown)),
        Unanswered::Busy(Explained::Unreachable("gone".to_string())),
        Unanswered::Failed("x".to_string()),
        Unanswered::Dismissed,
        Unanswered::NeverShown,
        Unanswered::TimedOut,
        Unanswered::Unrecognised("0".to_string()),
    ] {
        for index in 0..3 {
            assert!(
                !Answer::Unanswered(why.clone()).chose(index),
                "{:?} chose button {}",
                why,
                index
            );
        }
    }
    // A choice of one button is never a choice of another.
    assert!(Answer::Chose(0).chose(0));
    assert!(!Answer::Chose(1).chose(0));
    assert!(!Answer::Chose(0).chose(1));
}

// ── The popup half reading its own environment ────────────────────────────

#[test]
fn the_first_buttons_absence_is_what_makes_a_dialog_bare() {
    let bare = Popup::from_env(&Environment::from_pairs(&[
        (STATE_VAR, "danger"),
        (TITLE_VAR, "Stop"),
        (BODY_VAR, "It broke."),
    ]));
    assert!(bare.buttons.is_empty(), "a bare dialog read a button");
    assert_eq!(bare.dialog.state, State::Danger);
    assert_eq!(bare.dialog.title, "Stop");
    assert_eq!(bare.dialog.body, "It broke.");

    let actioned = Popup::from_env(&Environment::from_pairs(&[
        (STATE_VAR, "danger"),
        (&button_var(0), &format!("{} Delete", ENTER_KEY)),
        (&button_var(1), &format!("{} Keep", ESCAPE_KEY)),
    ]));
    assert_eq!(
        actioned.buttons,
        vec![
            Button::new("Delete").on_key(Key::Enter),
            Button::new("Keep").on_key(Key::Escape),
        ],
        "the button variables did not produce buttons"
    );
}

#[test]
fn the_list_ends_at_the_first_absent_index() {
    // 🔑 No separate count, so nothing can disagree with it. A gap ends the
    // list rather than being skipped over.
    let popup = Popup::from_env(&Environment::from_pairs(&[
        (&button_var(0), "a First"),
        (&button_var(2), "c Third"),
    ]));
    assert_eq!(popup.buttons.len(), 1, "a button past a gap was read");

    // And a second one without a first is no button at all.
    let popup = Popup::from_env(&Environment::from_pairs(&[(&button_var(1), "b Second")]));
    assert!(popup.buttons.is_empty());
}

#[test]
fn a_variable_set_but_empty_counts_as_absent() {
    // The idiom `crate::env` documents at nearly every call site. Herdr does
    // inject empty values.
    let popup = Popup::from_env(&Environment::from_pairs(&[
        (&button_var(0), ""),
        (TITLE_VAR, ""),
        (ANSWER_FILE_VAR, ""),
        (STARTED_FILE_VAR, ""),
    ]));
    assert!(popup.buttons.is_empty(), "an empty variable made a button");
    assert!(popup.dialog.title.is_empty());
    assert_eq!(popup.answer_file, None);
    assert_eq!(popup.started_file, None);
}

#[test]
fn an_empty_label_crosses_as_the_key_alone() {
    // A label is the caller's, and empty is one it may write. It travels as the
    // key with no separator, and reads back as an empty label on the same key.
    let mut opener = Fake::ok();
    ask(
        &mut opener,
        PLUGIN,
        &dialog(),
        &[Button::new(""), Button::new("keep")],
    );
    let env = &opener.only().env;
    assert_eq!(env.get(&button_var(0)).map(String::as_str), Some(ENTER_KEY));
    assert_eq!(
        read_back(env).buttons[0],
        Button::new("").on_key(Key::Enter)
    );
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
            for buttons in [Vec::new(), buttons(), three()] {
                let drawn = render(&dialog, &buttons, width);
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
        let drawn = plain(&render(&Dialog::new(state, "t", "b"), &[], 30));
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
        let drawn = plain(&render(&Dialog::new(state, "Title", "b"), &[], 40));
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
        let drawn = render(&Dialog::new(state, "t", "b"), &[], 30);
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
    let drawn = plain(&render(&dialog, &[], 40));
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

    let bare = plain(&render(&dialog, &[], 40));
    let bare: Vec<&str> = bare.lines().collect();
    assert_eq!(bare[bare.len() - 2], blank, "bare, second bottom row");
    assert_eq!(bare[bare.len() - 3], blank, "bare, first bottom row");

    let actioned = plain(&render(&dialog, &buttons(), 40));
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
    let drawn = plain(&render(&dialog, &buttons(), 40));
    let lines: Vec<&str> = drawn.lines().collect();
    let body = lines.iter().position(|l| l.contains("Body")).unwrap();
    let row = lines.iter().position(|l| l.contains("Rebuild")).unwrap();
    assert_eq!(row, body + 2, "the buttons sit against the body");
    assert_eq!(lines[body + 1].trim_matches(['│', ' ']), "", "no separator");
}

#[test]
fn a_bare_dialog_draws_no_buttons_and_no_separator() {
    let dialog = Dialog::new(State::Info, "T", "Body");
    let bare = plain(&render(&dialog, &[], 40));
    let actioned = plain(&render(&dialog, &buttons(), 40));
    assert!(!bare.contains("Rebuild"), "a bare dialog drew a button");
    assert!(
        !bare.contains("Leave it"),
        "a bare dialog drew a second button"
    );
    // The separator and the button row add two rows, and the bottom padding
    // gives one back, so an actioned dialog is exactly one row taller.
    assert_eq!(
        actioned.lines().count(),
        bare.lines().count() + 1,
        "the button row and its separator are not both present"
    );
}

#[test]
fn the_first_button_is_inverted_and_every_other_is_plain() {
    // Mike's design: the state's colour as the background with the text
    // inverted against it on the button the dialog leads with, and plain text
    // for every other. Emphasis is positional, so three buttons still carry
    // exactly one inversion.
    let drawn = render(&Dialog::new(State::Danger, "T", "B"), &three(), 60);
    let row = drawn
        .lines()
        .find(|line| line.contains("Delete"))
        .expect("no button row");
    assert!(row.contains("Archive"), "three buttons did not share a row");
    assert_eq!(
        row.matches("\u{1b}[7m").count(),
        1,
        "not exactly one inversion"
    );
    let inverted = row
        .split("\u{1b}[7m")
        .nth(1)
        .expect("the first button is not inverted");
    assert!(
        inverted.starts_with(&format!(" {} Delete ", ENTER_KEY)),
        "the inversion does not cover the first key and label: {:?}",
        inverted
    );
    assert!(
        !row.split("Delete").nth(1).unwrap().contains("\u{1b}[7m"),
        "inversion leaked past the first button"
    );
}

#[test]
fn a_long_body_wraps_inside_the_padding_rather_than_overflowing() {
    let body = "one two three four five six seven eight nine ten eleven twelve";
    let dialog = Dialog::new(State::Info, "T", body);
    let drawn = plain(&render(&dialog, &[], 30));
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
    let drawn = plain(&render(&Dialog::new(State::Info, long, "b"), &[], 30));
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
        let drawn = plain(&render(&dialog(), &buttons(), asked));
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
    let long = vec![
        Button::new("Rebuild every tab in this workspace"),
        Button::new("Leave everything exactly as it is"),
    ];
    for width in [24, 30, 40, 60] {
        let drawn = plain(&render(&dialog(), &long, width));
        for line in drawn.lines() {
            assert_eq!(width_of(line), width, "at {}: {:?}", width, line);
        }
    }

    // Both survive as something readable rather than one being erased.
    //
    // ⚠️ Not on one row. Labels this long cannot share a row at any of these
    // widths, so they stack, and each is then shortened only because it will
    // not fit a row by itself. Truncation is the last resort here, not the
    // first: a_frame_that_cannot_hold_every_button_stacks_them_and_keeps_the_labels_whole
    // pins the case stacking alone rescues.
    let drawn = plain(&render(&dialog(), &long, 40));
    assert!(
        drawn.lines().any(|line| line.contains("Rebuild")),
        "the first label was erased: {:?}",
        drawn
    );
    assert!(
        drawn.lines().any(|line| line.contains("Leave")),
        "the second label was erased: {:?}",
        drawn
    );

    // A pair that already fits is left exactly as the caller wrote it.
    let short = vec![Button::new("Go"), Button::new("Stop")];
    let fits = plain(&render(&dialog(), &short, 40));
    assert!(fits.contains(" Go "), "a fitting label was truncated");
    assert!(fits.contains("Stop"), "a fitting label was truncated");
}

#[test]
fn a_frame_that_cannot_hold_every_button_stacks_them_and_keeps_the_labels_whole() {
    // 🪤 The defect the preview exposed on 2026-09-11. The old layout kept the
    // border and cut the words instead: at the floor it drew `↵ reb` and
    // `esc kee`, so "rebuild anyway" and "keep them" both arrived as
    // fragments. Stacking costs one row of height per button and keeps every
    // label.
    //
    // 🔑 Whole is the assertion. The border holding is checked below as well,
    // but a frame whose borders line up around nonsense was the state this
    // came from, so the border alone cannot be what this test asks.
    for labels in [
        vec!["rebuild anyway", "keep them"],
        vec!["rebuild anyway", "keep them", "archive"],
        vec!["rebuild anyway", "keep them", "archive", "ask later"],
    ] {
        let buttons: Vec<Button> = labels.iter().map(|label| Button::new(label)).collect();
        let frame = layout(&dialog(), &buttons, 30, None);
        let drawn = plain(&frame.text);
        let lines: Vec<&str> = drawn.lines().collect();
        assert_eq!(frame.buttons.len(), labels.len(), "a button was not drawn");

        // One row each, consecutive, in the caller's order.
        for (index, rect) in frame.buttons.iter().enumerate() {
            assert_eq!(
                rect.row,
                frame.buttons[0].row + index as u16,
                "{:?} did not stack in order: {:?}",
                labels,
                drawn
            );
            assert!(
                lines[rect.row as usize].contains(labels[index]),
                "{:?} did not survive whole: {:?}",
                labels[index],
                lines[rect.row as usize]
            );
        }

        for line in drawn.lines() {
            assert_eq!(width_of(line), 30, "the border moved: {:?}", line);
        }
    }
}

#[test]
fn whether_the_buttons_share_a_row_is_decided_by_what_fits_rather_than_by_a_width() {
    // 🔑 The kit draws the key affordances itself and the labels are the
    // caller's, so one width stacks one list and not another. A threshold
    // constant could not answer this, and that is the whole reason the
    // condition is measured against the drawn widths.
    let rows = |buttons: &[Button], width: usize| -> Vec<u16> {
        layout(&dialog(), buttons, width, None)
            .buttons
            .iter()
            .map(|rect| rect.row)
            .collect()
    };
    let shared = |rows: &[u16]| rows.iter().all(|row| *row == rows[0]);

    // The same frame, two pairs of labels, two different answers.
    let short = vec![Button::new("Go"), Button::new("Stop")];
    let long = vec![Button::new("rebuild anyway"), Button::new("keep them")];
    assert!(
        shared(&rows(&short, 30)),
        "a pair that fits its row was stacked anyway"
    );
    assert!(
        !shared(&rows(&long, 30)),
        "a pair too wide for its row shared one"
    );

    // The same pair, one cell either side of where it stops fitting. Found by
    // measuring the drawn widths, not asserted from a constant.
    assert!(
        !shared(&rows(&long, 40)),
        "still too wide at 40, and it shared a row"
    );
    assert!(
        shared(&rows(&long, 41)),
        "one more cell fits, and it stacked anyway"
    );

    // 🔑 A sum rather than a pair: a third button costs its own width and a gap,
    // and when the sum does not fit, every button stacks rather than some.
    let with_third = vec![
        Button::new("rebuild anyway"),
        Button::new("keep them"),
        Button::new("x"),
    ];
    assert!(
        !shared(&rows(&with_third, 41)),
        "a third button fitted in the width the pair needed"
    );
    let stacked = rows(&with_third, 41);
    assert_eq!(stacked, vec![stacked[0], stacked[0] + 1, stacked[0] + 2]);
}

// ── The mouse ─────────────────────────────────────────────────────────────

#[test]
fn the_buttons_report_where_they_were_actually_drawn() {
    // ✅ The coordinates a click arrives carrying are pane-local, and the frame
    // is painted from the pane's origin, so a reported rectangle has to cover
    // exactly the cells the button really occupies, key affordance included.
    for width in [24, 40, 60, 80] {
        let frame = layout(&dialog(), &three(), width, None);
        assert_eq!(frame.buttons.len(), 3, "a button was not reported");

        // ⚠️ Each rectangle is read from the row *it* reports, never from one
        // row assumed to hold them all. A frame too narrow for a single row
        // stacks the buttons, and 24 is such a frame.
        let expected = [
            format!(" {}", ENTER_KEY),
            ESCAPE_KEY.to_string(),
            "!".to_string(),
        ];
        for (rect, key) in frame.buttons.iter().zip(&expected) {
            assert!(
                span(&frame.text, *rect).starts_with(key.as_str()),
                "a rectangle covers {:?} at width {}",
                span(&frame.text, *rect),
                width
            );
        }
        // The rectangles must not overlap, or one button would swallow clicks
        // meant for another. Stacked, they are free to share columns — being on
        // different rows is what separates them there.
        for pair in frame.buttons.windows(2) {
            if pair[0].row == pair[1].row {
                assert!(
                    pair[0].column + pair[0].width <= pair[1].column,
                    "the rectangles overlap at width {}",
                    width
                );
            }
        }
    }

    // Exactly, where the frame is wide enough for every label in full.
    let frame = layout(&dialog(), &three(), 60, None);
    assert_eq!(
        span(&frame.text, frame.buttons[0]),
        format!(" {} Delete ", ENTER_KEY)
    );
    assert_eq!(
        span(&frame.text, frame.buttons[1]),
        format!("{} Keep", ESCAPE_KEY)
    );
    assert_eq!(span(&frame.text, frame.buttons[2]), "! Archive");
}

#[test]
fn the_kit_draws_each_key_and_the_caller_never_types_one() {
    // 🔑 Drift, not convenience. Three plugins typing their own glyph would
    // eventually disagree about the symbol, the spacing, or whether to bother.
    let drawn = plain(&render(
        &dialog(),
        &[Button::new("close anyway"), Button::new("keep")],
        60,
    ));
    assert!(
        drawn.contains(&format!("{} close anyway", ENTER_KEY)),
        "the Enter key was not drawn: {:?}",
        drawn
    );
    assert!(
        drawn.contains(&format!("{} keep", ESCAPE_KEY)),
        "the Escape key was not drawn: {:?}",
        drawn
    );
    // The caller's label is kept verbatim, so only the prefix is the kit's.
    assert!(drawn.contains("close anyway"));
    assert!(drawn.contains("keep"));
}

#[test]
fn a_click_on_a_button_is_that_button_and_a_click_anywhere_else_is_none() {
    // ⚠️ Clicking the body must resolve nothing. A misclick that fired a
    // leading button whose action is destructive is the failure this prevents.
    let frame = layout(&dialog(), &three(), 60, None);
    for (index, rect) in frame.buttons.iter().enumerate() {
        assert_eq!(frame.hit(rect.column, rect.row), Some(index));
        assert_eq!(
            frame.hit(rect.column + rect.width - 1, rect.row),
            Some(index),
            "the last cell of button {} missed it",
            index
        );
        // One cell outside each edge, which is the gap between two buttons.
        assert_eq!(frame.hit(rect.column - 1, rect.row), None);
        assert_eq!(frame.hit(rect.column + rect.width, rect.row), None);
    }
    let first = frame.buttons[0];
    // The row above and the row below hold no buttons at all.
    assert_eq!(frame.hit(first.column, first.row - 1), None);
    assert_eq!(frame.hit(first.column, first.row + 1), None);
    // The title, the body, and the far corner.
    assert_eq!(frame.hit(0, 0), None);
    assert_eq!(frame.hit(5, 3), None);
    assert_eq!(frame.hit(59, 0), None);
}

#[test]
fn a_bare_dialog_offers_nothing_to_click() {
    let frame = layout(&dialog(), &[], 40, None);
    assert!(frame.buttons.is_empty());
    // Every position on a bare dialog is no button, so the caller's "any click
    // dismisses" rule can never be overridden by a hit.
    for row in 0..frame.text.lines().count() as u16 {
        for column in [0, 5, 20, 39] {
            assert_eq!(frame.hit(column, row), None);
        }
    }
}

#[test]
fn hover_changes_attributes_and_never_geometry() {
    // 🔑 The invariant that makes a hover repaint safe. Every redraw paints from
    // the pane's origin over the previous frame, so if the pointer could move a
    // cell the frame would accumulate rather than replace.
    let reference = layout(&dialog(), &three(), 60, None);
    for hot in [None, Some(0), Some(1), Some(2), Some(3)] {
        let frame = layout(&dialog(), &three(), 60, hot);
        assert_eq!(
            plain(&frame.text),
            plain(&reference.text),
            "{:?} moved a character",
            hot
        );
        assert_eq!(frame.buttons, reference.buttons, "{:?} moved a button", hot);
    }
}

#[test]
fn only_the_hovered_button_is_underlined() {
    let underline = "\u{1b}[4m";

    let none = layout(&dialog(), &three(), 60, None);
    assert!(
        !none.text.contains(underline),
        "nothing hovered, yet underlined"
    );

    for (hot, label) in [(0, "Delete"), (1, "Keep"), (2, "Archive")] {
        let frame = layout(&dialog(), &three(), 60, Some(hot));
        let row = frame
            .text
            .lines()
            .find(|line| line.contains(label))
            .unwrap();
        assert_eq!(
            row.matches(underline).count(),
            1,
            "not exactly one underline hovering {}",
            hot
        );
        // The underline opens after every earlier label and before this one.
        let before = row.split(label).next().unwrap();
        let after_underline = before.rsplit(underline).next().unwrap();
        assert!(
            before.contains(underline),
            "the underline is not before button {}: {:?}",
            hot,
            row
        );
        assert!(
            !["Delete", "Keep", "Archive"]
                .iter()
                .any(|other| after_underline.contains(other)),
            "the underline is on another button than {}: {:?}",
            hot,
            row
        );
    }

    // An index no button has marks nothing.
    let past = layout(&dialog(), &three(), 60, Some(3));
    assert!(
        !past.text.contains(underline),
        "a missing button was hovered"
    );
}

#[test]
fn an_empty_first_label_draws_its_key_alone_and_keeps_the_frame_square() {
    // Reachable from the caller rather than from truncation: `Button::new("")`
    // is a button a caller can hand in. The key must then stand alone, because
    // a separator with nothing after it pads the inversion with a cell of
    // nothing.
    let buttons = vec![Button::new(""), Button::new("keep")];
    for width in [24, 40, 60] {
        let drawn = plain(&render(&dialog(), &buttons, width));
        for line in drawn.lines() {
            assert_eq!(width_of(line), width, "at {}: {:?}", width, line);
        }
    }
    // The reported width is what discriminates. A separator that survived an
    // empty label would make the button one cell wider, and the surrounding
    // gap makes that invisible to any search of the drawn text.
    let frame = layout(&dialog(), &buttons, 60, None);
    let first = frame.buttons[0];
    assert_eq!(
        first.width,
        (cells_of(ENTER_KEY) + 2) as u16,
        "a separator survived an empty label"
    );
    assert_eq!(
        span(&frame.text, first),
        format!(" {} ", ENTER_KEY),
        "the key did not stand alone"
    );
}

/// Counts characters, independently of the module's own counter.
fn cells_of(text: &str) -> usize {
    text.chars().count()
}

#[test]
fn an_empty_body_still_draws_a_complete_frame() {
    let drawn = plain(&render(&Dialog::new(State::Info, "", ""), &[], 24));
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

#[test]
fn a_click_lands_on_the_right_button_when_they_are_stacked() {
    // 🚨 The rectangles move to different rows when a frame is too narrow for
    // one, and `Frame::hit` is what the mouse path asks. A hit test written
    // against a single shared row answers `None` for all but one of them and
    // loses every click on the rest, in silence.
    let buttons = vec![
        Button::new("rebuild anyway"),
        Button::new("keep them"),
        Button::new("archive"),
    ];
    let frame = layout(&dialog(), &buttons, 30, None);
    assert_eq!(frame.buttons.len(), 3);
    assert_ne!(
        frame.buttons[0].row, frame.buttons[1].row,
        "the buttons did not stack"
    );

    // Both edges of each, because a rectangle that is off by one at either end
    // still answers correctly in the middle.
    for (index, rect) in frame.buttons.iter().enumerate() {
        assert_eq!(Some(index), frame.hit(rect.column, rect.row));
        assert_eq!(
            Some(index),
            frame.hit(rect.column + rect.width - 1, rect.row)
        );
        // 🚨 A position on no button answers none. An actioned dialog treats
        // that as no answer, and its leading button may be destructive.
        assert_eq!(
            None,
            frame.hit(rect.column + rect.width, rect.row),
            "a cell past button {}'s right edge answered it",
            index
        );
    }
    let first = frame.buttons[0];
    assert_eq!(
        None,
        frame.hit(first.column, first.row - 1),
        "the blank row above the buttons answered one"
    );
}
