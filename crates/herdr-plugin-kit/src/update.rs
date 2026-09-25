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
//! which is the safe direction. [`managed`] is the door a plugin goes through
//! first, and the guard is checked again in [`check`], [`offer`] and [`apply`],
//! because each can be called alone.
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
//! | the result | [`check_and_save`], after the answer | the [`Available`] the last answered check found, or nothing |
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
//! ⚠️ **These calls take every path from the caller**, and read no environment
//! variable. Only the setup below chooses a directory, as the next sections
//! say.
//!
//! # Wiring it into a plugin
//!
//! Every plugin that offers updates needs the same setup, so the kit carries it
//! (SCOPE.md §8.2, "Where the kit ends"). A plugin supplies its id, a name for
//! its state directory, and a name for each offer record it keeps:
//!
//! | Call | What it does |
//! |---|---|
//! | [`lookup`], [`lookup_in`] | reads this plugin's install record through `plugin.list`, then [`managed_in`] |
//! | [`managed`], [`managed_in`] | 🚨 **the one door**: [`Files`] for a GitHub install, `None` for anything else |
//! | [`spawn_check_if_due`] | re-runs this binary as [`CHECK_FLAG`], detached and reaped, when a check is due |
//! | [`run_check`], [`run_check_in`] | the [`CHECK_FLAG`] side: [`lookup_in`], then [`check_and_save`] |
//! | [`Files::offered`] | the offer record for one channel, such as a dialog or a toast |
//!
//! 🔑 **The wording, the buttons, and the choice of dialog or toast stay in the
//! plugin.** This module still never prompts and never depends on `dialog`,
//! so a headless watcher can use all of it.
//!
//! # Where the files live
//!
//! 🔑 **In Herdr's per-plugin state directory, and inside the install's own
//! `plugin_root` only when Herdr provides none.** Decided by Mike 2026-09-25,
//! shipped in 0.5.4. SCOPE.md §8.2 records the decision.
//!
//! ✅ **Measured on isolated Herdr 0.9.1 servers, 2026-09-25.** An `[[actions]]`
//! command, four event hooks (`worktree.created`, `worktree.opened`,
//! `workspace.created`, `workspace.focused`) and a `[[startup]]` entry all
//! receive [`crate::env::PLUGIN_STATE_DIR_VAR`] and
//! [`crate::env::BIN_PATH_VAR`]. The state directory is
//! `$XDG_STATE_HOME/herdr/plugins/<plugin_id>`, and Herdr creates it. 🪤 Up to
//! 0.5.3 the files lived in `plugin_root`, because the docs said an event hook
//! receives neither variable. That claim rested on a Herdr 0.8.2 note that
//! listed only pane ids, and it is false on 0.9.1.
//!
//! | The files live in | When `HERDR_PLUGIN_STATE_DIR` is |
//! |---|---|
//! | `<HERDR_PLUGIN_STATE_DIR>/<state_dir>` | an absolute path whose last component is this plugin's id |
//! | `<plugin_root>/<state_dir>` | anything else: absent, empty, relative, or named for another plugin |
//!
//! 🚨 **A directory named for another plugin is not used.** A plugin that runs
//! another plugin's binary can hand it its own state directory (see
//! [`crate::env::PER_PLUGIN_VARS`]). Files written there would sit in the wrong
//! plugin's state, and two plugins using one state-directory name would share
//! an attempt stamp.
//!
//! 🚨 **The door is exactly as strict as before.** [`managed`] refuses a
//! `local:` install before it looks at any directory, so a linked working tree
//! gets no file in the state directory, as it gets none in its own root. It
//! still refuses a record with no absolute root and a name that is not one
//! plain path component, whichever directory would be used.
//!
//! ✅ **A reinstall empties `plugin_root`** (SCOPE.md §8.2.1). ⚠️ That it
//! leaves the state directory alone is inferred from where that directory sits,
//! outside the root, and has not been measured.
//!
//! ⚠️ **Accepted cost:** files an install saved under `plugin_root` before
//! 0.5.4 are left behind, once. The first launch after the upgrade finds no
//! stamp, which reads as "due", so it costs at most one extra check. Every
//! missing file already reads as the safe default.
//!
//! 🔑 **The detached check resolves the same directory as its parent.** The
//! child inherits its parent's process environment, but a caller of
//! [`managed_in`] may have resolved from a different [`Environment`]. So
//! [`spawn_check_if_due`] sets `HERDR_PLUGIN_STATE_DIR` on the child to the
//! directory its [`Files`] used, and removes it when they fell back to
//! `plugin_root`.
//!
//! The calls ending in `_in` take the [`Environment`] to resolve from. The
//! calls without the suffix, which 0.5.3 shipped, read the process
//! environment through [`Environment::from_process`].

