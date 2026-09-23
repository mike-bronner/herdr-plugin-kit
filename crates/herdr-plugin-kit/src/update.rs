//! Asks whether a newer release exists, and applies it when a user says so.
//!
//! Feature-gated. Nothing here reaches a plugin that does not ask for it, and
//! it costs no dependency: the network call shells out to `curl` (SCOPE.md
//! §9.8) and the apply step spawns Herdr's own CLI.
//!
//! # What this module decides, and what it refuses to decide
//!
//! 🔑 **It answers a value and acts only when told to**, which is this kit's
//! established shape rather than a new choice: [`crate::version`] formats a
//! report and never prints it, `dialog` answers `Shown` and lets the caller
//! act, and [`crate::api::client::Handshake::mismatch`] hands back a diagnosis.
//!
//! ⚠️ **So it does not prompt.** SCOPE.md §8.2 names `dialog::ask` as the
//! channel, and that is a statement about what a *plugin* calls. Prompting from
//! here would drag the `dialog` feature — and `crossterm` with it — into every
//! consumer that wanted updates, including a headless watcher. [`Decision`] is
//! what a plugin turns into a prompt, in one call it already knows how to make.
//!
//! # The three things that are easy to get wrong
//!
//! 🚨 **The stamp records the attempt, not the result.** ✅ The budget is 60
//! requests per hour, unauthenticated, **per IP rather than per machine**.
//! Three plugins on a daily timer spend 0.2% of it, so the plugin count is not
//! the risk. The risk is a 403 that leaves the timer unadvanced: one-per-day
//! becomes one-per-launch, against a budget that is already exhausted. Writing
//! the stamp first bounds that at the cost of one skipped day.
//!
//! 🚨 **A refusal is not "up to date".** [`Decision::NoAnswer`] exists so that a
//! 403, a timeout and an unreadable body cannot be mistaken for good news. A
//! shared gateway can exhaust the budget invisibly.
//!
//! 🚨 **A `local:` install is never refreshed.** SCOPE.md §8.3: an updater
//! running against a working tree is hostile, because a developer who asked to
//! compile silently gets a binary somebody else built. Herdr answers the
//! question authoritatively and `PluginSourceKind` **defaults to `Local`**,
//! which is the safe direction. The guard is checked three times, in
//! [`check`], [`offer`] and [`apply`], because each can be called alone.
//!
//! # Carrying a found update to the next launch
//!
//! SCOPE.md §8.2: the check runs detached, and the offer is made on a *later*
//! launch. So three files carry state between processes, and 🔑 **each has
//! exactly one writer**:
//!
//! | File | Written by | Holds |
//! |---|---|---|
//! | the attempt stamp | [`check`], before the network call | when a check was last attempted |
//! | the result | [`check_and_save`], after the answer | the [`Available`] it found, or nothing |
//! | the offer record | [`record_offer`], at the launch that offered it | when an offer was last shown or declined |
//!
//! 🚨 **One shared file would reopen the retry storm.** A result written after
//! the call could overwrite a newer attempt time written by another process,
//! and an attempt that no longer shows is an attempt that happens again.
//!
//! A launch reads the pair back through [`offer`], which makes no network call
//! and refuses anything it cannot trust: a result that is corrupt, one written
//! for another plugin, and one made stale because the plugin was upgraded
//! since. [`due`] answers whether a detached check is worth spawning at all.
//!
//! ⚠️ **The caller passes every path.** Nothing here reads
//! [`crate::env::PLUGIN_STATE_DIR_VAR`], because a Herdr event hook does not
//! receive it.

use std::fs;
use std::io;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::api::generated::{InstalledPluginInfo, PluginSourceKind};

