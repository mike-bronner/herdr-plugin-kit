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

use herdr_plugin_kit::dialog::{layout, Buttons, Dialog, Hot, State};

/// The same body for every state, so only the styling varies between frames.
const BODY: &str = "Rebuilding this tab will close the two panes that are \
                    running agents. Their work cannot be recovered.";

fn main() {
    let width = std::env::args()
        .nth(1)
        .and_then(|given| given.parse().ok())
        .unwrap_or(60);

    let buttons = Buttons::new("rebuild anyway", "keep them");
    let states = [
        (State::Info, "Nothing to do"),
        (State::Success, "Rebuilt"),
        (State::Warning, "Two agents are running"),
        (State::Danger, "Delete this worktree?"),
    ];

    // The eight: four states, each bare and actioned.
    for (state, title) in states {
        let dialog = Dialog::new(state, title, BODY);
        for buttons in [None, Some(&buttons)] {
            let variant = match buttons {
                Some(_) => "actioned",
                None => "bare",
            };
            println!("── {:?}, {} ──\n", state, variant);
            println!("{}\n", layout(&dialog, buttons, width, Hot::None).text);
        }
    }

    // Hover, which only an actioned dialog has.
    let dialog = Dialog::new(State::Warning, "Hover", BODY);
    for hot in [Hot::Primary, Hot::Cancel] {
        println!("── hover: {:?} ──\n", hot);
        println!("{}\n", layout(&dialog, Some(&buttons), width, hot).text);
    }

    // The two narrow shapes, which are the reason this example earns its keep.
    let narrow = Dialog::new(State::Danger, "Narrow", "At the floor.");

    // Stacked: one button per row, both labels whole, one row of height paid
    // for them. This is what a frame too narrow for a single row now draws.
    println!("── stacked, 30 cells ──\n");
    println!("{}\n", layout(&narrow, Some(&buttons), 30, Hot::None).text);

    // The floor, where stacking is not enough on its own and the primary label
    // is shortened as well. Truncation is the last resort, not the first.
    println!("── the 24-cell floor ──\n");
    println!("{}\n", layout(&narrow, Some(&buttons), 24, Hot::None).text);
}
