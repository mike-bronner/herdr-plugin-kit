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
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use herdr_plugin_kit::api::generated::{InstalledPluginInfo, PluginSourceInfo, PluginSourceKind};
use herdr_plugin_kit::update::{
    apply, check, check_and_save, due, install_arguments, is_managed, offer, record_offer,
    Available, Decision, Installer, Releases, Skipped, DEFAULT_INTERVAL,
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