use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::api::client::Client;
use crate::api::generated::{
    InstalledPluginInfo, PluginListAnswer, PluginListParams, PluginSourceKind, RequestMethod,
};
use crate::env::{Environment, PLUGIN_STATE_DIR_VAR};

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
    /// No release is newer than the one installed, under SemVer precedence.
    ///
    /// That includes an install newer than the newest release, such as a
    /// prerelease `releases/latest` does not list.
    UpToDate,
    /// A release strictly newer than the installed version exists.
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
        Ok(Some(tag)) => match newer(&tag, &plugin.version) {
            Err(unparsable) => Decision::NoAnswer(unparsable),
            Ok(false) => Decision::UpToDate,
            Ok(true) => Decision::Available(Available {
                installed: plugin.version.clone(),
                tag,
                repository: format!("{}/{}", owner, repo),
            }),
        },
    }
}

/// Whether the release `tag` is strictly newer than `installed`, under
/// SemVer 2.0.0 precedence.
///
/// 🚨 **Strictly newer, never merely different.** A string comparison offered
/// `v0.4.0` to an install of `0.4.0`, and offered an *older* release as an
/// update. [`apply`] passes the tag as `--ref`, so accepting that offer
/// installs the downgrade. The likely route is an installed prerelease,
/// because GitHub's `releases/latest` skips prereleases.
///
/// One leading `v` is ignored on either side (SCOPE.md §12.1: older tags
/// carry one). Build metadata is validated and then ignored, as SemVer §10
/// requires. An installed version newer than the release is not an update.
///
/// ⚠️ **A side that is not a semantic version is an error, never `false`.**
/// `false` becomes [`Decision::UpToDate`], and an unanswerable question read as
/// "up to date" is the conflation [`Decision::NoAnswer`] exists to prevent.
fn newer(tag: &str, installed: &str) -> Result<bool, String> {
    let unparsable =
        |side: &str, text: &str| format!("the {} {:?} is not a semantic version", side, text);
    let release = parse(tag).ok_or_else(|| unparsable("release tag", tag))?;
    let current = parse(installed).ok_or_else(|| unparsable("installed version", installed))?;
    Ok(precedence(&release, &current).is_gt())
}

/// A semantic version, holding only the parts precedence reads.
struct Version<'a> {
    /// Major, minor and patch, each digits with no leading zero.
    core: [&'a str; 3],
    /// The prerelease identifiers, empty for a release.
    pre: Vec<&'a str>,
}

/// Parses SemVer 2.0.0's grammar strictly, after one optional leading `v`.
///
/// Numbers stay text: canonical digits order by length and then by digit, so
/// no version is too large to compare.
fn parse(text: &str) -> Option<Version<'_>> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let (text, build) = match text.split_once('+') {
        Some((text, build)) => (text, build.split('.').collect()),
        None => (text, Vec::new()),
    };
    if !build.iter().all(|id| identifier(id)) {
        return None;
    }
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, pre.split('.').collect()),
        None => (text, Vec::new()),
    };
    if !pre
        .iter()
        .all(|id| identifier(id) && (!numeric(id) || canonical(id)))
    {
        return None;
    }
    let [major, minor, patch] = core.split('.').collect::<Vec<_>>()[..] else {
        return None;
    };
    if ![major, minor, patch]
        .iter()
        .all(|n| numeric(n) && canonical(n))
    {
        return None;
    }
    Some(Version {
        core: [major, minor, patch],
        pre,
    })
}

