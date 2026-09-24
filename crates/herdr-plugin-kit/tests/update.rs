//! What the update module promises.
//!
//! 🔑 **Two seams are driven and everything else is real.** The network and the
//! process spawn are traits, because a test that stood up an HTTP server or ran
//! Herdr would be testing those rather than this. Everything else — the stamp
//! file, the clock, the decision — is exercised as it ships: the stamp is a
//! real file in a real temporary directory, and the clock is a parameter.
//!
//! ⚠️ **A fake filesystem would prove nothing here**, because the stamp's whole
//! job is to survive between two processes. A fake clock would be the same
//! mistake in a different place, which is why `now` is an argument rather than
//! a trait: the caller already has one.

#![cfg(feature = "update")]

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{GenericFilePath, ListenerOptions, ToFsName as _};
use serde_json::{json, Value};

use herdr_plugin_kit::api::client::{Client, Socket};
use herdr_plugin_kit::api::generated::{InstalledPluginInfo, PluginSourceInfo, PluginSourceKind};
use herdr_plugin_kit::update::{
    apply, check, check_and_save, due, install_arguments, is_managed, lookup, managed, offer,
    record_offer, run_check, spawn_check_if_due, Available, Decision, Files, Installer, Releases,
    Skipped, DEFAULT_INTERVAL,
};

