//! The reader for Herdr's **launch contract**, and not a settings loader.
//!
//! The distinction is load-bearing, and it was settled by measuring every
//! variable against the API rather than by argument. The API serves shared
//! server state. The environment carries per-invocation facts that only the
//! launching process knows. Different data, not duplicate data.
//!
//! ✅ Measured: **the API can replace 2 of the 8 variables below, and both
//! replacements are circular.** `PluginListParams` is
//! `{"plugin_id": ["string","null"]}`, with no "self" concept, so a plugin
//! must already know its own `plugin_id` or `plugin_root` to find its entry.
//! Both of those come from here. `HERDR_SOCKET_PATH` is impossible by
//! construction: you need the socket to call the API at all.
//!
//! Availability seals it. Nothing in [`crate::version`] may fail, and
//! `--version` has to print when the socket is down, which is exactly when it
//! gets run. Routing it through the API would invert that guarantee.
//!
//! Plugin-specific config parsing stays in the plugin. Only the shared
//! mechanism lives here. See SCOPE.md §5 and §5.1.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The socket, or on Windows the named pipe, that carries the API.
///
/// ⚠️ Not a Unix socket everywhere. See SCOPE.md §4.2.
pub const SOCKET_PATH_VAR: &str = "HERDR_SOCKET_PATH";

/// Absolute path to the `herdr` binary that launched this process.
///
/// ✅ Measured 2026-09-25 on Herdr 0.9.1: set for an `[[actions]]` command,
/// four event hooks and a `[[startup]]` entry, the same processes as
/// [`PLUGIN_STATE_DIR_VAR`]. ⚠️ A `[[build]]` hook gets no `HERDR_*` variable
/// at all (SCOPE.md §8.3).
pub const BIN_PATH_VAR: &str = "HERDR_BIN_PATH";

/// Overrides where Herdr's own `config.toml` is read from.
pub const CONFIG_PATH_VAR: &str = "HERDR_CONFIG_PATH";

/// The plugin's checkout, which is where its manifest lives.
pub const PLUGIN_ROOT_VAR: &str = "HERDR_PLUGIN_ROOT";

/// Where this plugin's own configuration is kept.
pub const PLUGIN_CONFIG_DIR_VAR: &str = "HERDR_PLUGIN_CONFIG_DIR";

/// Where this plugin may write state that outlives one invocation.
///
/// ✅ **Measured 2026-09-25 on isolated Herdr 0.9.1 servers, and set for every
/// kind of plugin process measured:** an `[[actions]]` command, four event
/// hooks (`worktree.created`, `worktree.opened`, `workspace.created`,
/// `workspace.focused`) and a `[[startup]]` entry. Its value is
/// `$XDG_STATE_HOME/herdr/plugins/<plugin_id>`, and Herdr creates the
/// directory. [`BIN_PATH_VAR`] was set beside it in every case.
///
/// 🪤 **This corrects an earlier claim.** The kit said an event hook receives
/// neither variable, and that nobody had confirmed this one is set at all. The
/// claim came from agentic-panes-layout's docs, which rest on a Herdr 0.8.2
/// note that listed only pane ids. SCOPE.md §8.2 holds the table.
///
/// ⚠️ **Not measured:** Herdr 0.9.0, a pane command, and the value when
/// `XDG_STATE_HOME` is unset. For an action with it unset the value was
/// `~/.local/state/herdr/plugins/<plugin_id>`, and the other kinds are inferred
/// to match.
pub const PLUGIN_STATE_DIR_VAR: &str = "HERDR_PLUGIN_STATE_DIR";

/// The name of the event that triggered this invocation.
pub const PLUGIN_EVENT_VAR: &str = "HERDR_PLUGIN_EVENT";

/// The event's payload, as JSON.
///
/// Per-invocation, so it is not queryable from the API at any later point.
pub const PLUGIN_EVENT_JSON_VAR: &str = "HERDR_PLUGIN_EVENT_JSON";

