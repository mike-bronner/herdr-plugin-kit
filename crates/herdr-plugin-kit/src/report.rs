//! Tells the user something went wrong, and reports what became of the telling.
//!
//! 🚨 **The defect this module exists to fix is a discarded answer.** SCOPE.md
//! §7.2: `notification.show` answers `shown: bool` and a `reason` that is a
//! real enum of five values, and ✅ both plugins that send a toast today throw
//! that whole answer away. So a message Herdr dropped and a message the user
//! read are indistinguishable to the plugin, **from information the plugin
//! already received**. That is the same shape as a stale binary answering
//! `--version`: a silent failure detectable from data already in hand.
//!
//! So [`send`] hands the caller the [`NotificationShowReason`], and falls back
//! to a pane when the reason says nothing was delivered.
//!
//! # Why a pane for the detail
//!
//! Three limits, two of them measured (SCOPE.md §7.3):
//!
//! - ✅ **No severity.** Herdr hardcodes every API-originated notification to
//!   one kind, so a plugin cannot style an error differently from a success.
//! - ✅ **One at a time.** Only one toast is live, and the next answers `busy`.
//! - ⚠️ **A rate limit.** Weaker basis than the other two: what is established
//!   is that the schema declares a `rate_limited` reason the server can return.
//!   That is why this module routes on the reason rather than predicting when
//!   it fires.
//!
//! A pane has none of the three. It is the plugin's own terminal, carrying
//! every issue at once.
//!
//! # What this module will not do
//!
//! **Never waits, and never gates the caller.** A cosmetic warning must not
//! make somebody wait to get their workspace, so there is no channel here, no
//! marker, and nothing to poll. [`crate::dialog::ask`] is the module that waits,
//! and it waits because its answer is the thing the caller asked for.
//!
//! **Never draws.** The popup is opened through `plugin.pane.open`, which is a
//! socket call, so this module compiles without `crossterm` and the `report`
//! feature does not turn on `dialog`. recent-spaces is a headless watcher and
//! pays nothing for reporting its issues.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::api::generated::{
    NotificationShowParams, NotificationShowReason, PluginPaneOpenParams, PluginPanePlacement,
    PopupSize, PopupSizeString,
};
use crate::surface::Transport;

/// The pane entrypoint every consuming plugin declares for its issue reports.
///
/// The kit fixes the convention so the two halves cannot disagree. Each plugin
/// declares the matching `[[panes]]` entry, which a crate cannot supply.
pub const ENTRYPOINT: &str = "issues";

/// Where the plugin's issues were written, for the popup to read.
///
/// 🔑 **A path rather than the text itself.** Pane environment is a request
/// field, and a report carrying every issue at once is exactly the thing with
/// no length bound. A file has none of that question about it.
pub const FILE_VAR: &str = "HERDR_PLUGIN_ISSUES_FILE";

/// What the report is about, for the popup to head the list with.
pub const HEADING_VAR: &str = "HERDR_PLUGIN_ISSUES_HEADING";

/// The popup's requested width, as a percentage of the tab.
///
/// Wider than a dialog's 60%, because a dialog holds a sentence and this holds
/// a list. Both are requests: a size the schema refuses is dropped rather
/// than failing the whole request.
pub const WIDTH: &str = "80%";

/// The popup's requested height, as a percentage of the tab.
pub const HEIGHT: &str = "60%";

/// Whether the user can ignore this message and still have a working plugin.
///
/// 🚨 **This exists for one row of the [`send`] policy table, and that row is a
/// deliberate override of a user preference.** Everything else routes on what
/// Herdr answered. `disabled` is the one reason where what to do next depends
/// on what the message is, so the caller has to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something the user can ignore and still have a working plugin.
    Cosmetic,
    /// A diagnostic that stops the plugin working.
    ///
    /// ⚠️ Not a statement about urgency or tone. The question this answers is
    /// narrow: if the user never sees this, is the plugin broken for them?
    Diagnostic,
}

/// What to tell the user, and where the detail goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report<'a> {
    /// The notification's title, and the pane's heading ([`HEADING_VAR`]).
    ///
    /// One field rather than two, because they are one fact: what this report
    /// is about. A caller wanting them to differ is describing two reports.
    pub title: &'a str,
    /// The notification's body. One line, because a toast is one line.
    pub summary: &'a str,
    /// Every issue, in full, for the pane to show.
    ///
    /// Empty is allowed and means the summary was the whole of it, in which
    /// case the pane shows the summary rather than an empty file.
    pub detail: &'a str,
    /// Which side of the `disabled` policy row this message falls.
    pub kind: Kind,
}