/// A directory that removes itself, so a failing test leaks nothing.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "herdr-plugin-kit-update-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        TempDir(path)
    }

    fn stamp(&self) -> PathBuf {
        self.0.join("update-stamp")
    }

    fn result(&self) -> PathBuf {
        self.0.join("update-result.json")
    }

    fn offered(&self) -> PathBuf {
        self.0.join("update-offered")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A release reader whose answer the test chooses.
struct Answers {
    answer: Result<Option<String>, String>,
    asked: RefCell<Vec<String>>,
}

impl Answers {
    fn newest(tag: &str) -> Answers {
        Answers {
            answer: Ok(Some(tag.to_string())),
            asked: RefCell::new(Vec::new()),
        }
    }

    fn none() -> Answers {
        Answers {
            answer: Ok(None),
            asked: RefCell::new(Vec::new()),
        }
    }

    fn refused(reason: &str) -> Answers {
        Answers {
            answer: Err(reason.to_string()),
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl Releases for Answers {
    fn latest(&self, owner: &str, repo: &str) -> Result<Option<String>, String> {
        self.asked.borrow_mut().push(format!("{}/{}", owner, repo));
        self.answer.clone()
    }
}

/// An installer that records what it was told to run.
#[derive(Default)]
struct Spawns {
    ran: Vec<Vec<String>>,
    refuse: Option<String>,
}

impl Installer for Spawns {
    fn install(&mut self, owner: &str, repo: &str, tag: &str) -> Result<(), String> {
        self.ran.push(install_arguments(owner, repo, tag));
        match &self.refuse {
            Some(reason) => Err(reason.clone()),
            None => Ok(()),
        }
    }
}

fn plugin(kind: PluginSourceKind, version: &str) -> InstalledPluginInfo {
    InstalledPluginInfo {
        actions: Vec::new(),
        build: Vec::new(),
        description: None,
        enabled: true,
        events: Vec::new(),
        link_handlers: Vec::new(),
        manifest_path: "/plugins/p/herdr-plugin.toml".to_string(),
        min_herdr_version: "0.9.0".to_string(),
        name: "Project Finder".to_string(),
        panes: Vec::new(),
        platforms: None,
        plugin_id: "mikebronner.project-finder".to_string(),
        plugin_root: "/plugins/p".to_string(),
        source: PluginSourceInfo {
            installed_unix_ms: None,
            kind,
            managed_path: None,
            owner: Some("mike-bronner".to_string()),
            repo: Some("herdr-plugin-project-finder".to_string()),
            requested_ref: Some(version.to_string()),
            resolved_commit: None,
            subdir: None,
        },
        startup: Vec::new(),
        version: version.to_string(),
        warnings: Vec::new(),
    }
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

// ------------------------------------------------------- the local guard

#[test]
fn a_local_install_is_never_checked() {
    // 🚨 SCOPE.md §8.3. An updater running against a working tree is hostile:
    // a developer who asked to compile silently gets somebody else's binary.
    let directory = TempDir::new("local");
    let releases = Answers::newest("9.9.9");

    let decision = check(
        &plugin(PluginSourceKind::Local, "0.8.1"),
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(decision, Decision::Skipped(Skipped::LocalInstall));
    // Nothing was asked, and nothing was written: a local install leaves no
    // trace at all, so it cannot even consume the day's attempt.
    assert!(releases.asked.borrow().is_empty());
    assert!(!directory.stamp().exists());
}

#[test]
fn a_local_install_is_refused_by_apply_as_well() {
    // The two entry points can be called independently, so the guard is in
    // both. A caller holding an Available must not be able to spend it here.
    let mut spawns = Spawns::default();
    let update = Available {
        installed: "0.8.1".to_string(),
        tag: "0.9.1".to_string(),
        repository: "mike-bronner/herdr-plugin-project-finder".to_string(),
    };

    let refused = apply(
        &plugin(PluginSourceKind::Local, "0.8.1"),
        &update,
        &mut spawns,
    );

    assert!(refused.is_err());
    assert!(refused.unwrap_err().contains("local install"));
    assert!(spawns.ran.is_empty(), "nothing may be spawned");
}

#[test]
fn only_a_github_install_is_managed() {
    assert!(is_managed(&plugin(PluginSourceKind::Github, "0.8.1")));
    assert!(!is_managed(&plugin(PluginSourceKind::Local, "0.8.1")));
}

#[test]
fn a_github_install_naming_no_repository_is_skipped_rather_than_guessed() {
    let directory = TempDir::new("norepo");
    let releases = Answers::newest("0.9.1");
    let mut nameless = plugin(PluginSourceKind::Github, "0.8.1");
    nameless.source.repo = None;

    let decision = check(
        &nameless,
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(decision, Decision::Skipped(Skipped::NoRepository));
    assert!(releases.asked.borrow().is_empty());
}

#[test]
fn an_empty_owner_or_repository_is_skipped_as_well() {
    // ⚠️ A present-but-empty string is a different shape from an absent one,
    // and `Option` does not tell them apart. Without this, a plugin whose
    // record carries `""` is asked about as `https://api.github.com/repos///…`.
    for (owner, repo) in [
        (Some(String::new()), Some("r".to_string())),
        (Some("o".to_string()), Some(String::new())),
    ] {
        let directory = TempDir::new("emptyrepo");
        let releases = Answers::newest("0.9.1");
        let mut nameless = plugin(PluginSourceKind::Github, "0.8.1");
        nameless.source.owner = owner;
        nameless.source.repo = repo;

        let decision = check(
            &nameless,
            &directory.stamp(),
            at(1_000_000),
            DEFAULT_INTERVAL,
            &releases,
        );

        assert_eq!(decision, Decision::Skipped(Skipped::NoRepository));
        assert!(releases.asked.borrow().is_empty());
    }
}

// ------------------------------------------------------------- the timer

#[test]
fn a_refused_call_still_records_the_attempt() {
    // 🚨 The finding this module exists to encode. A 403 that left the timer
    // unadvanced turns one attempt a day into one attempt a launch, against a
    // budget of 60 an hour that is shared per IP and already exhausted.
    //
    // ⚠️ **Renamed 2026-09-13 from `the_stamp_is_written_before_the_call_
    // rather_than_after`**, which promised an ordering nothing can observe:
    // moving `stamp_attempt` below the call still writes the stamp on every
    // path, so no test could tell. What is observable, and what the 403 defect
    // actually needs, is that a *refused* call records the attempt. SCOPE.md
    // §15.7 carries the finding.
    let directory = TempDir::new("stamp-on-failure");
    let releases = Answers::refused("403 rate limit exceeded");

    let decision = check(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert!(matches!(decision, Decision::NoAnswer(_)));
    assert!(
        directory.stamp().exists(),
        "a refused call still records the attempt, or the next launch retries",
    );
}

#[test]
fn a_second_check_inside_the_interval_asks_nothing() {
    let directory = TempDir::new("too-soon");
    let subject = plugin(PluginSourceKind::Github, "0.8.1");
    let releases = Answers::refused("403 rate limit exceeded");

    check(
        &subject,
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );
    let again = check(
        &subject,
        &directory.stamp(),
        at(1_000_000 + 60),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(again, Decision::Skipped(Skipped::TooSoon));
    assert_eq!(releases.asked.borrow().len(), 1, "asked once, not twice");
}

#[test]
fn a_check_after_the_interval_asks_again() {
    let directory = TempDir::new("due");
    let subject = plugin(PluginSourceKind::Github, "0.8.1");
    let releases = Answers::newest("0.8.1");

    check(
        &subject,
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );
    let later = at(1_000_000 + DEFAULT_INTERVAL.as_secs());
    let again = check(
        &subject,
        &directory.stamp(),
        later,
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(again, Decision::UpToDate);
    assert_eq!(releases.asked.borrow().len(), 2);
}

#[test]
fn an_absent_or_unreadable_stamp_means_check_rather_than_stay_silent() {
    let directory = TempDir::new("unreadable");
    let subject = plugin(PluginSourceKind::Github, "0.8.1");
    let releases = Answers::newest("0.8.1");

    // Absent.
    assert_eq!(
        check(
            &subject,
            &directory.stamp(),
            at(10),
            DEFAULT_INTERVAL,
            &releases
        ),
        Decision::UpToDate
    );
    // Present and not a timestamp, which is the same question and must not
    // become "never check again".
    std::fs::write(directory.stamp(), "not a number\n").unwrap();
    assert_eq!(
        check(
            &subject,
            &directory.stamp(),
            at(20),
            DEFAULT_INTERVAL,
            &releases
        ),
        Decision::UpToDate
    );
    assert_eq!(releases.asked.borrow().len(), 2);
}

// ---------------------------------------------------------- the decision

#[test]
fn a_newer_release_is_available_and_names_both_versions() {
    let directory = TempDir::new("available");
    let releases = Answers::newest("0.9.1");

    let decision = check(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(
        decision,
        Decision::Available(Available {
            installed: "0.8.1".to_string(),
            tag: "0.9.1".to_string(),
            repository: "mike-bronner/herdr-plugin-project-finder".to_string(),
        })
    );
}

#[test]
fn the_installed_tag_is_up_to_date() {
    let directory = TempDir::new("current");
    let releases = Answers::newest("0.8.1");

    let decision = check(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(decision, Decision::UpToDate);
}

#[test]
fn a_repository_with_no_releases_is_up_to_date_rather_than_a_failure() {
    // Having no releases is a fact about the repository. Failing to ask is not.
    let directory = TempDir::new("none");
    let releases = Answers::none();

    let decision = check(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(decision, Decision::UpToDate);
}

#[test]
fn a_refusal_is_never_up_to_date() {
    // 🚨 The distinction the whole Decision type exists for. A shared gateway
    // can exhaust 60-per-hour invisibly, and "nobody answered" read as "you are
    // current" is an update that never happens and never says why.
    let directory = TempDir::new("refused");

    for reason in ["403 rate limit exceeded", "timed out", "not JSON"] {
        let releases = Answers::refused(reason);
        let _ = std::fs::remove_file(directory.stamp());
        let decision = check(
            &plugin(PluginSourceKind::Github, "0.8.1"),
            &directory.stamp(),
            at(1_000_000),
            DEFAULT_INTERVAL,
            &releases,
        );

        assert_ne!(decision, Decision::UpToDate, "{}", reason);
        match decision {
            Decision::NoAnswer(said) => assert!(said.contains(reason), "{}", said),
            other => panic!("expected a non-answer, got {:?}", other),
        }
    }
}

/// What `check` decides for an install of `installed` against a newest
/// release tagged `tag`.
fn decided(installed: &str, tag: &str) -> Decision {
    let directory = TempDir::new("precedence");
    check(
        &plugin(PluginSourceKind::Github, installed),
        &directory.stamp(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &Answers::newest(tag),
    )
}

#[test]
fn a_v_prefixed_tag_of_the_installed_version_is_up_to_date() {
    // 🚨 Reported by agentic-panes-layout against 0.5.0: `v0.4.0` against an
    // installed 0.4.0 offered a pointless reinstall. Tags cut before
    // 2026-09-10 carry the `v` and were never rewritten (SCOPE.md §12.1).
    assert_eq!(decided("0.4.0", "v0.4.0"), Decision::UpToDate);
    assert_eq!(decided("v0.4.0", "0.4.0"), Decision::UpToDate);
}

#[test]
fn an_older_release_is_never_offered() {
    // 🚨 Reported by agentic-panes-layout against 0.5.0. `apply` passes the tag
    // as `--ref`, so offering this installs a downgrade.
    for (installed, tag) in [
        ("0.9.1", "0.9.0"),
        ("0.10.0", "0.9.0"),
        ("1.0.0", "v0.9.9"),
        ("0.9.1", "0.9.1-rc.1"),
    ] {
        assert_eq!(
            decided(installed, tag),
            Decision::UpToDate,
            "{} {}",
            installed,
            tag
        );
    }
}

#[test]
fn an_installed_prerelease_newer_than_the_latest_release_is_up_to_date() {
    // The likely way to meet a downgrade: `releases/latest` skips prereleases,
    // so an install of one sees an older release as the newest.
    assert_eq!(decided("1.0.0-rc.1", "0.9.1"), Decision::UpToDate);
}

#[test]
fn the_release_of_an_installed_prerelease_is_available() {
    assert_eq!(
        decided("0.9.1-rc.2", "0.9.1"),
        Decision::Available(Available {
            installed: "0.9.1-rc.2".to_string(),
            tag: "0.9.1".to_string(),
            repository: "mike-bronner/herdr-plugin-project-finder".to_string(),
        })
    );
}

#[test]
fn versions_are_compared_as_numbers_rather_than_text() {
    // As text, "0.10.0" sorts below "0.9.0" and "0.9.11" below "0.9.2".
    for (installed, tag) in [("0.9.0", "0.10.0"), ("0.9.2", "0.9.11")] {
        assert!(
            matches!(decided(installed, tag), Decision::Available(_)),
            "{} {}",
            installed,
            tag
        );
    }
}

#[test]
fn a_newer_v_prefixed_tag_is_available_and_keeps_its_v() {
    // 🔑 The `v` is ignored for the comparison only. `apply` passes the tag as
    // `--ref`, and the tag that exists in the repository is `v0.9.1`.
    match decided("0.8.1", "v0.9.1") {
        Decision::Available(update) => assert_eq!(update.tag, "v0.9.1"),
        other => panic!("expected an update, got {:?}", other),
    }
}

#[test]
fn build_metadata_does_not_make_a_release_newer() {
    // SemVer §10: build metadata is ignored when determining precedence.
    assert_eq!(decided("0.9.1", "0.9.1+build.7"), Decision::UpToDate);
    assert_eq!(
        decided("0.9.1+build.7", "0.9.1+build.8"),
        Decision::UpToDate
    );
}

#[test]
fn a_version_that_cannot_be_parsed_is_a_non_answer_never_up_to_date() {
    // 🚨 The rule `Decision` exists for, applied to a version: a question that
    // cannot be answered is not "no update", and it is not an update either.
    for (installed, tag, side) in [
        ("0.8.1", "nightly", "release tag"),
        ("0.8.1", "0.9", "release tag"),
        ("0.8.1", "v0.09.1", "release tag"),
        ("dev", "0.9.1", "installed version"),
        ("0.8.1.2", "0.9.1", "installed version"),
    ] {
        match decided(installed, tag) {
            Decision::NoAnswer(reason) => assert!(reason.contains(side), "{}", reason),
            other => panic!(
                "{} {}: expected a non-answer, got {:?}",
                installed, tag, other
            ),
        }
    }
}

// ------------------------------------------------------------- the apply

#[test]
fn the_refresh_passes_the_ref_explicitly() {
    // 🚨 Measured 2026-09-13 (§8.2.1): a refresh without --ref erases
    // requested_ref and silently converts a pinned install into a floating
    // one. An update moves a pin; it never deletes one.
    let mut spawns = Spawns::default();
    let update = Available {
        installed: "0.8.1".to_string(),
        tag: "0.9.1".to_string(),
        repository: "mike-bronner/herdr-plugin-project-finder".to_string(),
    };

    apply(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &update,
        &mut spawns,
    )
    .unwrap();

    assert_eq!(
        spawns.ran,
        vec![vec![
            "plugin".to_string(),
            "install".to_string(),
            "mike-bronner/herdr-plugin-project-finder".to_string(),
            "--ref".to_string(),
            "0.9.1".to_string(),
            "--yes".to_string(),
        ]]
    );
}

#[test]
fn the_arguments_name_the_tag_rather_than_the_installed_version() {
    let arguments = install_arguments("owner", "repo", "1.2.3");

    let position = arguments
        .iter()
        .position(|a| a == "--ref")
        .expect("--ref is passed");
    assert_eq!(arguments[position + 1], "1.2.3");
}

#[test]
fn a_refusal_from_herdr_reaches_the_caller() {
    let mut spawns = Spawns {
        refuse: Some("duplicate pane id 'picker'".to_string()),
        ..Spawns::default()
    };
    let update = Available {
        installed: "0.8.1".to_string(),
        tag: "0.9.0".to_string(),
        repository: "mike-bronner/herdr-plugin-project-finder".to_string(),
    };

    let refused = apply(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &update,
        &mut spawns,
    );

    // ✅ The real message from the 2026-09-13 run: a release carrying that
    // defect cannot be installed at all, so an update onto it must report
    // rather than silently leave the plugin where it was.
    assert_eq!(refused, Err("duplicate pane id 'picker'".to_string()));
}

#[test]
fn a_repository_that_is_not_owner_slash_repo_is_refused() {
    let mut spawns = Spawns::default();
    let update = Available {
        installed: "0.8.1".to_string(),
        tag: "0.9.1".to_string(),
        repository: "not-a-repository".to_string(),
    };

    let refused = apply(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &update,
        &mut spawns,
    );

    assert!(refused.is_err());
    assert!(spawns.ran.is_empty());
}

#[test]
fn a_repository_with_an_empty_half_is_refused_too() {
    // ⚠️ `"owner/"` splits successfully and yields an empty repo, so the split
    // alone is not the check. Herdr would be asked to install `owner/`.
    for repository in ["owner/", "/repo"] {
        let mut spawns = Spawns::default();
        let update = Available {
            installed: "0.8.1".to_string(),
            tag: "0.9.1".to_string(),
            repository: repository.to_string(),
        };

        let refused = apply(
            &plugin(PluginSourceKind::Github, "0.8.1"),
            &update,
            &mut spawns,
        );

        assert!(refused.is_err(), "{} was accepted", repository);
        assert!(spawns.ran.is_empty());
    }
}

// ------------------------------------------- carrying it to the next launch

/// The update `check` finds for [`plugin`] at 0.8.1 against a 0.9.1 release.
fn found() -> Available {
    Available {
        installed: "0.8.1".to_string(),
        tag: "0.9.1".to_string(),
        repository: "mike-bronner/herdr-plugin-project-finder".to_string(),
    }
}

/// A detached check that found `found()`, run at `at(1_000_000)`.
fn saved_by_a_detached_check(directory: &TempDir) {
    let decision = check_and_save(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        &directory.result(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &Answers::newest("0.9.1"),
    );
    assert_eq!(decision, Decision::Available(found()));
}

fn offered_now(directory: &TempDir, version: &str) -> Option<Available> {
    offer(
        &plugin(PluginSourceKind::Github, version),
        &directory.result(),
        &directory.offered(),
        at(1_000_000 + 60),
        DEFAULT_INTERVAL,
    )
}

#[test]
fn a_found_update_is_offered_on_a_later_launch() {
    // 🔑 SCOPE.md §8.2: spawn detached, write the result, offer on the next
    // launch. `offer` takes no `Releases` at all, so the later launch cannot
    // make a network call.
    let directory = TempDir::new("carried");
    saved_by_a_detached_check(&directory);

    assert_eq!(offered_now(&directory, "0.8.1"), Some(found()));
}

#[test]
fn saving_the_result_leaves_the_attempt_stamp_in_its_old_format() {
    // 🚨 The stamp is the rate-limit guard, and stamps already on users'
    // machines must keep working. A result written into it would also let a
    // late result overwrite a newer attempt time.
    let directory = TempDir::new("stamp-format");
    saved_by_a_detached_check(&directory);

    assert_eq!(
        std::fs::read_to_string(directory.stamp()).unwrap(),
        "1000000\n"
    );
    assert_ne!(directory.result(), directory.stamp());
}

#[test]
fn a_refused_call_still_records_the_attempt_when_the_result_is_saved() {
    let directory = TempDir::new("save-stamp-on-failure");

    let decision = check_and_save(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        &directory.result(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &Answers::refused("403 rate limit exceeded"),
    );

    assert!(matches!(decision, Decision::NoAnswer(_)));
    assert_eq!(
        std::fs::read_to_string(directory.stamp()).unwrap(),
        "1000000\n"
    );
}

#[test]
fn an_update_found_before_the_plugin_was_upgraded_is_not_offered() {
    // 🚨 The saved answer describes an install that no longer exists. Upgraded
    // to exactly the offered tag, and upgraded past it, are both stale.
    //
    // ⚠️ Those two are refused by precedence as well, since 0.5.1, so they no
    // longer reach the installed-version test on their own. An install moved
    // to a version the tag is still newer than does, and it is stale too: the
    // saved `installed` would name a version that is no longer there.
    for version in ["0.9.1", "0.9.5", "0.8.2", "0.8.0"] {
        let directory = TempDir::new("stale");
        saved_by_a_detached_check(&directory);

        assert_eq!(offered_now(&directory, version), None, "{}", version);
    }
}

#[test]
fn a_saved_result_that_is_not_newer_is_not_offered() {
    // 🚨 0.5.0 compared strings, so a result it saved can hold a reinstall or
    // a downgrade. `offer` makes no network call, and re-checking the saved
    // pair is the only way it can refuse one.
    for tag in ["v0.8.1", "0.8.1", "0.8.0", "0.8.1-rc.1", "nightly"] {
        let directory = TempDir::new("not-newer");
        let saved = Available {
            tag: tag.to_string(),
            ..found()
        };
        std::fs::write(directory.result(), serde_json::to_string(&saved).unwrap()).unwrap();

        assert_eq!(offered_now(&directory, "0.8.1"), None, "{}", tag);
    }
}

#[test]
fn a_result_that_is_absent_unreadable_or_corrupt_is_not_offered() {
    let directory = TempDir::new("corrupt");

    // Absent.
    assert_eq!(offered_now(&directory, "0.8.1"), None);

    // Not JSON, JSON of the wrong shape, and a torn write.
    for text in [
        "not json at all",
        r#"{"installed":"0.8.1","tag":"0.9.1"}"#,
        r#"{"installed":"0.8.1","tag":"0.9.1","repository":"mike-bro"#,
    ] {
        std::fs::write(directory.result(), text).unwrap();
        assert_eq!(offered_now(&directory, "0.8.1"), None, "{}", text);
    }

    // Unreadable: a directory where the file should be.
    std::fs::remove_file(directory.result()).unwrap();
    std::fs::create_dir(directory.result()).unwrap();
    assert_eq!(offered_now(&directory, "0.8.1"), None);
}

#[test]
fn a_result_saved_for_another_repository_is_not_offered() {
    // ⚠️ Two plugins handed the same result path must not offer each other's
    // update, which `apply` would then install under this plugin's name.
    let directory = TempDir::new("foreign");
    let mut foreign = found();
    foreign.repository = "mike-bronner/herdr-plugin-recent-spaces".to_string();
    std::fs::write(directory.result(), serde_json::to_string(&foreign).unwrap()).unwrap();

    assert_eq!(offered_now(&directory, "0.8.1"), None);
}

#[test]
fn a_saved_result_is_never_offered_to_a_local_install() {
    // 🚨 SCOPE.md §8.3, the third entry point to carry the guard.
    let directory = TempDir::new("local-offer");
    saved_by_a_detached_check(&directory);

    let offered = offer(
        &plugin(PluginSourceKind::Local, "0.8.1"),
        &directory.result(),
        &directory.offered(),
        at(1_000_000 + 60),
        DEFAULT_INTERVAL,
    );

    assert_eq!(offered, None);
}

#[test]
fn a_later_up_to_date_or_non_answer_clears_the_older_result() {
    // 🚨 An Available left standing after a newer answer would be offered as
    // if it were still true.
    for (name, releases) in [
        ("up to date", Answers::newest("0.8.1")),
        ("the same version under a v", Answers::newest("v0.8.1")),
        ("an older release", Answers::newest("0.8.0")),
        ("a tag that is not a version", Answers::newest("nightly")),
        ("no releases", Answers::none()),
        ("no answer", Answers::refused("403 rate limit exceeded")),
    ] {
        let directory = TempDir::new("cleared");
        saved_by_a_detached_check(&directory);

        check_and_save(
            &plugin(PluginSourceKind::Github, "0.8.1"),
            &directory.stamp(),
            &directory.result(),
            at(1_000_000 + DEFAULT_INTERVAL.as_secs()),
            DEFAULT_INTERVAL,
            &releases,
        );

        assert!(!directory.result().exists(), "{}", name);
        assert_eq!(offered_now(&directory, "0.8.1"), None, "{}", name);
    }
}

#[test]
fn a_skipped_check_keeps_the_last_answer() {
    // A check inside the interval asked nothing, so it has nothing to replace
    // the last answer with.
    let directory = TempDir::new("kept");
    saved_by_a_detached_check(&directory);

    let decision = check_and_save(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        &directory.result(),
        at(1_000_000 + 60),
        DEFAULT_INTERVAL,
        &Answers::refused("must not be asked"),
    );

    assert_eq!(decision, Decision::Skipped(Skipped::TooSoon));
    assert_eq!(offered_now(&directory, "0.8.1"), Some(found()));
}

#[test]
fn a_newer_find_replaces_the_older_one() {
    let directory = TempDir::new("replaced");
    saved_by_a_detached_check(&directory);

    check_and_save(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        &directory.result(),
        at(1_000_000 + DEFAULT_INTERVAL.as_secs()),
        DEFAULT_INTERVAL,
        &Answers::newest("0.9.2"),
    );

    assert_eq!(
        offered_now(&directory, "0.8.1").map(|update| update.tag),
        Some("0.9.2".to_string())
    );
}

#[test]
fn a_shown_or_declined_offer_is_asked_again_only_after_the_interval() {
    let directory = TempDir::new("declined");
    saved_by_a_detached_check(&directory);
    let subject = plugin(PluginSourceKind::Github, "0.8.1");
    let shown = 2_000_000;

    record_offer(&directory.offered(), at(shown));

    let asked_at = |seconds| {
        offer(
            &subject,
            &directory.result(),
            &directory.offered(),
            at(seconds),
            DEFAULT_INTERVAL,
        )
    };
    assert_eq!(asked_at(shown + 60), None, "just declined");
    assert_eq!(
        asked_at(shown + DEFAULT_INTERVAL.as_secs() - 1),
        None,
        "one second short"
    );
    assert_eq!(
        asked_at(shown + DEFAULT_INTERVAL.as_secs()),
        Some(found()),
        "the interval has passed"
    );
}

#[test]
fn recording_an_offer_touches_neither_the_stamp_nor_the_result() {
    // 🔑 One writer per file. The launch writes only the offer record.
    let directory = TempDir::new("one-writer");
    saved_by_a_detached_check(&directory);
    let result = std::fs::read_to_string(directory.result()).unwrap();

    record_offer(&directory.offered(), at(2_000_000));

    assert_eq!(
        std::fs::read_to_string(directory.stamp()).unwrap(),
        "1000000\n"
    );
    assert_eq!(std::fs::read_to_string(directory.result()).unwrap(), result);
    assert_eq!(
        std::fs::read_to_string(directory.offered()).unwrap(),
        "2000000\n"
    );
}

#[test]
fn due_answers_without_a_network_call() {
    // 🔑 Public so a launch can decide whether to spawn a detached check. It
    // reads a stamp written by an older kit the same way.
    let directory = TempDir::new("due-public");
    std::fs::write(directory.stamp(), "1000000\n").unwrap();

    assert!(!due(
        &directory.stamp(),
        at(1_000_000 + 60),
        DEFAULT_INTERVAL
    ));
    assert!(due(
        &directory.stamp(),
        at(1_000_000 + DEFAULT_INTERVAL.as_secs()),
        DEFAULT_INTERVAL
    ));
    assert!(
        due(&directory.offered(), at(10), DEFAULT_INTERVAL),
        "absent"
    );
}

#[test]
fn a_result_that_cannot_be_written_does_not_stop_the_check() {
    // ⚠️ Like the stamp: the decision still reaches the caller.
    let directory = TempDir::new("unwritable");
    std::fs::create_dir(directory.result()).unwrap();

    let decision = check_and_save(
        &plugin(PluginSourceKind::Github, "0.8.1"),
        &directory.stamp(),
        &directory.result(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &Answers::newest("0.9.1"),
    );

    assert_eq!(decision, Decision::Available(found()));
    assert!(directory.stamp().exists());
}

// --------------------------------------------- the setup a plugin wires in

/// The state directory every test below names, as a plugin would.
const STATE_DIR: &str = ".project-finder-update";

/// [`plugin`], installed at `root`.
fn rooted(kind: PluginSourceKind, root: &Path) -> InstalledPluginInfo {
    InstalledPluginInfo {
        plugin_root: root.to_string_lossy().into_owned(),
        ..plugin(kind, "0.8.1")
    }
}

/// Every file under `directory`, so a test can say that nothing was written.
fn contents(directory: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(directory)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

#[test]
fn a_github_install_with_an_absolute_root_is_managed() {
    let (found, files) = managed(plugin(PluginSourceKind::Github, "0.8.1"), STATE_DIR)
        .expect("a GitHub install with a root is managed");

    assert_eq!(found.plugin_id, "mikebronner.project-finder");
    assert_eq!(files.dir(), Path::new("/plugins/p/.project-finder-update"));
}

#[test]
fn a_local_install_is_never_managed() {
    // 🚨 SCOPE.md §8.3, and the door every other call here goes through. A
    // linked working tree must never receive a file.
    assert!(managed(plugin(PluginSourceKind::Local, "0.8.1"), STATE_DIR).is_none());
}

#[test]
fn a_record_with_no_usable_root_is_never_managed() {
    // A root that is empty or blank, and a relative one, which would resolve
    // against whatever directory a hook happened to start in.
    for root in ["", "  ", "plugins/p", "./p"] {
        let record = InstalledPluginInfo {
            plugin_root: root.to_string(),
            ..plugin(PluginSourceKind::Github, "0.8.1")
        };
        assert!(managed(record, STATE_DIR).is_none(), "{:?}", root);
    }
}

#[test]
fn a_state_directory_that_could_leave_the_install_is_refused() {
    // 🚨 Joining an absolute path discards the root, and `..` climbs out of it.
    // Each of these would write outside the install.
    for name in ["", ".", "..", "a/b", "/tmp/elsewhere", "a/", "../x"] {
        assert!(
            managed(plugin(PluginSourceKind::Github, "0.8.1"), name).is_none(),
            "{:?}",
            name
        );
        assert_eq!(
            Files::under(Path::new("/plugins/p"), name),
            None,
            "{:?}",
            name
        );
    }
}

#[test]
fn the_files_keep_the_names_the_first_consumer_wrote() {
    // 🔑 agentic-panes-layout 0.5.0 wrote these names from its own copy. Moving
    // onto the kit must not strand a found update or restart an interval.
    let files = Files::under(Path::new("/root"), ".agent-layout-update").unwrap();
    let dir = Path::new("/root/.agent-layout-update");

    assert_eq!(files.stamp(), dir.join("checked"));
    assert_eq!(files.result(), dir.join("available.json"));
    assert_eq!(files.offered("dialog"), Some(dir.join("offered-dialog")));
    assert_eq!(files.offered("toast"), Some(dir.join("offered-toast")));
}

#[test]
fn an_offer_record_name_that_could_leave_the_directory_is_refused() {
    let files = Files::under(Path::new("/root"), STATE_DIR).unwrap();
    for name in ["", ".", "..", "a/b", "/tmp/x", "../checked"] {
        assert_eq!(files.offered(name), None, "{:?}", name);
    }
}

#[test]
fn one_channel_recording_an_offer_does_not_silence_another() {
    // 🔑 A toast pointing the user to a dialog must not start the dialog's
    // interval. So each channel keeps its own record.
    let directory = TempDir::new("channels");
    let files = Files::under(&directory.0, STATE_DIR).unwrap();
    let subject = rooted(PluginSourceKind::Github, &directory.0);
    check_and_save(
        &subject,
        &files.stamp(),
        &files.result(),
        at(1_000_000),
        DEFAULT_INTERVAL,
        &Answers::newest("0.9.1"),
    );
    let toast = files.offered("toast").unwrap();
    let dialog = files.offered("dialog").unwrap();

    record_offer(&toast, at(1_000_060));

    let now = at(1_000_120);
    assert_eq!(
        offer(&subject, &files.result(), &toast, now, DEFAULT_INTERVAL),
        None
    );
    assert_eq!(
        offer(&subject, &files.result(), &dialog, now, DEFAULT_INTERVAL),
        Some(found())
    );
}

// ------------------------------------------------- reading the install record

/// Keeps two peers in one test run from colliding on a path.
static NEXT_SOCKET: AtomicU32 = AtomicU32::new(0);

/// A Herdr that answers one `plugin.list` with `plugins`, over a real socket.
///
/// ⚠️ **A second copy of the transport suite's scripted server, cut down to
/// one answer.** Each integration test is its own crate, so the two cannot
/// share a helper without a support module neither needs anywhere else.
struct Herdr {
    path: PathBuf,
    worker: Option<JoinHandle<Option<Value>>>,
}

impl Herdr {
    fn listing(plugins: Vec<InstalledPluginInfo>) -> Herdr {
        let ordinal = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
        let leaf = format!("herdr-kit-update-{}-{}", std::process::id(), ordinal);
        let path = match cfg!(windows) {
            true => PathBuf::from(format!(r"\\.\pipe\{}", leaf)),
            false => std::env::temp_dir().join(format!("{}.sock", leaf)),
        };
        let listener = ListenerOptions::new()
            .name(path.clone().to_fs_name::<GenericFilePath>().unwrap())
            .create_sync()
            .expect("the scratch socket must be bindable");
        let plugins = serde_json::to_value(plugins).unwrap();

        let worker = std::thread::spawn(move || {
            let stream = listener.accept().ok()?;
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).ok()?;
            let request: Value = serde_json::from_str(&line).ok()?;
            let answer = json!({
                "id": request["id"],
                "result": {"type": "plugin_list", "plugins": plugins},
            });
            let mut writer = &stream;
            writer.write_all(format!("{}\n", answer).as_bytes()).ok()?;
            writer.flush().ok()?;
            Some(request)
        });
        Herdr {
            path,
            worker: Some(worker),
        }
    }

    fn client(&self) -> Client {
        Client::new(Socket::at(&self.path), "mikebronner.project-finder")
            .with_timeout(Duration::from_secs(2))
    }

    /// The request it was sent.
    fn request(&mut self) -> Value {
        self.worker
            .take()
            .unwrap()
            .join()
            .unwrap()
            .expect("the peer was asked")
    }
}

impl Drop for Herdr {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The record of another installed plugin.
fn somebody_else() -> InstalledPluginInfo {
    InstalledPluginInfo {
        plugin_id: "mikebronner.recent-spaces".to_string(),
        ..plugin(PluginSourceKind::Github, "1.0.0")
    }
}

#[test]
fn lookup_finds_this_plugin_and_asks_herdr_for_it_by_id() {
    let mut herdr = Herdr::listing(vec![
        somebody_else(),
        plugin(PluginSourceKind::Github, "0.8.1"),
    ]);

    let (found, files) = lookup(&herdr.client(), "mikebronner.project-finder", STATE_DIR)
        .expect("this plugin's record is in the list");

    assert_eq!(found.plugin_id, "mikebronner.project-finder");
    assert_eq!(files.dir(), Path::new("/plugins/p/.project-finder-update"));
    let request = herdr.request();
    assert_eq!(request["method"], "plugin.list");
    assert_eq!(request["params"]["plugin_id"], "mikebronner.project-finder");
}

#[test]
fn lookup_never_takes_another_plugins_record() {
    // ⚠️ A server that ignored the filter still cannot hand back somebody
    // else's install, which would put this plugin's files in its root.
    let herdr = Herdr::listing(vec![somebody_else()]);

    assert!(lookup(&herdr.client(), "mikebronner.project-finder", STATE_DIR).is_none());
}

#[test]
fn lookup_refuses_a_local_install() {
    let herdr = Herdr::listing(vec![plugin(PluginSourceKind::Local, "0.8.1")]);

    assert!(lookup(&herdr.client(), "mikebronner.project-finder", STATE_DIR).is_none());
}

#[test]
fn lookup_answers_none_when_herdr_does_not_answer() {
    let client = Client::new(
        Socket::at(std::env::temp_dir().join("herdr-kit-update-nobody.sock")),
        "mikebronner.project-finder",
    );

    assert!(lookup(&client, "mikebronner.project-finder", STATE_DIR).is_none());
}

// ------------------------------------------------------ the detached side

#[test]
fn run_check_saves_what_it_found_inside_the_install() {
    let directory = TempDir::new("run-check");
    let herdr = Herdr::listing(vec![rooted(PluginSourceKind::Github, &directory.0)]);
    let releases = Answers::newest("0.9.1");

    let decision = run_check(
        &herdr.client(),
        "mikebronner.project-finder",
        STATE_DIR,
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(decision, Some(Decision::Available(found())));
    let files = Files::under(&directory.0, STATE_DIR).unwrap();
    assert_eq!(std::fs::read_to_string(files.stamp()).unwrap(), "1000000\n");
    assert_eq!(
        offer(
            &rooted(PluginSourceKind::Github, &directory.0),
            &files.result(),
            &files.offered("dialog").unwrap(),
            at(1_000_060),
            DEFAULT_INTERVAL,
        ),
        Some(found())
    );
}

#[test]
fn run_check_writes_nothing_into_a_local_install() {
    // 🚨 The door, end to end. A linked working tree is refused before the
    // network is asked and before a directory is made.
    let directory = TempDir::new("run-check-local");
    let herdr = Herdr::listing(vec![rooted(PluginSourceKind::Local, &directory.0)]);
    let releases = Answers::newest("9.9.9");

    let decision = run_check(
        &herdr.client(),
        "mikebronner.project-finder",
        STATE_DIR,
        at(1_000_000),
        DEFAULT_INTERVAL,
        &releases,
    );

    assert_eq!(decision, None);
    assert!(releases.asked.borrow().is_empty());
    assert_eq!(contents(&directory.0), Vec::<PathBuf>::new());
}

#[test]
fn a_detached_check_is_not_spawned_inside_the_interval() {
    let directory = TempDir::new("spawn-too-soon");
    let files = Files::under(&directory.0, STATE_DIR).unwrap();
    std::fs::create_dir_all(files.dir()).unwrap();
    std::fs::write(files.stamp(), "1000000\n").unwrap();

    assert!(!spawn_check_if_due(
        &files,
        at(1_000_000 + 60),
        DEFAULT_INTERVAL
    ));
}

#[test]
#[cfg(unix)]
fn a_detached_check_is_spawned_once_the_interval_has_passed_and_then_reaped() {
    // ⚠️ This spawns the test binary itself, which answers the flag it does
    // not know with an error and exits. What the spawn passes is checked in
    // the module's own tests, against a script that writes it down.
    //
    // 🚨 Then it waits for this process to have no children left. A child
    // nobody waits for stays a zombie, listed under this process until it
    // exits, so the wait below never ends early for an unreaped one. It is
    // the only test in this file that starts a process.
    let directory = TempDir::new("spawn-due");
    let files = Files::under(&directory.0, STATE_DIR).unwrap();

    assert!(spawn_check_if_due(&files, at(1_000_000), DEFAULT_INTERVAL));
    // The launch writes nothing. The attempt stamp is the detached check's.
    assert!(!files.stamp().exists());

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut left = children();
    while !left.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        left = children();
    }
    assert_eq!(left, Vec::<String>::new(), "a child was never reaped");
}

/// This process's children, as `ps` lists them, zombies included, leaving out
/// the `ps` that is asking.
#[cfg(unix)]
fn children() -> Vec<String> {
    let me = std::process::id().to_string();
    let ps = std::process::Command::new("ps")
        .args(["-A", "-o", "pid=,ppid="])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("ps runs");
    let asking = ps.id().to_string();
    let listed = ps.wait_with_output().expect("ps answers");
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let (pid, parent) = (fields.next()?, fields.next()?);
            (parent == me && pid != asking).then(|| pid.to_string())
        })
        .collect()
}