/// The variables that name **this** plugin's own launch, and no other's.
///
/// 🚨 **Handing these to another plugin's binary makes it read this plugin as
/// itself.** project-finder spawns agentic-panes-layout's `bin/agent-layout`,
/// and a child that inherits [`PLUGIN_ROOT_VAR`] reads project-finder's
/// checkout as its own manifest, its own config, and its own state directory.
///
/// ⚠️ **Named for the condition, not for what a caller does about it.** A
/// plugin spawning its *own* helper wants every one of these kept: they are
/// true for that child. They are false only for a binary belonging to some
/// other plugin, which is the case that needs the filter.
///
/// ```
/// use herdr_plugin_kit::env::{Environment, PER_PLUGIN_VARS};
///
/// let env = Environment::from_pairs(&[
///     ("HERDR_SOCKET_PATH", "/run/herdr.sock"),
///     ("HERDR_PLUGIN_ROOT", "/plugins/project-finder"),
///     ("PATH", "/usr/bin"),
/// ]);
///
/// let for_another_plugin: Vec<(&str, &str)> = env
///     .vars()
///     .filter(|(key, _)| !PER_PLUGIN_VARS.contains(key))
///     .collect();
///
/// assert_eq!(
///     for_another_plugin,
///     [("HERDR_SOCKET_PATH", "/run/herdr.sock"), ("PATH", "/usr/bin")],
/// );
/// ```
///
/// # Why this is a constant rather than a method
///
/// 🚨 **Which variables belong to one plugin is a fact about Herdr's contract,
/// and Herdr does not write it down.** The first hand-written filter got two
/// of the three directory variables and missed [`PLUGIN_STATE_DIR_VAR`],
/// which recent-spaces reads to place a lock file. The defect is not that one
/// plugin had a bug; it is that the second person to write this filter had
/// nothing to copy.
///
/// ✅ **Herdr does set [`PLUGIN_STATE_DIR_VAR`] for a plugin's own
/// processes.** Measured 2026-09-25 on isolated Herdr 0.9.1 servers: see
/// [`PLUGIN_STATE_DIR_VAR`]. ⚠️ **Still unmeasured, and the list does not rest
/// on it:** whether a pane command receives it, so the leak through a pane may
/// be theoretical. The reason to write the list down is
/// that nothing else states it.
///
/// 🔑 **The event pair is included on a fail-closed reading.** A child was not
/// triggered by the event that triggered this process, so
/// [`PLUGIN_EVENT_VAR`] and [`PLUGIN_EVENT_JSON_VAR`] are false for it in the
/// same way the directories are. Stripping one a caller wanted costs them a
/// line to put it back. Leaving one they did not want is silent.
pub const PER_PLUGIN_VARS: &[&str] = &[
    PLUGIN_ROOT_VAR,
    PLUGIN_CONFIG_DIR_VAR,
    PLUGIN_STATE_DIR_VAR,
    PLUGIN_EVENT_VAR,
    PLUGIN_EVENT_JSON_VAR,
];

/// A snapshot of the variables a process was launched with.
///
/// A snapshot rather than a live view, and that is the point. Reading through
/// this type instead of [`std::env::var`] keeps every lookup testable without
/// mutating process globals, which matters more once these crates leave
/// edition 2021 and `set_var` becomes `unsafe`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    vars: BTreeMap<String, String>,
}

impl Environment {
    /// Snapshots the current process environment.
    pub fn from_process() -> Environment {
        Environment {
            vars: std::env::vars().collect(),
        }
    }

    /// Builds an environment from literal pairs.
    ///
    /// This is the test seam. It is what lets every consumer of this crate
    /// exercise its own variable handling without touching process globals.
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Environment {
        Environment {
            vars: pairs
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        }
    }