/// How long to wait between checks, when a caller expresses no opinion.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How long the release lookup may take before it is a non-answer.
///
/// ⚠️ An HTTPS call needs its own bound. SCOPE.md §8.2 carries the reason from
/// the `ls-remote` analysis: a detached spawn that hangs leaks a process.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// What a check concluded.
///
/// 🔑 Four cases, because a caller that cannot tell them apart cannot behave
/// differently. "Nobody answered" and "you are current" are the pair that
/// matters most, and conflating them is how a rate limit reads as good news.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Nothing was asked, and this is why.
    Skipped(Skipped),
    /// The newest release is the one already installed.
    UpToDate,
    /// A newer release exists.
    Available(Available),
    /// The question could not be answered. **Not** an answer of "no update".
    NoAnswer(String),
}

/// Why a check asked nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skipped {
    /// The interval has not elapsed since the last attempt.
    TooSoon,
    /// 🚨 A `local:` install, which this module never refreshes (§8.3).
    LocalInstall,
    /// A `github:` install whose record names no owner or repo, so there is
    /// nothing to ask about. Fails closed rather than guessing a repository.
    NoRepository,
}

/// A newer release, and what applying it would do.
///
/// Serializable because [`check_and_save`] keeps it on disk for a later launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Available {
    /// What is installed now.
    pub installed: String,
    /// The tag of the newest release.
    pub tag: String,
    /// The repository it came from, as `owner/repo`.
    pub repository: String,
}

/// Where the newest release comes from.
///
/// 🔑 **A seam because the network is one**, not because the call is hard. A
/// test that drove a fake HTTP stack would prove something about the stack; a
/// test that drives this proves what the module does with an answer, including
/// the answers nobody can produce on demand — a 403, a timeout, a body that is
/// not JSON.
pub trait Releases {
    /// The newest release tag for `owner/repo`, or why there is no answer.
    ///
    /// ⚠️ **A repository with no releases answers `Ok(None)`, not an error.**
    /// Having none is a fact about the repository; failing to ask is not.
    fn latest(&self, owner: &str, repo: &str) -> Result<Option<String>, String>;
}

/// How the apply step runs Herdr.
///
/// 🔑 **A seam because spawning a process is one.** The real implementation
/// runs the CLI; a test drives this and asserts the arguments, which is the
/// only way to hold `--ref` in place (see [`apply`]).
pub trait Installer {
    /// Run the refresh for `owner/repo` at `tag`.
    fn install(&mut self, owner: &str, repo: &str, tag: &str) -> Result<(), String>;
}

/// Reads the newest release with `curl`.
///
/// ✅ No HTTP dependency, which is SCOPE.md §9.8's decision applied here: the
/// plugins carry zero HTTP stacks, and pulling one in to fetch one small
/// document per day is a bad trade.
#[derive(Debug, Clone)]
pub struct CurlReleases {
    timeout: Duration,
}

impl Default for CurlReleases {
    fn default() -> CurlReleases {
        CurlReleases {
            timeout: CHECK_TIMEOUT,
        }
    }
}

impl CurlReleases {
    /// A reader with a timeout of the caller's choosing.
    pub fn with_timeout(timeout: Duration) -> CurlReleases {
        CurlReleases { timeout }
    }
}

impl Releases for CurlReleases {
    fn latest(&self, owner: &str, repo: &str) -> Result<Option<String>, String> {
        let url = format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            owner, repo
        );
        // --fail turns a 403 or a 404 into a non-zero exit rather than a body
        // that parses to nothing, and the protocol floors stop a redirect
        // downgrading the transport. The same shape as codegen/sync_api.py.
        let finished = Command::new("curl")
            .args([
                "--proto",
                "=https",
                "--tlsv1.2",
                "--fail",
                "--silent",
                "--show-error",
            ])
            .args(["--max-time", &self.timeout.as_secs().to_string()])
            .args(["--header", "Accept: application/vnd.github+json"])
            .arg(&url)
            .output()
            .map_err(|error| format!("curl could not run: {}", error))?;

        if !finished.status.success() {
            return Err(format!(
                "{} answered nothing usable: {}",
                url,
                String::from_utf8_lossy(&finished.stderr).trim()
            ));
        }
        tag_of(&String::from_utf8_lossy(&finished.stdout))
    }
}

