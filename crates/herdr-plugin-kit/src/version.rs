//! What this binary is, where it came from, and whether it is current.
//!
//! Promoted from recent-spaces, which had the best of the three
//! implementations. See SCOPE.md §6.
//!
//! # The split, and why it is not optional
//!
//! ⚠️ **`env!` resolves in whichever crate it is written in.** A plain kit
//! function calling `env!("CARGO_PKG_VERSION")` would capture **the kit's**
//! version and the kit's commit, and every plugin that called it would report
//! them as its own. The bug would be invisible, because the output would still
//! look like a version report.
//!
//! So the compile-time half is a macro that expands in the plugin's crate, and
//! the rest is plain functions here:
//!
//! | Piece | Where it lives | What it does |
//! |---|---|---|
//! | `herdr-plugin-kit-build::stamp()` | the plugin's `build.rs` | emits `HERDR_PLUGIN_COMMIT` and `HERDR_PLUGIN_BUILT` |
//! | [`crate::version_report!`] | expands in the plugin's crate | captures that plugin's own `env!` values |
//! | [`report`], [`read_manifest`], [`provenance_of`] | here | format, read the manifest, compute staleness |
//!
//! # Nothing here fails
//!
//! 🔑 **Every lookup degrades to a word.** This is the report somebody runs
//! when the plugin is already broken, so a report that panicked would tell them
//! nothing at all. No function in this module returns a `Result`, and none of
//! them panics. A missing stamp reads `unknown`, an unreadable manifest says so
//! and names the path it tried, and a tree whose state could not be checked
//! reads `-unverified` rather than clean.
//!
//! ✅ It also never touches the socket. `--version` has to answer when the
//! server is down, which is exactly when it gets run, so nothing here reaches
//! the API. SCOPE.md §5.1 records why that decides where these values come
//! from.

use std::path::{Path, PathBuf};

use crate::env::{Environment, PLUGIN_ROOT_VAR};

/// What a lookup answers when it cannot answer.
pub const UNKNOWN: &str = "unknown";

/// The plugin manifest Herdr reads, and the file this module compares against.
pub const MANIFEST_FILE: &str = "herdr-plugin.toml";

/// Appended to the binary's own path to find the note the fetch path writes.
///
/// So a binary at `target/release/watch` has its note at
/// `target/release/watch.download`. Beside the binary rather than in the state
/// directory, because the two have to travel together: a note that outlived
/// the binary it describes would describe the wrong one.
pub const PROVENANCE_SUFFIX: &str = ".download";

/// The consuming plugin's own compile-time facts.
///
/// ⚠️ **Never build one of these by hand inside a kit function.** Use
/// [`crate::version_report!`], which expands in the plugin's crate. The whole reason
/// this type exists is to carry values across that boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Build {
    /// The plugin's `CARGO_PKG_VERSION`.
    pub version: &'static str,
    /// The commit, with its `-dirty` or `-unverified` marker.
    pub commit: &'static str,
    /// The build instant, in UTC.
    pub built: &'static str,
}

impl Build {
    /// Builds the triple, degrading anything absent or empty to [`UNKNOWN`].
    ///
    /// The two stamp values are `Option` because a plugin whose `build.rs`
    /// does not call `stamp()` still has to compile and still has to report.
    /// `option_env!` is what makes that possible, and this is where its `None`
    /// becomes a word rather than a failure.
    pub fn new(
        version: &'static str,
        commit: Option<&'static str>,
        built: Option<&'static str>,
    ) -> Build {
        Build {
            version: or_unknown(Some(version)),
            commit: or_unknown(commit),
            built: or_unknown(built),
        }
    }
}