/// A prerelease or build identifier: one or more of `[0-9A-Za-z-]`.
fn identifier(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Digits only.
fn numeric(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

/// A number written without a leading zero, which SemVer requires.
fn canonical(number: &str) -> bool {
    number == "0" || !number.starts_with('0')
}

/// SemVer §11: core numerically field by field, then a release above any of
/// its prereleases, then prerelease identifiers left to right.
fn precedence(a: &Version, b: &Version) -> Ordering {
    use Ordering::{Equal, Greater, Less};
    let pre = match (a.pre.is_empty(), b.pre.is_empty()) {
        (true, true) => Equal,
        (true, false) => Greater,
        (false, true) => Less,
        (false, false) => a
            .pre
            .iter()
            .zip(&b.pre)
            .map(|(x, y)| identifier_order(x, y))
            .fold(Equal, Ordering::then)
            .then(a.pre.len().cmp(&b.pre.len())),
    };
    a.core
        .iter()
        .zip(&b.core)
        .map(|(x, y)| numeric_order(x, y))
        .chain([pre])
        .fold(Equal, Ordering::then)
}

/// Two prerelease identifiers: numbers numerically and below any text, text
/// in ASCII order.
fn identifier_order(x: &str, y: &str) -> Ordering {
    match (numeric(x), numeric(y)) {
        (true, true) => numeric_order(x, y),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => x.cmp(y),
    }
}

/// Two canonical numbers, compared as numbers: `10` is above `9`.
fn numeric_order(x: &str, y: &str) -> Ordering {
    x.len().cmp(&y.len()).then_with(|| x.cmp(y))
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
/// | [`Decision::NoAnswer`] | untouched |
/// | [`Decision::Skipped`] | untouched |
///
/// 🚨 **Only an answer replaces the last answer.** `UpToDate` is an answer: the
/// saved update is no longer current, and left standing it would be offered
/// as if it were still true. A skip asked nothing, and a non-answer learned
/// nothing, so neither has anything to replace the older find with.
///
/// 🚨 **A non-answer is not information.** It is a network outage, a timeout,
/// a missing `curl`, or a tag that is not a version. Clearing on it lost a
/// found update that no launch had offered yet, until the next answered check
/// up to one interval later. Decided by Mike 2026-09-24, shipped in 0.5.3.
///
/// ✅ **A result kept this way is still safe to read**, because [`offer`]
/// refuses one that is corrupt, found for another repository, found against
/// another installed version, or not newer than the version installed now.
/// ⚠️ The one case it cannot catch is a release pulled from GitHub after it was
/// found. The next answered check clears that.
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
/// - that `Available` was found against the version installed now,
/// - its tag is strictly newer than that version, as [`check`] requires, and
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

/// The flag a detached check runs under.
///
/// [`spawn_check_if_due`] passes it, and the plugin's `main` answers it by
/// calling [`run_check`] and exiting.
pub const CHECK_FLAG: &str = "--check-update";

/// Where one install keeps its update state.
///
/// Built by [`Files::under`], which refuses a path that could land outside the
/// install, and by [`managed_in`], which moves the same name into Herdr's state
/// directory when Herdr provides one (see "Where the files live" above). The
/// names on disk are fixed, and are the ones agentic-panes-layout wrote from
/// its own copy:
///
/// | File | Name | Written by |
/// |---|---|---|
/// | [`Files::stamp`] | `checked` | [`check`] |
/// | [`Files::result`] | `available.json` | [`check_and_save`] |
/// | [`Files::offered`] | `offered-<name>` | [`record_offer`], one file per channel |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files {
    dir: PathBuf,
    /// Herdr's state directory `dir` sits in, or `None` when it sits under
    /// `plugin_root`. [`spawn_check_if_due`] hands it to the child.
    herdr_state: Option<PathBuf>,
}

impl Files {
    /// The files in `state_dir`, a directory directly under `plugin_root`.
    ///
    /// 🚨 **Fails closed.** `None` when `plugin_root` is not an absolute path,
    /// or when `state_dir` is not a single plain name. A relative root would
    /// resolve against whatever directory the hook was started in. A
    /// `state_dir` of `..`, `a/b` or an absolute path would reach outside the
    /// install, because joining an absolute path discards the root.
    pub fn under(plugin_root: &Path, state_dir: &str) -> Option<Files> {
        if !plugin_root.is_absolute() || !plain_name(state_dir) {
            return None;
        }
        Some(Files {
            dir: plugin_root.join(state_dir),
            herdr_state: None,
        })
    }

    /// The directory that holds every file below.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// When a check was last attempted. Pass it to [`due`] and [`check`].
    pub fn stamp(&self) -> PathBuf {
        self.dir.join("checked")
    }

    /// The update the last answered check found. Pass it to [`check_and_save`] and
    /// [`offer`].
    pub fn result(&self) -> PathBuf {
        self.dir.join("available.json")
    }

    /// The offer record for the channel called `name`.
    ///
    /// 🔑 **One record per channel, because one would silence another.** A
    /// toast that points a user to a dialog must not start the dialog's
    /// interval. So each channel names its own record, and [`offer`] and
    /// [`record_offer`] take the one it names.
    ///
    /// `None` when `name` is not a single plain name, for the reason
    /// [`Files::under`] gives.
    pub fn offered(&self, name: &str) -> Option<PathBuf> {
        plain_name(name).then(|| self.dir.join(format!("offered-{}", name)))
    }
}

/// Whether `name` is exactly one ordinary path component.
///
/// Refuses the empty name, `.`, `..`, a root, and anything with a separator,
/// so a name joined onto a directory stays inside it.
fn plain_name(name: &str) -> bool {
    let mut parts = Path::new(name).components();
    matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(part)), None) if part == name
    )
}

