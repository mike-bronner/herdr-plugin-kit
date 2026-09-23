//! Draws every dialog combination locally, in real colour.
//!
//! ```sh
//! cargo run --features dialog --example preview        # 60 cells wide
//! cargo run --features dialog --example preview -- 80
//! ```
//!
//! 🔑 **Why this exists, and what it already paid for.** The palette in
//! `dialog::State::colour` could not be measured: the probe that settled
//! everything else about these dialogs captured geometry and characters but
//! not colour, because the capture library discards SGR attributes. So the
//! only way to judge it was to look at it, and before this example that meant
//! linking a temporary plugin into a live Herdr server and unlinking it
//! afterwards. **Mike made three such trips and the answer was still
//! unsettled.** This turns each of those into one local command.
//!
//! ✅ **The fourth attempt was this example, and it settled it.** The palette
//! was reviewed and approved on 2026-09-11 from output this file rendered,
//! along with the stacked and floor button layouts below.
//!
//! 🔑 **That is the argument for building the thing that makes a check
//! trivial**, and it is recorded here rather than in a commit message nobody
//! re-reads. A check that costs a round trip to a live server is a check that
//! does not happen; the same check as a local command happened immediately.
//!
//! 🚨 **It draws through [`herdr_plugin_kit::dialog::layout`], which is the
//! exact call the popup's own paint path makes.** That is deliberate and must
//! stay: a preview with a drawing path of its own could agree with itself while
//! disagreeing with the dialog, which is worse than having no preview.
//!
//! ⚠️ What it cannot show: the popup's real width comes from Herdr's `60%`
//! sizing against the tab, the frame is painted from the pane's origin rather
//! than inline, and hover here is a still rather than something the pointer
//! drives.

use herdr_plugin_kit::dialog::{layout, Button, Dialog, Key, State};

/// The same body for every state, so only the styling varies between frames.
const BODY: &str = "Rebuilding this tab will close the two panes that are \
                    running agents. Their work cannot be recovered.";

fn main() {
    let width = std::env::args()
        .nth(1)
        .and_then(|given| given.parse().ok())
        .unwrap_or(60);

    let buttons = vec![Button::new("rebuild anyway"), Button::new("keep them")];
    let states = [
        (State::Info, "Nothing to do"),
        (State::Success, "Rebuilt"),
        (State::Warning, "Two agents are running"),
        (State::Danger, "Delete this worktree?"),
    ];

    // The eight: four states, each bare and actioned.
    for (state, title) in states {
        let dialog = Dialog::new(state, title, BODY);
        for (variant, buttons) in [("bare", &[][..]), ("actioned", &buttons[..])] {
            println!("── {:?}, {} ──\n", state, variant);
            println!("{}\n", layout(&dialog, buttons, width, None).text);
        }
    }

    // Hover, which only an actioned dialog has.
    let dialog = Dialog::new(State::Warning, "Hover", BODY);
    for hot in 0..buttons.len() {
        println!("── hover: button {} ──\n", hot);
        println!("{}\n", layout(&dialog, &buttons, width, Some(hot)).text);
    }

    // Named keys, which are the drawing decision of 2026-09-14 (SCOPE.md §7.5.8)
    // and the part a reviewer has to look at rather than read about. Each
    // button draws the one key that answers it, so naming a key replaces the
    // default glyph rather than joining it.
    let pair = |first: Option<Key>, second: Option<Key>| {
        let named = |label: &str, key: Option<Key>| match key {
            Some(key) => Button::new(label).on_key(key),
            None => Button::new(label),
        };
        vec![named("rebuild anyway", first), named("keep them", second)]
    };
    let asking = Dialog::new(State::Danger, "Close the pane?", BODY);
    for (caption, keys) in [
        ("a named first key", pair(Some(Key::Char('y')), None)),
        ("a named second key", pair(None, Some(Key::Char('n')))),
        (
            "both named",
            pair(Some(Key::Char('y')), Some(Key::Char('n'))),
        ),
        (
            "the safe answer under Enter",
            pair(Some(Key::Char('y')), Some(Key::Enter)),
        ),
    ] {
        println!("── {} ──\n", caption);
        println!("{}\n", layout(&asking, &keys, width, None).text);
    }

    // More than two, which is what round 4 added. The ladder hands the third
    // button the first free character, and only the first is emphasised.
    let three = vec![
        Button::new("delete").on_key(Key::Char('d')),
        Button::new("keep them").on_key(Key::Enter),
        Button::new("archive"),
    ];
    println!("── three buttons, the safe answer under Enter ──\n");
    println!("{}\n", layout(&asking, &three, width, None).text);

    // The two narrow shapes, which are the reason this example earns its keep.
    let narrow = Dialog::new(State::Danger, "Narrow", "At the floor.");

    // 📏 The same floor with one-cell keys on both buttons, which is the
    // cheapest a button pair can be drawn and gives the labels the most room the
    // frame can offer.
    let named = pair(Some(Key::Char('y')), Some(Key::Char('n')));
    println!("── named keys at the 24-cell floor ──\n");
    println!("{}\n", layout(&narrow, &named, 24, None).text);

    // Stacked: one button per row, every label whole, one row of height paid
    // for each. This is what a frame too narrow for a single row draws.
    println!("── stacked, 30 cells ──\n");
    println!("{}\n", layout(&narrow, &buttons, 30, None).text);
    println!("── three stacked, 30 cells ──\n");
    println!("{}\n", layout(&narrow, &three, 30, None).text);

    // The floor, where stacking is not enough on its own and the first label
    // is shortened as well. Truncation is the last resort, not the first.
    println!("── the 24-cell floor ──\n");
    println!("{}\n", layout(&narrow, &buttons, 24, None).text);
}