/// Reads `"tag_name"` out of a release document.
///
/// ⚠️ **Hand-written rather than a JSON dependency, and narrow on purpose.**
/// It reads one string from a document this module asked for by name. A body
/// that does not carry one is a non-answer, never an absence of releases.
fn tag_of(body: &str) -> Result<Option<String>, String> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|error| format!("the release document is not JSON: {}", error))?;
    match value.get("tag_name").and_then(serde_json::Value::as_str) {
        Some(tag) if !tag.is_empty() => Ok(Some(tag.to_string())),
        _ => Err("the release document carries no tag_name".to_string()),
    }
}

/// Runs `herdr plugin install` to apply an update.
#[derive(Debug, Clone)]
pub struct HerdrInstaller {
    binary: String,
}

impl HerdrInstaller {
    /// Names the Herdr binary to run, which a plugin reads from
    /// [`crate::env::BIN_PATH_VAR`].
    pub fn at(binary: impl Into<String>) -> HerdrInstaller {
        HerdrInstaller {
            binary: binary.into(),
        }
    }
}

impl Installer for HerdrInstaller {
    fn install(&mut self, owner: &str, repo: &str, tag: &str) -> Result<(), String> {
        let finished = Command::new(&self.binary)
            .args(install_arguments(owner, repo, tag))
            .output()
            .map_err(|error| format!("{} could not run: {}", self.binary, error))?;
        match finished.status.success() {
            true => Ok(()),
            false => Err(format!(
                "{} refused the install: {}",
                self.binary,
                String::from_utf8_lossy(&finished.stderr).trim()
            )),
        }
    }
}

/// The arguments the refresh runs with.
///
/// 🚨 **`--ref` is mandatory and this is the only place that decides it.** ✅
/// Measured 2026-09-13 (§8.2.1): installing with `--ref` records
/// `requested_ref` in `plugins.json`, and refreshing **without** it produces a
/// record carrying none — so the documented refresh silently converts a pinned
/// install into a floating one. An update has to *move* a pin, never delete it.
pub fn install_arguments(owner: &str, repo: &str, tag: &str) -> Vec<String> {
    vec![
        "plugin".to_string(),
        "install".to_string(),
        format!("{}/{}", owner, repo),
        "--ref".to_string(),
        tag.to_string(),
        "--yes".to_string(),
    ]
}

/// Whether this install is one this module may touch at all.
///
/// 🚨 SCOPE.md §8.3. `PluginSourceKind` defaults to `Local`, so an install
/// Herdr cannot describe is treated as one that must be left alone.
pub fn is_managed(plugin: &InstalledPluginInfo) -> bool {
    matches!(plugin.source.kind, PluginSourceKind::Github)
}

/// Asks whether a newer release exists, honouring the timer and the guard.
///
/// The order is load-bearing. The guard runs before the timer, so a `local:`
/// install never even writes a stamp, and the stamp is written before the
/// network call rather than after it.
pub fn check(
    plugin: &InstalledPluginInfo,
    stamp: &Path,
    now: SystemTime,
    interval: Duration,
    releases: &impl Releases,
) -> Decision {
    if !is_managed(plugin) {
        return Decision::Skipped(Skipped::LocalInstall);
    }
    let Some((owner, repo)) = repository_of(plugin) else {
        return Decision::Skipped(Skipped::NoRepository);
    };
    if !due(stamp, now, interval) {
        return Decision::Skipped(Skipped::TooSoon);
    }

    // 🚨 Before the call, never after. A 403 that left this unwritten would
    // turn one attempt a day into one attempt a launch.
    write_time(stamp, now);

    match releases.latest(owner, repo) {
        Err(reason) => Decision::NoAnswer(reason),
        Ok(None) => Decision::UpToDate,
        Ok(Some(tag)) if tag == plugin.version => Decision::UpToDate,
        Ok(Some(tag)) => Decision::Available(Available {
            installed: plugin.version.clone(),
            tag,
            repository: format!("{}/{}", owner, repo),
        }),
    }
}