/// [`managed_in`], resolving from this process's environment.
pub fn managed(
    plugin: InstalledPluginInfo,
    state_dir: &str,
) -> Option<(InstalledPluginInfo, Files)> {
    managed_in(plugin, state_dir, &Environment::from_process())
}

/// The install record and its [`Files`], when this is an install the plugin may
/// update.
///
/// 🚨 **The one door, and it fails closed.** `None` for a `local:` install
/// (SCOPE.md §8.3), and `None` when [`Files::under`] refuses the record's
/// `plugin_root` or `state_dir`, which includes a record with no root at all.
/// Both refusals hold even when `env` names a usable state directory, so a
/// linked working tree gets no file anywhere. A caller asks this first, and on
/// `None` it spawns nothing, reads nothing, writes nothing and asks nothing.
///
/// Otherwise the files live in `<HERDR_PLUGIN_STATE_DIR>/<state_dir>` when
/// `env` names an absolute state directory whose last component is this
/// plugin's id, and in `<plugin_root>/<state_dir>` when it does not. The
/// module documentation says why.
pub fn managed_in(
    plugin: InstalledPluginInfo,
    state_dir: &str,
    env: &Environment,
) -> Option<(InstalledPluginInfo, Files)> {
    if !is_managed(&plugin) {
        return None;
    }
    let files = Files::under(Path::new(&plugin.plugin_root), state_dir)?;
    let files = match herdr_state_dir(&plugin, env) {
        Some(root) => Files {
            dir: root.join(state_dir),
            herdr_state: Some(root),
        },
        None => files,
    };
    Some((plugin, files))
}

/// Herdr's state directory for `plugin`, when `env` names one this install can
/// use: an absolute path whose last component is the plugin's id.
///
/// ✅ Herdr 0.9.1 sets it to `$XDG_STATE_HOME/herdr/plugins/<plugin_id>`. 🚨 A
/// directory named for another plugin is refused, because a plugin that runs
/// another plugin's binary can hand it its own. An empty or relative value is
/// refused too, and every refusal falls back to `plugin_root`.
fn herdr_state_dir(plugin: &InstalledPluginInfo, env: &Environment) -> Option<PathBuf> {
    let dir = Path::new(env.get(PLUGIN_STATE_DIR_VAR)?);
    let ours = dir.file_name() == Some(std::ffi::OsStr::new(&plugin.plugin_id));
    (dir.is_absolute() && ours).then(|| dir.to_path_buf())
}

/// [`lookup_in`], resolving from this process's environment.
pub fn lookup(
    client: &Client,
    plugin_id: &str,
    state_dir: &str,
) -> Option<(InstalledPluginInfo, Files)> {
    lookup_in(client, plugin_id, state_dir, &Environment::from_process())
}