/// Tells the user, and answers **what Herdr said about delivery**.
///
/// Sends a notification first, and opens a pane carrying [`Report::detail`]
/// when the reason says nothing was delivered. The reason is handed back
/// either way.
///
/// # The fallback policy, and the reason it travels with
///
/// | `reason` | Falls back to a pane | Why |
/// |---|---|---|
/// | `no_foreground_client` | ✅ yes | nothing was there to draw it, so nothing was delivered |
/// | `rate_limited` | ✅ yes | the message was dropped rather than shown |
/// | `busy` | ✅ yes | one toast is live at a time, and this was not it |
/// | `shown` | ❌ no | it was delivered |
/// | `disabled`, [`Kind::Cosmetic`] | ❌ no | respected |
/// | `disabled`, [`Kind::Diagnostic`] | ✅ yes | overridden |
///
/// 🚨 **Decided by Mike, and the reason is carried here on purpose, because
/// the last row on its own reads like a plugin ignoring a user preference.** A
/// user who turns off toasts has said something about toasts, not about
/// diagnostics. So `disabled` is respected for anything cosmetic, and
/// overridden only for a diagnostic that stops the plugin working.
///
/// ⚠️ **A notification that could not be sent at all falls back too**, and that
/// row is not in §7.2's table because the table routes on reasons Herdr
/// returned. A send that failed produced no reason, and it is the one case
/// where non-delivery is certain rather than reported. The `Err` still reaches
/// the caller, so nothing is hidden by the fallback.
///
/// ⚠️ **The return says only what Herdr answered about the notification.**
/// Whether a pane appeared is deliberately not in it, because §7.4 makes that
/// failure silent: stderr already carries the message.
pub fn send(
    transport: &mut impl Transport,
    plugin_id: &str,
    report: &Report<'_>,
) -> Result<NotificationShowReason, String> {
    let sent = transport.show_notification(NotificationShowParams {
        body: Some(report.summary.to_string()),
        position: None,
        sound: None,
        title: report.title.to_string(),
    });

    let undelivered = match &sent {
        Ok(reason) => falls_back(*reason, report.kind),
        Err(_) => true,
    };
    if undelivered {
        fall_back(transport, plugin_id, report, &std::env::temp_dir());
    }

    sent
}

/// Whether this reason, for this kind of message, means the pane is needed.
///
/// Pure, and separated from the sending for that reason: every row of [`send`]'s
/// table is then checked in microseconds without a transport, and a row cannot
/// rot unnoticed because provoking it needs a live server.
fn falls_back(reason: NotificationShowReason, kind: Kind) -> bool {
    match reason {
        // Delivered. Nothing to add.
        NotificationShowReason::Shown => false,
        // The user said something about toasts, not about diagnostics.
        NotificationShowReason::Disabled => matches!(kind, Kind::Diagnostic),
        // Every remaining reason is Herdr saying it did not show the message.
        NotificationShowReason::RateLimited
        | NotificationShowReason::NoForegroundClient
        | NotificationShowReason::Busy => true,
    }
}

/// Writes the issues and asks for a popup over them.
///
/// 🚨 **Every failure here is silent, and that is SCOPE.md §7.4 rather than
/// laziness.** stderr already carries the message: this pane is a second, more
/// readable copy of something the plugin has already emitted. So a failure to
/// write the file or open the popup costs the user a nicety, while reporting it
/// would cost them either a second failure message about the failure to show
/// the first, or a caller that now has to handle an error path for its own
/// error path. ⚠️ The counterpart is real and accepted: a popup that never
/// opens looks exactly like one the user dismissed.
///
/// The caller is never gated on this. No answer is read, no marker is watched,
/// and nothing is waited for.
fn fall_back(transport: &mut impl Transport, plugin_id: &str, report: &Report<'_>, dir: &Path) {
    let Ok(path) = write_issues(dir, report) else {
        return;
    };
    let _ = transport.open_pane(open_params(plugin_id, report, &path));
}

/// Writes the detail to a file of its own, and answers where.
fn write_issues(dir: &Path, report: &Report<'_>) -> std::io::Result<PathBuf> {
    let path = issues_path(dir);
    let text = match report.detail.is_empty() {
        true => report.summary,
        false => report.detail,
    };
    std::fs::write(&path, text)?;
    Ok(path)
}

/// A path no concurrent call can collide with.
///
/// 🚨 **Keyed on the process id *and* a counter, because the pid alone is not
/// unique enough.** ✅ SCOPE.md §7.4 records that keying on the pid alone caused
/// a real race which failed a test suite about one run in four: every call in a
/// process shared one path, so a second report overwrote a first whose popup had
/// not read it yet. Two reports from one plugin is the normal case here rather
/// than an exotic one, since a watcher reports whenever it finds something.
///
/// ⚠️ **The file is not removed.** The popup outlives this call and reads the
/// file afterwards, so there is no moment here at which deleting it is safe.
/// This is why the file lives in the temp directory rather than anywhere the
/// kit would have to keep tidy.
fn issues_path(dir: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    dir.join(format!(
        "herdr-plugin-issues-{}-{}.txt",
        std::process::id(),
        n
    ))
}

