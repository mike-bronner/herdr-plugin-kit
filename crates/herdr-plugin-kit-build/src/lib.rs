//! Stamps a Herdr plugin binary with the tree it was built from.
//!
//! A plugin's `build.rs` calls one function, and does nothing else:
//!
//! ```no_run
//! herdr_plugin_kit_build::stamp();
//! ```
//!
//! That emits two environment variables into the *plugin's* compilation, which
//! [`herdr_plugin_kit::version`] then reads back through a macro:
//!
//! | Variable | Holds |
//! |---|---|
//! | `HERDR_PLUGIN_COMMIT` | the short commit, with a `-dirty` or `-unverified` marker |
//! | `HERDR_PLUGIN_BUILT` | the build instant, in UTC |
//!
//! # Why this is its own crate
//!
//! ⚠️ **So that it can never become a runtime dependency of a plugin.** It goes
//! in `[build-dependencies]`, where it is compiled for the host, used once, and
//! left out of the shipped binary. Folding it into `herdr-plugin-kit` would put
//! it in every plugin's dependency graph for the sake of one function that
//! stops being called the moment the build ends.
//!
//! # Why a stamp at all
//!
//! ✅ A running plugin was once found two commits behind its own source. The
//! manifest Herdr held was current and the compiled artifact was not, so two
//! features were registered without ever running. **The crate version could not
//! have caught it**, because under this release convention the version only
//! moves on a release commit, and the stale binary and the current manifest
//! both read `0.3.0`. The commit is what tells them apart.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// The commit this binary was built from, with its cleanliness marker.
pub const COMMIT_VAR: &str = "HERDR_PLUGIN_COMMIT";

/// The instant this binary was built, in UTC.
pub const BUILT_VAR: &str = "HERDR_PLUGIN_BUILT";

/// What a lookup answers when it cannot answer.
pub const UNKNOWN: &str = "unknown";

/// Everything the compiler reads, and nothing else.
///
/// **This one list defines what the marker means.** Both halves use it: the
/// paths cargo watches for a rebuild, and the pathspec `git status` is asked
/// about. Letting those two differ is what makes a marker incoherent, because
/// editing a README could then report `-dirty` while nothing rebuilt to notice.
///
/// The meaning chosen is **what was compiled**, not what `git status` says
/// about the tree. A commit on this binary claims it was built from that
/// commit, and only these files can make the claim false.
///
/// ⚠️ **`rust-toolchain` and `rust-toolchain.toml` are both listed, and that is
/// not redundant.** Git pathspecs match whole path components, so
/// `rust-toolchain` does not match `rust-toolchain.toml`. Measured, not
/// assumed. Drop either entry and a file the compiler reads goes unwatched.
///
/// ✅ This is the same list `bin/build` uses to decide whether a checkout is
/// releasable (SCOPE.md §9.4.1). Both answer "was this binary built from
/// committed source?", so they cannot be allowed to disagree.
pub const BUILD_INPUTS: [&str; 7] = [
    "src",
    "build.rs",
    "Cargo.toml",
    "Cargo.lock",
    ".cargo",
    "rust-toolchain",
    "rust-toolchain.toml",
];

/// The git files whose contents move when `HEAD` does.
///
/// `HEAD` alone is not enough. A commit made after `git pack-refs` writes a
/// loose ref where there was none, and a commit that edits no file changes
/// nothing cargo would otherwise watch.
const GIT_PATHS: [&str; 3] = ["HEAD", "refs", "packed-refs"];

/// Emits the stamp. Call this from a plugin's `build.rs`, and nothing else.
///
/// **Nothing here fails.** Every lookup degrades to a word, because a build
/// script that panicked would cost a plugin its whole build to report a commit.
pub fn stamp() {
    for line in directives(&package_root()) {
        println!("{}", line);
    }
}

/// The package being built, or the working directory if cargo did not say.
fn package_root() -> PathBuf {
    match std::env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from("."),
    }
}