/// Reads the install record for `plugin_id` through `plugin.list`, then asks
/// [`managed_in`] with `env`.
///
/// `None` on any failure: a call Herdr did not answer, and an answer holding no
/// record for this plugin. ⚠️ **The answer is filtered by `plugin_id` again**,
/// so a server that ignored the request's filter still cannot hand back
/// another plugin's record.
pub fn lookup_in(
    client: &Client,
    plugin_id: &str,
    state_dir: &str,
    env: &Environment,
) -> Option<(InstalledPluginInfo, Files)> {
    let answer = client
        .call::<PluginListAnswer>(RequestMethod::PluginList(PluginListParams {
            plugin_id: Some(plugin_id.to_string()),
        }))
        .ok()?;
    let plugin = answer
        .plugins
        .into_iter()
        .find(|plugin| plugin.plugin_id == plugin_id)?;
    managed_in(plugin, state_dir, env)
}

/// Starts a detached check when one is due, and never waits for it.
///
/// Re-runs the current binary with [`CHECK_FLAG`], so a launch never waits on
/// the network call. Answers whether a check was started.
///
/// 🚨 **The child is reaped on a thread of its own.** A dropped child that
/// nobody waits for stays a zombie until the caller exits, and a long-lived
/// watcher calling this once per interval would collect one per call. The
/// thread waits and ends. It does not hold up the caller, and a caller that
/// exits first loses nothing: the child is in its own process group.
///
/// 🔑 **The child is told which directory to use.** It gets
/// `HERDR_PLUGIN_STATE_DIR` set to the state directory `files` sits in, or
/// removed when `files` sit under `plugin_root`. So its [`run_check`] resolves
/// the directory this launch resolved, whatever environment it inherited.
///
/// ⚠️ A failure to spawn is ignored rather than reported: the worst case is
/// that a later launch tries again.
pub fn spawn_check_if_due(files: &Files, now: SystemTime, interval: Duration) -> bool {
    if !due(&files.stamp(), now, interval) {
        return false;
    }
    let Ok(me) = std::env::current_exe() else {
        return false;
    };
    let Ok(mut child) = detached(&me, files).spawn() else {
        return false;
    };
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    true
}