/// Builds the `plugin.pane.open` request.
///
/// ⚠️ **The placement is `popup` and is not a parameter**, matching
/// `dialog`: ✅ an overlay covers the entire tab with zero rows of any
/// underlying pane surviving, measured 2026-09-11.
///
/// `focus` is `true` and `workspace_id` is unset, both following the
/// configuration `dialog` measured working against a live Herdr on 2026-09-11
/// rather than an independent choice made here.
fn open_params(plugin_id: &str, report: &Report<'_>, path: &Path) -> PluginPaneOpenParams {
    let mut env: HashMap<String, String> = HashMap::new();
    env.insert(HEADING_VAR.to_string(), report.title.to_string());
    env.insert(FILE_VAR.to_string(), path.to_string_lossy().into_owned());

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
/// request, and the test suite pins [`WIDTH`] and [`HEIGHT`] as valid so an edit
/// that breaks one reddens instead of silently unsizing every report.
fn popup_size(percent: &str) -> Option<PopupSize> {
    PopupSizeString::try_from(percent)
        .ok()
        .map(PopupSize::String)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transport that answers what it is told to and records what it got.
    struct Fake {
        reason: Result<NotificationShowReason, String>,
        opened: Vec<PluginPaneOpenParams>,
        notified: Vec<NotificationShowParams>,
    }

    impl Fake {
        fn answering(reason: NotificationShowReason) -> Fake {
            Fake {
                reason: Ok(reason),
                opened: Vec::new(),
                notified: Vec::new(),
            }
        }
    }

    impl Transport for Fake {
        fn open_pane(
            &mut self,
            params: PluginPaneOpenParams,
        ) -> Result<(), crate::surface::OpenError> {
            self.opened.push(params);
            Ok(())
        }

        fn show_notification(
            &mut self,
            params: NotificationShowParams,
        ) -> Result<NotificationShowReason, String> {
            self.notified.push(params);
            self.reason.clone()
        }
    }

    fn report(kind: Kind) -> Report<'static> {
        Report {
            title: "Two plugins are stale",
            summary: "2 issues found",
            detail: "one\ntwo",
            kind,
        }
    }

    /// A directory of this test's own, removed when the test ends.
    struct Dir(PathBuf);

    impl Dir {
        fn new(name: &str) -> Dir {
            let dir = std::env::temp_dir().join(format!(
                "herdr-report-test-{}-{}",
                std::process::id(),
                name
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Dir(dir)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_counter_makes_every_path_unique_within_one_process() {
        let dir = Dir::new("unique");
        let first = issues_path(&dir.0);
        let second = issues_path(&dir.0);
        assert_ne!(first, second);
    }

    #[test]
    fn every_path_carries_the_process_id() {
        let dir = Dir::new("pid");
        let path = issues_path(&dir.0);
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.contains(&format!("-{}-", std::process::id())),
            "{} carries no pid",
            name
        );
    }

    #[test]
    fn the_detail_is_what_reaches_the_file() {
        let dir = Dir::new("detail");
        let path = write_issues(&dir.0, &report(Kind::Cosmetic)).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "one\ntwo");
    }

    #[test]
    fn an_empty_detail_writes_the_summary_rather_than_nothing() {
        let dir = Dir::new("empty");
        let bare = Report {
            detail: "",
            ..report(Kind::Cosmetic)
        };
        let path = write_issues(&dir.0, &bare).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "2 issues found");
    }

    #[test]
    fn two_reports_do_not_overwrite_each_other() {
        let dir = Dir::new("race");
        let first = write_issues(&dir.0, &report(Kind::Cosmetic)).unwrap();
        let second = write_issues(
            &dir.0,
            &Report {
                detail: "three",
                ..report(Kind::Cosmetic)
            },
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(first).unwrap(), "one\ntwo");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "three");
    }

    #[test]
    fn an_unwritable_directory_opens_no_pane_and_says_nothing() {
        let mut fake = Fake::answering(NotificationShowReason::Busy);
        fall_back(
            &mut fake,
            "p",
            &report(Kind::Cosmetic),
            Path::new("/no/such/directory/anywhere"),
        );
        assert!(fake.opened.is_empty());
    }

    #[test]
    fn the_pane_request_carries_the_file_and_the_heading() {
        let dir = Dir::new("params");
        let mut fake = Fake::answering(NotificationShowReason::Busy);
        fall_back(&mut fake, "issue-finder", &report(Kind::Diagnostic), &dir.0);

        let params = &fake.opened[0];
        assert_eq!(params.entrypoint, ENTRYPOINT);
        assert_eq!(params.plugin_id, "issue-finder");
        assert_eq!(params.placement, Some(PluginPanePlacement::Popup));
        assert_eq!(
            params.env.get(HEADING_VAR).map(String::as_str),
            Some("Two plugins are stale")
        );
        let path = params.env.get(FILE_VAR).expect("no file variable");
        assert_eq!(std::fs::read_to_string(path).unwrap(), "one\ntwo");
    }

    #[test]
    fn both_popup_sizes_are_shapes_the_schema_accepts() {
        assert!(popup_size(WIDTH).is_some(), "{} is not a size", WIDTH);
        assert!(popup_size(HEIGHT).is_some(), "{} is not a size", HEIGHT);
    }

    #[test]
    fn a_size_the_schema_refuses_drops_the_size_rather_than_the_request() {
        assert!(popup_size("120%").is_none());
    }
}