/// Every line [`stamp`] prints, in order.
///
/// Split out from the printing so that the whole of it is testable. A function
/// whose only output is stdout can be read, but it cannot be checked.
fn directives(root: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    for input in BUILD_INPUTS {
        // ⚠️ Watched only when it exists. A `rerun-if-changed` pointing at a
        // path that is not there makes cargo rebuild on every single
        // invocation, which would make the timestamp move under an unchanged
        // binary. Most plugins carry no `.cargo` and no toolchain file.
        if root.join(input).exists() {
            lines.push(format!("cargo:rerun-if-changed={}", input));
        }
    }
    for path in git_watch_paths(root) {
        lines.push(format!("cargo:rerun-if-changed={}", path.display()));
    }
    lines.push(format!("cargo:rustc-env={}={}", COMMIT_VAR, commit(root)));
    lines.push(format!("cargo:rustc-env={}={}", BUILT_VAR, built()));
    lines
}

/// The git files worth watching, resolved through git and filtered to those
/// that exist. A worktree or a submodule does not keep them where you expect.
fn git_watch_paths(root: &Path) -> Vec<PathBuf> {
    GIT_PATHS
        .iter()
        .filter_map(|name| git(root, &["rev-parse", "--git-path", name]))
        .map(|found| match Path::new(&found).is_absolute() {
            true => PathBuf::from(found),
            false => root.join(found),
        })
        .filter(|path| path.exists())
        .collect()
}

/// The short commit, marked with what the tree says about it.
///
/// Three answers, and they are deliberately distinct:
///
/// - `abc1234` — built from exactly this commit.
/// - `abc1234-dirty` — a compiler-read path had uncommitted changes.
/// - `abc1234-unverified` — the status check could not run, so cleanliness is
///   unknown. ⚠️ **Unknown is not clean.** Reporting it as clean would be the
///   one lie this whole mechanism exists to prevent.
/// - `unknown` — there is no git here at all.
fn commit(root: &Path) -> String {
    let Some(short) = git(root, &["rev-parse", "--short", "HEAD"]) else {
        return UNKNOWN.to_string();
    };
    let mut status = vec!["status", "--porcelain", "--"];
    status.extend(BUILD_INPUTS);
    match run(root, &status) {
        Err(_) => format!("{}-unverified", short),
        Ok(output) if !output.status.success() => format!("{}-unverified", short),
        Ok(output) if !String::from_utf8_lossy(&output.stdout).trim().is_empty() => {
            format!("{}-dirty", short)
        }
        Ok(_) => short,
    }
}

/// Runs git in `root` and answers its trimmed output, or `None` for anything
/// short of a clean success with something to say.
fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = run(root, args).ok()?;
    if !output.status.success() {
        return None;
    }
    let answer = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match answer.is_empty() {
        true => None,
        false => Some(answer),
    }
}

fn run(root: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("git").args(args).current_dir(root).output()
}

/// Now, in UTC, to the second.
fn built() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    utc(seconds)
}