/// Runs `program` with [`CHECK_FLAG`], with no stdio, in its own process group,
/// and with `HERDR_PLUGIN_STATE_DIR` set to the Herdr state directory `files`
/// sit in, or removed when they sit under `plugin_root`.
///
/// 🔑 **Its own process group, so the check outlives a hook** whose group
/// Herdr ends. ⚠️ Unix only: on Windows the child is spawned in the caller's
/// group.
///
/// 🚨 **No stdio, because an inherited pipe would hold the caller's reader
/// open.** On the `[[startup]]` path stderr is a pipe (SCOPE.md §10), so a
/// child that kept it would keep Herdr waiting for the whole network call.
fn detached(program: &Path, files: &Files) -> Command {
    let mut command = Command::new(program);
    command
        .arg(CHECK_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match files.herdr_state.as_deref() {
        Some(dir) => command.env(PLUGIN_STATE_DIR_VAR, dir),
        None => command.env_remove(PLUGIN_STATE_DIR_VAR),
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

/// [`run_check_in`], resolving from this process's environment.
///
/// 🔑 This is the call a plugin's `main` makes for [`CHECK_FLAG`], because
/// [`spawn_check_if_due`] sets the one variable it resolves from.
pub fn run_check(
    client: &Client,
    plugin_id: &str,
    state_dir: &str,
    now: SystemTime,
    interval: Duration,
    releases: &impl Releases,
) -> Option<Decision> {
    run_check_in(
        client,
        plugin_id,
        state_dir,
        &Environment::from_process(),
        now,
        interval,
        releases,
    )
}

/// The [`CHECK_FLAG`] side: finds the install, then asks `releases` and keeps
/// the answer for a later launch.
///
/// `None` when [`lookup_in`] refuses, in which case nothing was asked and
/// nothing was written. Otherwise the [`Decision`] from [`check_and_save`], for
/// the plugin to log. Pass [`CurlReleases::default`] outside a test.
pub fn run_check_in(
    client: &Client,
    plugin_id: &str,
    state_dir: &str,
    env: &Environment,
    now: SystemTime,
    interval: Duration,
    releases: &impl Releases,
) -> Option<Decision> {
    let (plugin, files) = lookup_in(client, plugin_id, state_dir, env)?;
    Some(check_and_save(
        &plugin,
        &files.stamp(),
        &files.result(),
        now,
        interval,
        releases,
    ))
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
        Decision::UpToDate => {
            let _ = fs::remove_file(result);
        }
        Decision::NoAnswer(_) => {}
        Decision::Skipped(_) => {}
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
    // 🚨 A result 0.5.0 saved may offer a reinstall or a downgrade, because its
    // check compared strings. Offer only what `check` would offer now.
    if newer(&update.tag, &plugin.version) != Ok(true) {
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

    // 🔑 **`precedence` and `parse` are private, and the integration suite
    // reaches them only one pair at a time through `check`.** The grammar and
    // the ordering are checked here, whole, against SemVer 2.0.0's own text.

    fn order(a: &str, b: &str) -> Ordering {
        let parsed = |text| parse(text).unwrap_or_else(|| panic!("{:?} should parse", text));
        precedence(&parsed(a), &parsed(b))
    }

    /// Asserts every version in `ascending` is strictly below the next, and
    /// that each pair reads the same way round in both directions.
    fn assert_ascending(ascending: &[&str]) {
        for pair in ascending.windows(2) {
            assert_eq!(order(pair[0], pair[1]), Ordering::Less, "{:?}", pair);
            assert_eq!(order(pair[1], pair[0]), Ordering::Greater, "{:?}", pair);
        }
    }

    #[test]
    fn the_core_compares_major_then_minor_then_patch_as_numbers() {
        // SemVer §11.2's example, plus the two a string comparison gets wrong:
        // `1.10.0` is above `1.9.0`, and a major outranks a larger patch.
        assert_ascending(&[
            "1.0.0", "1.9.9", "1.10.0", "2.0.0", "2.1.0", "2.1.1", "10.0.0",
        ]);
    }

    #[test]
    fn a_release_is_above_its_prereleases_in_semvers_own_order() {
        // SemVer §11.4's example, verbatim, with the one-digit against
        // two-digit pair (`beta.2`, `beta.11`) that string order reverses.
        assert_ascending(&[
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ]);
    }

    #[test]
    fn a_prerelease_of_a_later_version_is_above_an_earlier_release() {
        assert_ascending(&["0.9.1", "0.9.2-rc.1", "0.9.2"]);
    }

    #[test]
    fn text_identifiers_compare_in_ascii_order() {
        // Uppercase sorts before lowercase in ASCII, and a hyphen before both.
        assert_ascending(&["1.0.0-A-1", "1.0.0-Alpha", "1.0.0-alpha"]);
    }

    #[test]
    fn build_metadata_and_a_leading_v_do_not_change_precedence() {
        for (a, b) in [
            ("1.0.0", "1.0.0+20260923"),
            ("1.0.0+exp.sha.5114f85", "1.0.0+21AF26D3-117B344092BD"),
            ("1.0.0-rc.1+001", "1.0.0-rc.1"),
            ("v0.4.0", "0.4.0"),
            ("v1.0.0-beta+b", "1.0.0-beta"),
        ] {
            assert_eq!(order(a, b), Ordering::Equal, "{} {}", a, b);
        }
    }

    #[test]
    fn versions_semver_allows_are_parsed() {
        for text in [
            "0.0.0",
            "1.2.3-0",
            "1.2.3-0a",
            "1.2.3-x-y-z.--",
            "1.2.3+007",
            "1.2.3-alpha.10.beta+build.1",
            "99999999999999999999999.0.0",
        ] {
            assert!(parse(text).is_some(), "{}", text);
        }
    }

    #[test]
    fn versions_semver_forbids_are_refused() {
        for text in [
            "",
            "v",
            "latest",
            "1",
            "1.0",
            "1.0.0.0",
            "1..0",
            "1.x.0",
            "01.0.0",
            "1.00.0",
            "1.0.00",
            "-1.0.0",
            "1.0.0-",
            "1.0.0-alpha..1",
            "1.0.0-01",
            "1.0.0-alpha_1",
            "1.0.0+",
            "1.0.0+build..1",
            "1.0.0+build_1",
            "1.0.0+a+b",
            "V1.0.0",
            "vv1.0.0",
            " 1.0.0",
        ] {
            assert!(parse(text).is_none(), "{:?}", text);
        }
    }

    #[test]
    fn newer_is_strict() {
        assert_eq!(newer("0.9.1", "0.9.0"), Ok(true));
        assert_eq!(newer("0.9.1", "0.9.1"), Ok(false));
        assert_eq!(newer("0.9.0", "0.9.1"), Ok(false));
    }

    /// A script at `<dir>/plugin` that writes to `<dir>/seen` the arguments
    /// it was given, its process group, then, for each of its stdin, stdout
    /// and stderr, `null` when that descriptor is `/dev/null` and `other` when
    /// it is not, and last its `HERDR_PLUGIN_STATE_DIR`, or `unset`.
    ///
    /// `[ A -ef B ]` compares device and inode, so the script tells a null
    /// descriptor from an inherited one without reading from it.
    #[cfg(unix)]
    fn probe_script(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "herdr-plugin-kit-spawn-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("plugin");
        let seen = dir.join("seen");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 printf '%s\\n' \"$*\" > {0:?}\n\
                 ps -o pgid= -p $$ >> {0:?}\n\
                 for fd in 0 1 2; do\n\
                 if [ /dev/fd/$fd -ef /dev/null ]; then n=\"$n null\"; else n=\"$n other\"; fi\n\
                 done\n\
                 echo $n >> {0:?}\n\
                 printf '%s\\n' \"${{HERDR_PLUGIN_STATE_DIR-unset}}\" >> {0:?}\n",
                seen
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        (dir, script, seen)
    }

    /// 🔑 **`detached` is private, and the integration suite can only
    /// reach it by re-running its own test binary**, which answers the flag
    /// with an error and reports nothing back. So the spawn is driven here
    /// against a script that writes down what it was given.
    ///
    /// stdin is checked by the next test, because this one cannot tell a null
    /// stdin from an inherited one when its own stdin is already null.
    #[test]
    #[cfg(unix)]
    fn a_detached_check_gets_the_flag_its_own_process_group_and_no_output() {
        let (dir, script, seen) = probe_script("group");

        let state = dir.join("state");
        let files = Files {
            dir: state.join("u"),
            herdr_state: Some(state.clone()),
        };
        let mut child = detached(&script, &files).spawn().unwrap();
        let pid = child.id();
        assert!(child.wait().unwrap().success());

        let text = fs::read_to_string(&seen).unwrap();
        let _ = fs::remove_dir_all(&dir);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first(), Some(&CHECK_FLAG), "{}", text);
        // A group leader's group id is its own pid, which is what
        // `process_group(0)` makes it. Inherited, it would be this test's.
        assert_eq!(
            lines.get(1).map(|line| line.trim()),
            Some(pid.to_string().as_str()),
            "{}",
            text
        );
        // 🚨 Inherited, stdout and stderr would be whatever this test has,
        // and under `cargo test` driven by `tools/mutate.py` both are pipes.
        let fds: Vec<&str> = lines.get(2).unwrap_or(&"").split(' ').collect();
        assert_eq!(fds.get(1..), Some(&["null", "null"][..]), "{}", text);
        // The state directory reaches the child as the variable it reads.
        assert_eq!(
            lines.get(3).copied(),
            Some(state.to_string_lossy().as_ref()),
            "{}",
            text
        );
    }

    /// Set only in the copy of the test binary the next test starts.
    #[cfg(unix)]
    const PROBE: &str = "HERDR_PLUGIN_KIT_SPAWN_PROBE";

    /// 🔑 **Whatever stdin this test was started with, the spawn is driven
    /// from a process whose stdin is a pipe.** If this test's own stdin were
    /// `/dev/null`, as a CI runner's may be, an inherited stdin would read as
    /// null as well, and the mutation dropping `Stdio::null()` would survive
    /// or not depending on who ran the suite. So the test re-runs its own
    /// binary with a piped stdin, and that copy does the spawn.
    #[test]
    #[cfg(unix)]
    fn a_detached_check_gets_a_null_stdin_even_from_a_parent_that_has_one() {
        if let Some(script) = std::env::var_os(PROBE) {
            let files = Files::under(Path::new("/plugins/p"), "u").unwrap();
            let mut child = detached(Path::new(&script), &files).spawn().unwrap();
            assert!(child.wait().unwrap().success());
            return;
        }
        let (dir, script, seen) = probe_script("stdin");

        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "update::tests::a_detached_check_gets_a_null_stdin_even_from_a_parent_that_has_one",
                "--test-threads=1",
            ])
            .env(PROBE, &script)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();

        let text = fs::read_to_string(&seen).unwrap_or_default();
        let _ = fs::remove_dir_all(&dir);
        assert!(status.success(), "the probe copy failed: {}", text);
        let fds: Vec<&str> = text.lines().nth(2).unwrap_or("").split(' ').collect();
        assert_eq!(fds.first(), Some(&"null"), "{}", text);
    }

    /// An install of `plugin_id` at `/plugins/p`, managed from GitHub.
    fn installed(plugin_id: &str) -> InstalledPluginInfo {
        serde_json::from_value(serde_json::json!({
            "plugin_id": plugin_id,
            "name": "Probe",
            "version": "0.8.1",
            "plugin_root": "/plugins/p",
            "manifest_path": "/plugins/p/herdr-plugin.toml",
            "min_herdr_version": "0.9.0",
            "enabled": true,
            "source": {"kind": "github", "owner": "o", "repo": "r"},
        }))
        .expect("a minimal install record")
    }

    /// What a child started by `command` resolves, when the environment it
    /// inherits is `inherited`: that environment, with the command's own
    /// settings and removals applied, as the OS applies them.
    fn resolved_by_child(command: &Command, inherited: &Environment) -> Option<Files> {
        let mut vars: std::collections::BTreeMap<String, String> = inherited
            .vars()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        for (key, value) in command.get_envs() {
            let key = key.to_string_lossy().into_owned();
            match value {
                Some(value) => vars.insert(key, value.to_string_lossy().into_owned()),
                None => vars.remove(&key),
            };
        }
        let pairs: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        managed_in(installed("o.probe"), "u", &Environment::from_pairs(&pairs)).map(|(_, f)| f)
    }

    /// 🔑 **The child must resolve what its parent resolved, whatever it
    /// inherits.** A caller of `managed_in` may resolve from an environment
    /// that is not its process's, and the child only ever sees the process's.
    /// So each case below gives the child an inheritance that disagrees with
    /// the parent's answer, and a child that trusted it would resolve wrongly.
    #[test]
    fn a_detached_check_resolves_the_directory_its_parent_resolved() {
        let ours = "/state/herdr/plugins/o.probe";
        let with_state = Environment::from_pairs(&[(PLUGIN_STATE_DIR_VAR, ours)]);
        let without = Environment::default();

        // The parent used the state directory, and the child inherits none.
        let (_, parent) = managed_in(installed("o.probe"), "u", &with_state).unwrap();
        assert_eq!(parent.dir(), Path::new(ours).join("u"));
        let command = detached(Path::new("/bin/true"), &parent);
        assert_eq!(resolved_by_child(&command, &without), Some(parent));

        // The parent fell back to plugin_root, and the child inherits a state
        // directory it would otherwise use.
        let (_, parent) = managed_in(installed("o.probe"), "u", &without).unwrap();
        assert_eq!(parent.dir(), Path::new("/plugins/p/u"));
        let command = detached(Path::new("/bin/true"), &parent);
        assert_eq!(resolved_by_child(&command, &with_state), Some(parent));
    }

    #[test]
    fn newer_names_the_side_that_is_not_a_version() {
        let tag = newer("nightly", "0.9.1").unwrap_err();
        assert!(
            tag.contains("release tag") && tag.contains("nightly"),
            "{}",
            tag
        );
        let installed = newer("0.9.1", "dev").unwrap_err();
        assert!(
            installed.contains("installed version") && installed.contains("dev"),
            "{}",
            installed
        );
    }
}