/// Runs [`check`], then keeps what it learned in `result` for a later launch.
///
/// This is what a detached check calls. The attempt stamp is written exactly as
/// [`check`] writes it, before the network call; `result` is written after.
///
/// | The check answered | `result` afterwards |
/// |---|---|
/// | [`Decision::Available`] | holds that update |
/// | [`Decision::UpToDate`] | removed |
/// | [`Decision::NoAnswer`] | removed |
/// | [`Decision::Skipped`] | untouched |
///
/// 🚨 **A check that found nothing clears the older result.** An `Available`
/// left standing after the newest answer was "current" or "nobody answered"
/// would be offered as if it were still true. A skip asked nothing, so it has
/// nothing to replace the last answer with.
///
/// ⚠️ A result that cannot be written or removed is ignored, as the stamp is:
/// the decision is still returned, and [`offer`] refuses anything it cannot
/// trust.
pub fn check_and_save(
    plugin: &InstalledPluginInfo,
    stamp: &Path,
    result: &Path,
    now: SystemTime,
    interval: Duration,
    releases: &impl Releases,
) -> Decision {
    let decision = check(plugin, stamp, now, interval, releases);
    save(result, &decision);
    decision
}

/// The saved update to offer at this launch, if there is one to offer now.
///
/// Makes no network call. Answers `Some` only when all of these hold:
///
/// - `plugin` is a managed install (SCOPE.md §8.3),
/// - `result` holds a readable [`Available`] for this plugin's repository,
/// - that `Available` was found against the version installed now, and
/// - `interval` has elapsed since [`record_offer`] last wrote `offered`.
///
/// 🚨 **A result found against another version is stale, and is refused.** The
/// plugin was upgraded (or reinstalled) since the check, so the saved answer
/// describes an install that no longer exists. The next check replaces it.
///
/// ⚠️ **A result that is absent, unreadable or corrupt is refused** rather than
/// repaired: offering an update nobody can vouch for is worse than waiting a
/// day for the next check. An `offered` record that is absent or unreadable
/// means "never offered", as [`due`] reads any stamp.
pub fn offer(
    plugin: &InstalledPluginInfo,
    result: &Path,
    offered: &Path,
    now: SystemTime,
    interval: Duration,
) -> Option<Available> {
    let update = saved(plugin, result)?;
    if !due(offered, now, interval) {
        return None;
    }
    Some(update)
}

/// Records that an offer was shown or declined at `now`.
///
/// [`offer`] then stays silent until the interval has passed again. Written in
/// the attempt stamp's format, so [`due`] reads both.
pub fn record_offer(offered: &Path, now: SystemTime) {
    write_time(offered, now);
}

/// Applies an update a caller has decided to take.
///
/// 🚨 **The guard is re-checked here.** [`check`] and this can be called
/// independently, and a caller holding an [`Available`] from one plugin must
/// not be able to apply it against another.
pub fn apply(
    plugin: &InstalledPluginInfo,
    update: &Available,
    installer: &mut impl Installer,
) -> Result<(), String> {
    if !is_managed(plugin) {
        return Err(format!(
            "{} is a local install, and this never refreshes one: a developer \
             who asked to compile would silently get somebody else's binary",
            plugin.plugin_id
        ));
    }
    let (owner, repo) = match update.repository.split_once('/') {
        Some((owner, repo)) if !owner.is_empty() && !repo.is_empty() => (owner, repo),
        _ => return Err(format!("{:?} is not an owner/repo", update.repository)),
    };
    installer.install(owner, repo, &update.tag)
}

/// Whether `interval` has elapsed since the time recorded in `stamp`.
///
/// Public so a launch can ask whether a detached check is worth spawning
/// without making the network call [`check`] would make. It reads the attempt
/// stamp and the offer record alike.
///
/// An unreadable or absent stamp means "never attempted", which is the
/// direction that checks rather than the one that stays silent forever.
pub fn due(stamp: &Path, now: SystemTime, interval: Duration) -> bool {
    let Ok(text) = fs::read_to_string(stamp) else {
        return true;
    };
    let Ok(then) = text.trim().parse::<u64>() else {
        return true;
    };
    let Ok(elapsed) = now.duration_since(UNIX_EPOCH) else {
        return true;
    };
    elapsed.as_secs().saturating_sub(then) >= interval.as_secs()
}

