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
pub const BIN_PATH_VAR: &str = "HERDR_BIN_PATH";

/// Overrides where Herdr's own `config.toml` is read from.
pub const CONFIG_PATH_VAR: &str = "HERDR_CONFIG_PATH";

/// The plugin's checkout, which is where its manifest lives.
pub const PLUGIN_ROOT_VAR: &str = "HERDR_PLUGIN_ROOT";

/// Where this plugin's own configuration is kept.
pub const PLUGIN_CONFIG_DIR_VAR: &str = "HERDR_PLUGIN_CONFIG_DIR";

/// Where this plugin may write state that outlives one invocation.
pub const PLUGIN_STATE_DIR_VAR: &str = "HERDR_PLUGIN_STATE_DIR";

/// The name of the event that triggered this invocation.
pub const PLUGIN_EVENT_VAR: &str = "HERDR_PLUGIN_EVENT";

/// The event's payload, as JSON.
///
/// Per-invocation, so it is not queryable from the API at any later point.
pub const PLUGIN_EVENT_JSON_VAR: &str = "HERDR_PLUGIN_EVENT_JSON";

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
