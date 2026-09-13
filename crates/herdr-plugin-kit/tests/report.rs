//! What the `report` module promises.
//!
//! Everything here runs without a Herdr server. The module takes its sender as
//! a trait, so both halves are exercised against fakes answering exactly what a
//! measured Herdr answers: the five delivery reasons, and a notification that
//! cannot be sent at all.
//!
//! The pane half is real where it can be. [`send`] writes its file into the
//! process temp directory, so the tests that reach it read the file back
//! through the path the fake was handed, and remove it afterwards. The only
//! thing standing in for a live server is the transport.

#![cfg(feature = "report")]

use std::path::PathBuf;

use herdr_plugin_kit::api::generated::{
    NotificationShowParams, NotificationShowReason, PluginPaneOpenParams, PluginPanePlacement,
};
use herdr_plugin_kit::report::{
    send, Kind, Report, ENTRYPOINT, FILE_VAR, HEADING_VAR, HEIGHT, WIDTH,
};
use herdr_plugin_kit::surface::{OpenError, Transport};

const PLUGIN: &str = "mikebronner.test-plugin";

/// A sender that records what it was handed and answers what it was told to.
struct Fake {
    /// What `notification.show` answers, or `Err` when it cannot be sent.
    notification: Result<NotificationShowReason, String>,
    /// What `plugin.pane.open` answers.
    open: Result<(), OpenError>,
    opened: Vec<PluginPaneOpenParams>,
    notified: Vec<NotificationShowParams>,
}

impl Fake {
    fn answering(reason: NotificationShowReason) -> Fake {
        Fake {
            notification: Ok(reason),
            open: Ok(()),
            opened: Vec::new(),
            notified: Vec::new(),
        }
    }

    /// A notification that could not be sent at all.
    fn unreachable() -> Fake {
        let mut fake = Fake::answering(NotificationShowReason::Shown);
        fake.notification = Err("socket is gone".to_string());
        fake
    }

    /// Every file this fake was handed, so a test can clean up after itself.
    fn files(&self) -> Vec<PathBuf> {
        self.opened
            .iter()
            .filter_map(|params| params.env.get(FILE_VAR))
            .map(PathBuf::from)
            .collect()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        for path in self.files() {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Transport for Fake {
    fn open_pane(&mut self, params: PluginPaneOpenParams) -> Result<(), OpenError> {
        self.opened.push(params);
        self.open.clone()
    }

    fn show_notification(
        &mut self,
        params: NotificationShowParams,
    ) -> Result<NotificationShowReason, String> {
        self.notified.push(params);
        self.notification.clone()
    }
}

fn cosmetic() -> Report<'static> {
    Report {
        title: "Two plugins are stale",
        summary: "2 issues found",
        detail: "recent-spaces is 3 releases behind\nproject-finder is 1 release behind",
        kind: Kind::Cosmetic,
    }
}

fn diagnostic() -> Report<'static> {
    Report {
        kind: Kind::Diagnostic,
        ..cosmetic()
    }
}

/// Sends `report` and answers whether a pane was opened, with the reason.
fn run(fake: &mut Fake, report: &Report<'_>) -> (Result<NotificationShowReason, String>, bool) {
    let answer = send(fake, PLUGIN, report);
    (answer, !fake.opened.is_empty())
}

// The policy table of SCOPE.md §7.2, one test per row.

#[test]
fn a_shown_notification_opens_no_pane() {
    let mut fake = Fake::answering(NotificationShowReason::Shown);
    let (answer, opened) = run(&mut fake, &cosmetic());
    assert_eq!(answer, Ok(NotificationShowReason::Shown));
    assert!(!opened, "a delivered message needs no pane");
}

#[test]
fn a_shown_diagnostic_opens_no_pane_either() {
    let mut fake = Fake::answering(NotificationShowReason::Shown);
    let (_, opened) = run(&mut fake, &diagnostic());
    assert!(!opened, "delivery is delivery whatever the message is");
}

#[test]
fn no_foreground_client_falls_back() {
    let mut fake = Fake::answering(NotificationShowReason::NoForegroundClient);
    let (answer, opened) = run(&mut fake, &cosmetic());
    assert_eq!(answer, Ok(NotificationShowReason::NoForegroundClient));
    assert!(opened, "nothing was there to draw it");
}

#[test]
fn a_rate_limited_notification_falls_back() {
    let mut fake = Fake::answering(NotificationShowReason::RateLimited);
    let (_, opened) = run(&mut fake, &cosmetic());
    assert!(opened, "the message was dropped rather than shown");
}

#[test]
fn a_busy_notification_falls_back() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    let (_, opened) = run(&mut fake, &cosmetic());
    assert!(opened, "one toast is live at a time, and this was not it");
}

#[test]
fn a_disabled_cosmetic_message_is_respected() {
    let mut fake = Fake::answering(NotificationShowReason::Disabled);
    let (answer, opened) = run(&mut fake, &cosmetic());
    assert_eq!(answer, Ok(NotificationShowReason::Disabled));
    assert!(
        !opened,
        "a user who turns off toasts has said something about toasts"
    );
}