/// The plugin's `owner` and `repo`, when its record names both.
fn repository_of(plugin: &InstalledPluginInfo) -> Option<(&str, &str)> {
    match (
        plugin.source.owner.as_deref(),
        plugin.source.repo.as_deref(),
    ) {
        (Some(owner), Some(repo)) if !owner.is_empty() && !repo.is_empty() => Some((owner, repo)),
        _ => None,
    }
}

/// Keeps what a check learned, per the table on [`check_and_save`].
fn save(result: &Path, decision: &Decision) {
    match decision {
        Decision::Available(update) => {
            if let Ok(text) = serde_json::to_string(update) {
                let _ = write_whole(result, &text);
            }
        }
        Decision::Skipped(_) => {}
        _ => {
            let _ = fs::remove_file(result);
        }
    }
}

/// The saved update, when it is readable and still describes this install.
fn saved(plugin: &InstalledPluginInfo, result: &Path) -> Option<Available> {
    if !is_managed(plugin) {
        return None;
    }
    let (owner, repo) = repository_of(plugin)?;
    let update: Available = serde_json::from_str(&fs::read_to_string(result).ok()?).ok()?;
    if update.repository != format!("{}/{}", owner, repo) {
        return None;
    }
    if update.installed != plugin.version {
        return None;
    }
    Some(update)
}

/// Replaces `path` in one step, so a launch reading it mid-write sees the old
/// file or the new one and never half of either.
fn write_whole(path: &Path, text: &str) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        fs::create_dir_all(directory)?;
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    fs::write(&partial, text)?;
    fs::rename(&partial, path).inspect_err(|_| {
        let _ = fs::remove_file(&partial);
    })
}

/// Writes `now` as whole seconds since the epoch, ignoring a failure to write.
///
/// 🔑 **The attempt stamp's format, unchanged**, so stamps already on users'
/// machines keep working. The offer record shares it.
///
/// ⚠️ **A stamp that cannot be written must not stop the check**, and it must
/// not be reported as one either: the worst case is that the next launch asks
/// again, which is the behaviour without a stamp at all.
fn write_time(stamp: &Path, now: SystemTime) {
    let Ok(elapsed) = now.duration_since(UNIX_EPOCH) else {
        return;
    };
    if let Some(directory) = stamp.parent() {
        let _ = fs::create_dir_all(directory);
    }
    let _ = fs::write(stamp, format!("{}\n", elapsed.as_secs()));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔑 **`tag_of` is private and reached through the network seam, so the
    /// integration suite cannot drive it.** Its rule — a body carrying no usable
    /// tag is a *non-answer*, never an absence of releases — is the one that
    /// decides whether a plugin is told it is up to date. It is checked here
    /// rather than left to a fake that would have to return the answer under
    /// test.
    #[test]
    fn a_release_document_answers_its_tag() {
        assert_eq!(
            tag_of(r#"{"tag_name":"0.9.1"}"#),
            Ok(Some("0.9.1".to_string()))
        );
    }

    #[test]
    fn a_body_with_no_tag_is_a_non_answer_rather_than_no_releases() {
        // 🚨 `Ok(None)` here would become `Decision::UpToDate`, which tells a
        // plugin the opposite of what happened.
        assert!(tag_of(r#"{"message":"Not Found"}"#).is_err());
    }

    #[test]
    fn an_empty_tag_is_a_non_answer_too() {
        assert!(tag_of(r#"{"tag_name":""}"#).is_err());
    }

    #[test]
    fn a_body_that_is_not_json_is_a_non_answer() {
        assert!(tag_of("rate limit exceeded").is_err());
    }
}