fn or_unknown(value: Option<&'static str>) -> &'static str {
    match value {
        Some(text) if !text.trim().is_empty() => text,
        _ => UNKNOWN,
    }
}

/// Produces the whole version report, capturing the **calling crate's** values.
///
/// ```no_run
/// use herdr_plugin_kit::env::Environment;
///
/// let environment = Environment::from_process();
/// print!("{}", herdr_plugin_kit::version_report!("watch", &environment));
/// ```
///
/// ⚠️ **This has to be a macro.** Written as a function it would capture the
/// kit's own version and commit for every plugin that called it. See the module
/// documentation.
#[macro_export]
macro_rules! version_report {
    ($name:expr, $env:expr $(,)?) => {{
        // Every one of these expands in the *calling* crate, which is the
        // entire point. `CARGO_PKG_VERSION` is the caller's, and the two
        // `option_env!` lookups read what the caller's own `build.rs` emitted.
        let build = $crate::version::Build::new(
            env!("CARGO_PKG_VERSION"),
            option_env!("HERDR_PLUGIN_COMMIT"),
            option_env!("HERDR_PLUGIN_BUILT"),
        );
        let exe = ::std::env::current_exe().ok();
        $crate::version::report(
            $name,
            &build,
            &$crate::version::read_manifest(
                $crate::version::root_of($env, exe.as_deref()).as_deref(),
            ),
            &$crate::version::provenance_of(exe.as_deref()),
        )
    }};
}

/// What reading the plugin manifest produced.
///
/// Five outcomes rather than an `Option`, because they call for five different
/// things from whoever is reading. "Not found" and "found but will not parse"
/// are different problems, and collapsing them costs the reader the diagnosis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Manifest {
    /// Read, parsed, and carrying a version.
    Found { version: String, path: PathBuf },
    /// The file could not be read at this path.
    Unreadable(PathBuf),
    /// The file was read but is not valid TOML.
    Unparsed(PathBuf),
    /// Valid TOML, with no `version` key.
    NoVersion(PathBuf),
    /// No plugin root could be found, so there was nowhere to look.
    NoRoot,
}

/// How this binary arrived.
///
/// ✅ Runtime state, read from a file, and deliberately **not** part of the
/// compile-time macro. A binary can be built once and shipped, so how it
/// arrived is not something its own compilation can know. SCOPE.md §6.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provenance {
    /// No note beside the binary, so it was compiled here.
    Compiled,
    /// A note beside the binary says it was fetched.
    ///
    /// Both fields are optional because the note is written by a shell shim,
    /// and a note that is present but thin still proves the binary was
    /// fetched. That fact matters more than the detail: it is what decides
    /// whether the remedy is "rebuild" or "reinstall".
    Fetched {
        asset: Option<String>,
        url: Option<String>,
    },
    /// A note is there and could not be read.
    ///
    /// ⚠️ Kept apart from `Compiled` on purpose. A note that exists is
    /// evidence the binary was fetched, so reading this as "compiled" would
    /// tell a user with no toolchain to run `cargo build`.
    FetchedUnreadable,
}

/// Finds the plugin's checkout: the environment first, then the binary's own
/// location.
///
/// `HERDR_PLUGIN_ROOT` is authoritative because Herdr sets it, and it is the
/// only answer available when the binary has been moved. The walk is the
/// fallback for a plugin run by hand, and it confirms itself by checking that
/// a manifest is actually there, rather than trusting the shape of the path.
///
/// A binary at `<root>/target/release/<name>` puts the root three ancestors up.
pub fn root_of(env: &Environment, exe: Option<&Path>) -> Option<PathBuf> {
    if let Some(named) = env.get(PLUGIN_ROOT_VAR).filter(|root| !root.is_empty()) {
        return Some(PathBuf::from(named));
    }
    let root = exe?.ancestors().nth(3)?;
    match root.join(MANIFEST_FILE).is_file() {
        true => Some(root.to_path_buf()),
        false => None,
    }
}

/// Reads the plugin manifest, naming what went wrong rather than guessing.
pub fn read_manifest(root: Option<&Path>) -> Manifest {
    let Some(root) = root else {
        return Manifest::NoRoot;
    };
    let path = root.join(MANIFEST_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Manifest::Unreadable(path);
    };
    match text.parse::<toml::Table>() {
        Err(_) => Manifest::Unparsed(path),
        Ok(table) => match table.get("version").and_then(toml::Value::as_str) {
            Some(version) => Manifest::Found {
                version: version.to_string(),
                path,
            },
            None => Manifest::NoVersion(path),
        },
    }
}