#[test]
fn a_disabled_diagnostic_is_overridden() {
    let mut fake = Fake::answering(NotificationShowReason::Disabled);
    let (_, opened) = run(&mut fake, &diagnostic());
    assert!(
        opened,
        "the preference is about toasts, not about a plugin that cannot work"
    );
}

#[test]
fn a_notification_that_cannot_be_sent_falls_back_and_still_reports_the_failure() {
    let mut fake = Fake::unreachable();
    let (answer, opened) = run(&mut fake, &cosmetic());
    assert_eq!(answer, Err("socket is gone".to_string()));
    assert!(opened, "non-delivery is certain rather than reported here");
}

// What the caller is handed, and what it is not.

#[test]
fn every_reason_reaches_the_caller_rather_than_a_bare_success() {
    for reason in [
        NotificationShowReason::Shown,
        NotificationShowReason::Disabled,
        NotificationShowReason::RateLimited,
        NotificationShowReason::NoForegroundClient,
        NotificationShowReason::Busy,
    ] {
        let mut fake = Fake::answering(reason);
        assert_eq!(send(&mut fake, PLUGIN, &cosmetic()), Ok(reason));
    }
}

#[test]
fn a_pane_that_will_not_open_is_silent_and_changes_no_answer() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    fake.open = Err(OpenError::Failed("no room".to_string()));
    let answer = send(&mut fake, PLUGIN, &cosmetic());
    assert_eq!(
        answer,
        Ok(NotificationShowReason::Busy),
        "stderr already carries the message, so the failure is deliberately silent"
    );
}

#[test]
fn a_busy_pane_is_silent_too() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    fake.open = Err(OpenError::Busy);
    assert_eq!(
        send(&mut fake, PLUGIN, &cosmetic()),
        Ok(NotificationShowReason::Busy)
    );
}

// The notification itself.

#[test]
fn the_notification_carries_the_title_and_the_summary() {
    let mut fake = Fake::answering(NotificationShowReason::Shown);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();

    assert_eq!(fake.notified.len(), 1);
    let params = &fake.notified[0];
    assert_eq!(params.title, "Two plugins are stale");
    assert_eq!(params.body.as_deref(), Some("2 issues found"));
}

#[test]
fn exactly_one_notification_is_sent_whatever_the_answer() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();
    assert_eq!(
        fake.notified.len(),
        1,
        "a fallback opens a pane rather than sending a second toast"
    );
}

// The pane.

#[test]
fn the_pane_reads_the_detail_from_a_file_it_was_handed() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();

    let params = &fake.opened[0];
    let path = params.env.get(FILE_VAR).expect("no file variable");
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "recent-spaces is 3 releases behind\nproject-finder is 1 release behind",
        "the pane carries every issue at once, which is why it exists"
    );
}

#[test]
fn the_pane_is_headed_with_the_same_title_the_notification_used() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();

    assert_eq!(
        fake.opened[0].env.get(HEADING_VAR).map(String::as_str),
        Some("Two plugins are stale")
    );
}

#[test]
fn the_pane_request_names_the_fixed_entrypoint_and_the_calling_plugin() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();

    let params = &fake.opened[0];
    assert_eq!(params.entrypoint, ENTRYPOINT);
    assert_eq!(params.entrypoint, "issues");
    assert_eq!(params.plugin_id, PLUGIN);
}

#[test]
fn the_two_environment_variables_are_the_documented_names() {
    assert_eq!(FILE_VAR, "HERDR_PLUGIN_ISSUES_FILE");
    assert_eq!(HEADING_VAR, "HERDR_PLUGIN_ISSUES_HEADING");
}

#[test]
fn the_pane_is_a_popup_over_the_tab_rather_than_a_split() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();

    let params = &fake.opened[0];
    assert_eq!(params.placement, Some(PluginPanePlacement::Popup));
    assert_eq!(params.direction, None);
    assert_eq!(params.target_pane_id, None);
    assert_eq!(params.workspace_id, None);
    assert!(params.focus);
}

#[test]
fn both_requested_sizes_survive_into_the_request() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();

    let params = &fake.opened[0];
    assert!(
        params.width.is_some(),
        "{} did not survive as a size",
        WIDTH
    );
    assert!(
        params.height.is_some(),
        "{} did not survive as a size",
        HEIGHT
    );
}

#[test]
fn two_reports_from_one_process_write_two_different_files() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();
    send(
        &mut fake,
        PLUGIN,
        &Report {
            detail: "a third thing",
            ..cosmetic()
        },
    )
    .unwrap();

    let files = fake.files();
    assert_eq!(files.len(), 2);
    assert_ne!(
        files[0], files[1],
        "pid alone failed a suite about one run in four"
    );
    assert_eq!(std::fs::read_to_string(&files[1]).unwrap(), "a third thing");
}

#[test]
fn the_file_outlives_the_call_because_the_popup_reads_it_afterwards() {
    let mut fake = Fake::answering(NotificationShowReason::Busy);
    send(&mut fake, PLUGIN, &cosmetic()).unwrap();

    let path = &fake.files()[0];
    assert!(
        path.exists(),
        "deleting it here would race the popup that was just asked to read it"
    );
}
