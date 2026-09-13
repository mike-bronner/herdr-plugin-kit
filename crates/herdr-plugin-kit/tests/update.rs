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
    apply, check, install_arguments, is_managed, Available, Decision, Installer, Releases, Skipped,
    DEFAULT_INTERVAL,
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