/// Reads the note beside the binary to decide how the binary arrived.
///
/// The note is a `KEY=value` file, which is why [`crate::env::parse_env_file`]
/// reads it: the fetch path writes it from shell, in the same shape as every
/// other file that crosses that boundary.
///
/// ⚠️ A **directory** with the note's name reads as `Compiled`, matching the
/// `[ -f ]` the shim asks. The two have to agree, or a plugin and its own
/// installer would disagree about where the binary came from.
pub fn provenance_of(exe: Option<&Path>) -> Provenance {
    let Some(exe) = exe else {
        return Provenance::Compiled;
    };
    let mut note = exe.to_path_buf().into_os_string();
    note.push(PROVENANCE_SUFFIX);
    let note = PathBuf::from(note);
    if !note.is_file() {
        return Provenance::Compiled;
    }
    let Ok(text) = std::fs::read_to_string(&note) else {
        return Provenance::FetchedUnreadable;
    };
    let pairs = crate::env::parse_env_file(&text);
    let value = |wanted: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == wanted)
            .map(|(_, found)| found.clone())
            .filter(|found| !found.is_empty())
    };
    Provenance::Fetched {
        asset: value("asset"),
        url: value("url"),
    }
}

fn manifest_line(manifest: &Manifest) -> String {
    match manifest {
        Manifest::Found { version, path } => format!("manifest {} at {}", version, path.display()),
        Manifest::Unreadable(path) => format!("manifest unreadable at {}", path.display()),
        Manifest::Unparsed(path) => format!("manifest unparsed at {}", path.display()),
        Manifest::NoVersion(path) => {
            format!("manifest has no version key at {}", path.display())
        }
        Manifest::NoRoot => format!(
            "manifest not found: set {} to the plugin checkout to read it",
            PLUGIN_ROOT_VAR
        ),
    }
}

fn provenance_line(provenance: &Provenance) -> String {
    match provenance {
        Provenance::Compiled => "built from source on this machine".to_string(),
        Provenance::Fetched {
            asset: Some(asset),
            url: Some(url),
        } => format!("fetched {} from {}", asset, url),
        Provenance::Fetched {
            asset: Some(asset),
            url: None,
        } => format!("fetched {}, and the note records no url", asset),
        Provenance::Fetched { asset: None, .. } => {
            "fetched, and the note beside it names no asset".to_string()
        }
        Provenance::FetchedUnreadable => {
            "fetched, and the note beside it could not be read".to_string()
        }
    }
}

/// What to tell somebody holding a stale binary.
///
/// 🔑 It follows **how the binary arrived**, not how it was made. Whoever
/// installed a published binary has no toolchain, so "rebuild it" is not an
/// instruction they can follow.
fn remedy(provenance: &Provenance, wanted: &str) -> String {
    match provenance {
        Provenance::Compiled => "Rebuild it with `cargo build --release`.".to_string(),
        _ => format!(
            "This binary was fetched, so reinstall the plugin to get the {} binary.",
            wanted
        ),
    }
}

fn stale_line(build: &Build, manifest: &Manifest, provenance: &Provenance) -> Option<String> {
    let Manifest::Found { version, .. } = manifest else {
        return None;
    };
    if version == build.version {
        return None;
    }
    Some(format!(
        "STALE: this binary is {} but the manifest is {}. {}",
        build.version,
        version,
        remedy(provenance, version)
    ))
}

/// Formats the report. Three lines of fact, then a verdict only when there is
/// one.
///
/// ```text
/// watch 0.5.0 (a1b2c3d, built 2026-09-11T04:39:22Z)
/// manifest 0.5.0 at /p/herdr-plugin.toml
/// built from source on this machine
/// ```
///
/// The `STALE:` line is appended when the binary and the manifest disagree.
/// Its absence is the report saying the two agree, which is why the first
/// three lines are unconditional: a report with a line missing would be
/// ambiguous between "fine" and "could not tell".
pub fn report(name: &str, build: &Build, manifest: &Manifest, provenance: &Provenance) -> String {
    let mut lines = vec![
        format!(
            "{} {} ({}, built {})",
            name, build.version, build.commit, build.built
        ),
        manifest_line(manifest),
        provenance_line(provenance),
    ];
    if let Some(stale) = stale_line(build, manifest, provenance) {
        lines.push(stale);
    }
    lines.push(String::new());
    lines.join("\n")
}