/// Formats a Unix instant as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Hand-rolled rather than pulled from `chrono` or `time`, because this crate's
/// dependency list being empty is what keeps it honest as a build-only helper.
/// The calendar arithmetic below is Howard Hinnant's `civil_from_days`, and it
/// is exact for every date this will ever see.
fn utc(seconds: u64) -> String {
    let (year, month, day) = civil_from_days((seconds / 86_400) as i64);
    let rest = seconds % 86_400;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A git repository that removes itself.
    struct Repo(PathBuf);

    impl Repo {
        fn bare_directory(tag: &str) -> Repo {
            let path = std::env::temp_dir().join(format!(
                "herdr-plugin-kit-build-{}-{}-{:?}",
                tag,
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("src")).expect("cannot make a temporary tree");
            std::fs::write(path.join("src/lib.rs"), "// nothing\n").unwrap();
            std::fs::write(path.join("Cargo.toml"), "[package]\n").unwrap();
            std::fs::write(path.join("README.md"), "compiled into nothing\n").unwrap();
            Repo(path)
        }

        fn committed(tag: &str) -> Repo {
            let repo = Repo::bare_directory(tag);
            repo.git(&["init", "--quiet"]);
            repo.git(&["add", "."]);
            repo.git(&["commit", "--quiet", "-m", "first"]);
            repo
        }

        fn git(&self, args: &[&str]) {
            let output = Command::new("git")
                .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
                .args(args)
                .current_dir(&self.0)
                .output()
                .expect("cannot run git");
            assert!(
                output.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }

        fn head(&self) -> String {
            git(&self.0, &["rev-parse", "--short", "HEAD"]).expect("no HEAD")
        }

        fn write(&self, name: &str, text: &str) {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, text).unwrap();
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_instant_is_formatted_as_utc_to_the_second() {
        for (seconds, expected) in [
            (0, "1970-01-01T00:00:00Z"),
            (86_399, "1970-01-01T23:59:59Z"),
            (86_400, "1970-01-02T00:00:00Z"),
            (951_782_399, "2000-02-28T23:59:59Z"),
            // A leap day in a year divisible by 400.
            (951_782_400, "2000-02-29T00:00:00Z"),
            (1_709_164_800, "2024-02-29T00:00:00Z"),
            (1_789_101_562, "2026-09-11T04:39:22Z"),
            // 2100 is divisible by 100 and not by 400, so it is not a leap
            // year. This is the case a naive "every four years" gets wrong.
            (4_102_444_800, "2100-01-01T00:00:00Z"),
            (4_107_542_400, "2100-03-01T00:00:00Z"),
        ] {
            assert_eq!(utc(seconds), expected, "{}", seconds);
        }
    }

    #[test]
    fn the_instant_has_a_fixed_width_whatever_the_date() {
        for seconds in [0, 1, 951_782_400, 1_789_101_562, 4_107_542_400] {
            assert_eq!(utc(seconds).len(), "0000-00-00T00:00:00Z".len());
        }
    }

    #[test]
    fn a_clean_checkout_reports_the_bare_commit() {
        let repo = Repo::committed("clean");
        assert_eq!(commit(repo.path()), repo.head());
    }

    #[test]
    fn an_uncommitted_compiled_file_makes_the_marker_dirty() {
        let repo = Repo::committed("dirty");
        repo.write("src/lib.rs", "// edited\n");
        assert_eq!(commit(repo.path()), format!("{}-dirty", repo.head()));
    }

    #[test]
    fn a_file_the_binary_is_not_compiled_from_cannot_make_the_marker_dirty() {
        let repo = Repo::committed("readme");
        repo.write("README.md", "edited, and compiled into nothing\n");
        assert_eq!(
            commit(repo.path()),
            repo.head(),
            "an uncommitted file that reaches no compiler cannot falsify the commit"
        );
    }

    #[test]
    fn both_toolchain_file_names_are_watched_because_one_does_not_match_the_other() {
        // ✅ Measured: a git pathspec matches whole path components, so
        // `rust-toolchain` does not match `rust-toolchain.toml`. Each name
        // therefore needs its own entry, and this test is what proves it.
        for name in ["rust-toolchain", "rust-toolchain.toml"] {
            let repo = Repo::committed("toolchain");
            repo.write(name, "[toolchain]\nchannel = \"1.80\"\n");
            assert_eq!(
                commit(repo.path()),
                format!("{}-dirty", repo.head()),
                "{} changes what the compiler does, so it has to be watched",
                name
            );
        }
    }

    /// ⚠️ **Unix only**, because it works by making a file unreadable, and
    /// Windows read-only permissions do not stop a read. Windows here is
    /// compile-verified and nothing more, so this branch goes unexercised
    /// there rather than being tested by something that proves less.
    #[cfg(unix)]
    #[test]
    fn a_tree_whose_state_cannot_be_checked_is_never_reported_as_clean() {
        use std::os::unix::fs::PermissionsExt;

        let repo = Repo::committed("unverified");
        let head = repo.head();
        let index = repo.path().join(".git/index");
        std::fs::set_permissions(&index, std::fs::Permissions::from_mode(0o000)).unwrap();

        // The whole test rests on this, so it is asserted rather than assumed.
        // It fails when the suite runs as root, which no CI runner here does.
        let broken = run(repo.path(), &["status", "--porcelain", "--", "src"]).unwrap();
        assert!(
            !broken.status.success(),
            "git status still worked with an unreadable index, so this test can prove \
             nothing. Running as root would do that."
        );

        let marker = commit(repo.path());
        std::fs::set_permissions(&index, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(marker, format!("{}-unverified", head));
        assert_ne!(
            marker, head,
            "a status check that could not run must never read as a clean tree"
        );
        assert!(!marker.ends_with("-dirty"), "{}", marker);
    }

    #[test]
    fn a_checkout_with_no_git_at_all_reports_unknown() {
        let repo = Repo::bare_directory("nogit");
        assert!(!repo.path().join(".git").exists());
        assert_eq!(commit(repo.path()), UNKNOWN);
    }

    #[test]
    fn a_directory_that_does_not_exist_reports_unknown_rather_than_panicking() {
        assert_eq!(
            commit(Path::new("/no/such/directory/anywhere")),
            UNKNOWN,
            "a build script that panicked here would cost the plugin its whole build"
        );
    }

    #[test]
    fn the_directives_name_both_variables_and_watch_only_what_is_there() {
        let repo = Repo::committed("directives");
        let lines = directives(repo.path());

        assert!(lines.contains(&"cargo:rerun-if-changed=src".to_string()));
        assert!(lines.contains(&"cargo:rerun-if-changed=Cargo.toml".to_string()));
        assert!(
            !lines.iter().any(|line| line.ends_with("=rust-toolchain")),
            "this tree has no rust-toolchain, and watching a path that is not there \
             rebuilds on every invocation: {:?}",
            lines
        );
        assert!(
            !lines.iter().any(|line| line.ends_with("=.cargo")),
            "{:?}",
            lines
        );

        let commit_line = format!("cargo:rustc-env={}={}", COMMIT_VAR, repo.head());
        assert!(lines.contains(&commit_line), "{:?}", lines);

        let built = lines
            .iter()
            .find_map(|line| line.strip_prefix(&format!("cargo:rustc-env={}=", BUILT_VAR)))
            .expect("no build instant emitted");
        assert_eq!(built.len(), "0000-00-00T00:00:00Z".len(), "{}", built);
        assert!(built.ends_with('Z'), "{}", built);
    }

    #[test]
    fn a_toolchain_file_that_is_there_is_watched() {
        let repo = Repo::committed("watched");
        repo.write("rust-toolchain.toml", "[toolchain]\n");
        assert!(
            directives(repo.path())
                .contains(&"cargo:rerun-if-changed=rust-toolchain.toml".to_string()),
            "the existence filter must not be a blanket exclusion"
        );
    }

    #[test]
    fn the_git_files_that_move_with_head_are_watched() {
        let repo = Repo::committed("gitpaths");
        let watched = git_watch_paths(repo.path());
        assert!(
            watched.iter().any(|path| path.ends_with("HEAD")),
            "a commit that edits no file changes nothing else cargo watches: {:?}",
            watched
        );
        assert!(
            watched.iter().all(|path| path.exists()),
            "a watched path that is not there rebuilds forever: {:?}",
            watched
        );
        assert!(
            !repo.path().join(".git/packed-refs").exists(),
            "this checkout packs no refs, which is exactly the absent-path case above"
        );
    }

    #[test]
    fn git_paths_resolve_against_the_package_rather_than_the_current_directory() {
        let repo = Repo::committed("relative");
        // `git rev-parse --git-path HEAD` answers `.git/HEAD`, a relative path.
        // Left relative it would be resolved against whatever directory cargo
        // happens to run the build script in.
        for path in git_watch_paths(repo.path()) {
            assert!(path.is_absolute(), "{:?}", path);
            assert!(path.starts_with(repo.path()), "{:?}", path);
        }
    }
}