    /// Reads one variable, exactly as it was set.
    ///
    /// ⚠️ **A variable that is set but empty answers `Some("")`, not `None`.**
    /// That is deliberate: "set to nothing" and "not set" are different facts,
    /// and this type does not decide for the caller which one matters. Herdr
    /// does inject empty values, so the idiom at nearly every call site is:
    ///
    /// ```
    /// use herdr_plugin_kit::env::{Environment, PLUGIN_ROOT_VAR};
    ///
    /// let env = Environment::from_pairs(&[(PLUGIN_ROOT_VAR, "")]);
    /// let root = env.get(PLUGIN_ROOT_VAR).filter(|value| !value.is_empty());
    /// assert_eq!(root, None);
    /// ```
    pub fn get(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    /// Every variable, in key order, borrowed.
    ///
    /// 🔑 **The accessor exists so that a plugin can write its own policy**,
    /// rather than the kit adjudicating one. A copy with one key overridden, a
    /// copy with pairs filled underneath, and a filtered child environment are
    /// three shapes two consumers genuinely disagree about, and each is a few
    /// lines over this iterator. SCOPE.md §13's one-consumer bar is why the kit
    /// ships the accessor and not the three.
    ///
    /// It yields `(&str, &str)` rather than the map's own `(&String, &String)`
    /// so that it composes both ways without an intermediate:
    ///
    /// ```
    /// use herdr_plugin_kit::env::Environment;
    ///
    /// let env = Environment::from_pairs(&[("B", "2"), ("A", "1")]);
    ///
    /// // Straight into a child process, which takes `AsRef<OsStr>` pairs.
    /// let mut command = std::process::Command::new("true");
    /// command.envs(env.vars().filter(|(key, _)| *key != "B"));
    ///
    /// // Or back into another Environment, through `from_pairs`.
    /// let kept: Vec<(&str, &str)> = env.vars().filter(|(key, _)| *key != "B").collect();
    /// assert_eq!(Environment::from_pairs(&kept).get("A"), Some("1"));
    /// assert_eq!(Environment::from_pairs(&kept).get("B"), None);
    /// ```
    ///
    /// ⚠️ Key order, not launch order. The snapshot is a `BTreeMap`, and no
    /// caller has ever needed the order a process was handed its variables in.
    pub fn vars(&self) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.vars
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// The user's home directory, and never a failure.
    ///
    /// `HOME` first, then `USERPROFILE`, then `/`. An empty value counts as
    /// absent at both steps, because `PathBuf::from("")` joined with anything
    /// yields a *relative* path, which is a worse answer than the fallback.
    ///
    /// ⚠️ The `USERPROFILE` step and the `/` fallback on Windows are
    /// compile-verified and nothing more. Nobody on this project has Windows
    /// hardware. See the README.
    pub fn home(&self) -> PathBuf {
        self.get("HOME")
            .filter(|value| !value.is_empty())
            .or_else(|| self.get("USERPROFILE").filter(|value| !value.is_empty()))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"))
    }

    /// Expands a leading `~` through [`Environment::home`].
    ///
    /// `~` answers the home directory, `~/rest` answers it joined with *rest*,
    /// and **everything else is returned unchanged**. Promoted from two
    /// independent implementations that agree exactly on those three rules:
    /// project-finder's `expanduser` and agentic-panes-layout's `expand_home`.
    ///
    /// ```
    /// use herdr_plugin_kit::env::Environment;
    /// use std::path::PathBuf;
    ///
    /// let env = Environment::from_pairs(&[("HOME", "/home/mike")]);
    ///
    /// assert_eq!(env.expanduser("~"), PathBuf::from("/home/mike"));
    /// assert_eq!(env.expanduser("~/.config"), PathBuf::from("/home/mike/.config"));
    /// assert_eq!(env.expanduser("/etc/hosts"), PathBuf::from("/etc/hosts"));
    /// ```
    ///
    /// ⚠️ **`~other` is not expanded**, and that is the promoted behaviour
    /// rather than an omission: resolving another user's home needs the
    /// password database, which this type deliberately cannot reach. A path
    /// beginning `~other` comes back as it went in, which is wrong in a way a
    /// caller can see, rather than resolved to this user's home, which is
    /// wrong in a way they cannot.
    ///
    /// 🔑 **It answers a [`PathBuf`], where one donor answered a `String`
    /// through `to_string_lossy`.** Two of that donor's three call sites wrap
    /// the result in `PathBuf::from` immediately, and the lossy step cannot
    /// round-trip a path that is not UTF-8. A caller that needs a string
    /// converts at its own edge, where the loss is visible.
    pub fn expanduser(&self, value: &str) -> PathBuf {
        if value == "~" {
            return self.home();
        }
        match value.strip_prefix("~/") {
            Some(rest) => self.home().join(rest),
            None => PathBuf::from(value),
        }
    }
}

/// Parses `KEY=value` lines, skipping anything malformed rather than failing.
///
/// Skipping is the promoted behaviour, and it is right for this input: these
/// files are written by shell shims, and one unreadable line must not cost the
/// caller the other twenty. Nothing here reports an error, so a caller that
/// needs to know a key was absent checks for the key.
///
/// - A line with no `=` is skipped, and so is an empty one.
/// - A line whose first non-blank character is `#` is a comment.
/// - Key and value are trimmed.
/// - One matching pair of wrapping quotes is removed, single or double.
/// - A key that trims to nothing is skipped. A value that does is kept.
pub fn parse_env_file(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || !line.contains('=') {
            continue;
        }
        // `contains` above already proved the separator is there.
        let (key, value) = line.split_once('=').unwrap();
        let key = key.trim();
        let mut value = value.trim().to_string();
        let characters: Vec<char> = value.chars().collect();
        if characters.len() >= 2
            && characters[0] == characters[characters.len() - 1]
            && (characters[0] == '"' || characters[0] == '\'')
        {
            value = characters[1..characters.len() - 1].iter().collect();
        }
        if !key.is_empty() {
            pairs.push((key.to_string(), value));
        }
    }
    pairs
}

/// Reads and parses a `KEY=value` file, answering an empty list if it cannot.
///
/// A file that is absent and a file that is empty give the same answer, on
/// purpose: both mean "this file set nothing". A caller that needs to tell
/// them apart asks the filesystem, because that is a different question.
pub fn read_env_file(path: &Path) -> Vec<(String, String)> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_env_file(&text),
        Err(_) => Vec::new(),
    }
}
