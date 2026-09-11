//! What the `version` module promises.
//!
//! The expensive test in here is deliberate.
//! [`the_macro_captures_the_consuming_crate_and_not_the_kit`] compiles a real
//! consumer crate, because that is the only way to prove the one claim the
//! module exists to make. Every cheaper test would expand the macro inside the
//! kit, where the kit *is* the calling crate, and so would pass whether the
//! macro were correct or not.

use std::path::{Path, PathBuf};
use std::process::Command;

use herdr_plugin_kit::env::Environment;
use herdr_plugin_kit::version::{
    self, Build, Manifest, Provenance, MANIFEST_FILE, PROVENANCE_SUFFIX, UNKNOWN,
};

/// A directory that removes itself, so a failing test leaks nothing.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "herdr-plugin-kit-version-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("cannot make a temporary directory");
        TempDir(path)
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, text).expect("cannot write the fixture");
        path
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn stamped(version: &'static str) -> Build {
    Build::new(version, Some("a1b2c3d"), Some("2026-09-11T04:39:22Z"))
}

fn found(version: &str) -> Manifest {
    Manifest::Found {
        version: version.to_string(),
        path: PathBuf::from("/p/herdr-plugin.toml"),
    }
}

fn line(report: &str, index: usize) -> String {
    report
        .lines()
        .nth(index)
        .unwrap_or_else(|| panic!("no line {} in:\n{}", index, report))
        .to_string()
}

// --- the report's shape -----------------------------------------------------

#[test]
fn the_report_states_three_facts_and_no_verdict_when_the_versions_agree() {
    let report = version::report(
        "watch",
        &stamped("0.5.0"),
        &found("0.5.0"),
        &Provenance::Compiled,
    );

    assert_eq!(
        report,
        "watch 0.5.0 (a1b2c3d, built 2026-09-11T04:39:22Z)\n\
         manifest 0.5.0 at /p/herdr-plugin.toml\n\
         built from source on this machine\n"
    );
    assert!(!report.contains("STALE"), "{}", report);
}

#[test]
fn the_three_fact_lines_are_unconditional() {
    // A report with a line missing would be ambiguous between "fine" and
    // "could not tell", so every failure mode still produces three lines.
    for manifest in [
        found("0.5.0"),
        Manifest::Unreadable(PathBuf::from("/p/herdr-plugin.toml")),
        Manifest::Unparsed(PathBuf::from("/p/herdr-plugin.toml")),
        Manifest::NoVersion(PathBuf::from("/p/herdr-plugin.toml")),
        Manifest::NoRoot,
    ] {
        for provenance in [
            Provenance::Compiled,
            Provenance::FetchedUnreadable,
            Provenance::Fetched {
                asset: None,
                url: None,
            },
        ] {
            let report = version::report("watch", &stamped("0.5.0"), &manifest, &provenance);
            assert!(
                report.lines().count() >= 3,
                "{:?} / {:?} gave:\n{}",
                manifest,
                provenance,
                report
            );
            assert!(report.ends_with('\n'), "{}", report);
        }
    }
}

#[test]
fn every_manifest_failure_is_named_rather_than_guessed_at() {
    let path = PathBuf::from("/p/herdr-plugin.toml");
    for (manifest, expected) in [
        (found("1.2.3"), "manifest 1.2.3 at /p/herdr-plugin.toml"),
        (
            Manifest::Unreadable(path.clone()),
            "manifest unreadable at /p/herdr-plugin.toml",
        ),
        (
            Manifest::Unparsed(path.clone()),
            "manifest unparsed at /p/herdr-plugin.toml",
        ),
        (
            Manifest::NoVersion(path),
            "manifest has no version key at /p/herdr-plugin.toml",
        ),
        (
            Manifest::NoRoot,
            "manifest not found: set HERDR_PLUGIN_ROOT to the plugin checkout to read it",
        ),
    ] {
        let report = version::report("watch", &stamped("1.2.3"), &manifest, &Provenance::Compiled);
        assert_eq!(line(&report, 1), expected, "{:?}", manifest);
    }
}

#[test]
fn a_manifest_that_parsed_is_never_reported_as_a_syntax_error() {
    let report = version::report(
        "watch",
        &stamped("1.2.3"),
        &Manifest::NoVersion(PathBuf::from("/p/herdr-plugin.toml")),
        &Provenance::Compiled,
    );
    assert!(!report.contains("unparsed"), "{}", report);
}

#[test]
fn every_provenance_gets_its_own_line() {
    for (provenance, expected) in [
        (Provenance::Compiled, "built from source on this machine"),
        (
            Provenance::Fetched {
                asset: Some("watch-macos-arm64-6c55e13a5445".to_string()),
                url: Some("https://example.test/a".to_string()),
            },
            "fetched watch-macos-arm64-6c55e13a5445 from https://example.test/a",
        ),
        (
            Provenance::Fetched {
                asset: Some("watch-macos-arm64-6c55e13a5445".to_string()),
                url: None,
            },
            "fetched watch-macos-arm64-6c55e13a5445, and the note records no url",
        ),
        (
            Provenance::Fetched {
                asset: None,
                url: Some("https://example.test/a".to_string()),
            },
            "fetched, and the note beside it names no asset",
        ),
        (
            Provenance::FetchedUnreadable,
            "fetched, and the note beside it could not be read",
        ),
    ] {
        let report = version::report("watch", &stamped("1.2.3"), &found("1.2.3"), &provenance);
        assert_eq!(line(&report, 2), expected, "{:?}", provenance);
    }
}

// --- staleness --------------------------------------------------------------

#[test]
fn the_stale_verdict_names_both_versions() {
    let report = version::report(
        "watch",
        &stamped("0.5.0"),
        &found("9.9.9"),
        &Provenance::Compiled,
    );
    assert_eq!(
        line(&report, 3),
        "STALE: this binary is 0.5.0 but the manifest is 9.9.9. \
         Rebuild it with `cargo build --release`."
    );
}

#[test]
fn the_remedy_follows_where_the_binary_came_from() {
    let fetched = Provenance::Fetched {
        asset: Some("watch-macos-arm64-6c55e13a5445".to_string()),
        url: None,
    };
    for provenance in [fetched, Provenance::FetchedUnreadable] {
        let report = version::report("watch", &stamped("0.5.0"), &found("9.9.9"), &provenance);
        assert_eq!(
            line(&report, 3),
            "STALE: this binary is 0.5.0 but the manifest is 9.9.9. \
             This binary was fetched, so reinstall the plugin to get the 9.9.9 binary."
        );
        assert!(
            !report.contains("cargo"),
            "whoever installed a fetched binary has no toolchain, so a rebuild is not \
             an instruction they can follow: {}",
            report
        );
    }
}

#[test]
fn a_manifest_that_could_not_be_read_produces_no_verdict_either_way() {
    // Staleness is unknown here, and an unknown must not be reported as
    // agreement. Three lines, no fourth.
    for manifest in [
        Manifest::Unreadable(PathBuf::from("/p/herdr-plugin.toml")),
        Manifest::Unparsed(PathBuf::from("/p/herdr-plugin.toml")),
        Manifest::NoVersion(PathBuf::from("/p/herdr-plugin.toml")),
        Manifest::NoRoot,
    ] {
        let report = version::report("watch", &stamped("0.5.0"), &manifest, &Provenance::Compiled);
        assert_eq!(
            report.lines().count(),
            3,
            "{:?} gave:\n{}",
            manifest,
            report
        );
        assert!(!report.contains("STALE"), "{}", report);
    }
}

// --- nothing may fail -------------------------------------------------------

#[test]
fn a_report_still_says_something_when_every_lookup_it_depends_on_fails() {
    // No stamp, no manifest, no binary to find a note beside. This is the
    // state the report exists for: everything is broken, and it still has to
    // answer.
    let build = Build::new(env!("CARGO_PKG_VERSION"), None, None);
    let manifest =
        version::read_manifest(version::root_of(&Environment::default(), None).as_deref());
    let provenance = version::provenance_of(None);

    assert_eq!(build.commit, UNKNOWN);
    assert_eq!(build.built, UNKNOWN);
    assert_eq!(manifest, Manifest::NoRoot);
    assert_eq!(provenance, Provenance::Compiled);

    let report = version::report("watch", &build, &manifest, &provenance);
    assert_eq!(report.lines().count(), 3, "{}", report);
    assert_eq!(
        line(&report, 0),
        format!(
            "watch {} (unknown, built unknown)",
            env!("CARGO_PKG_VERSION")
        )
    );
    assert_eq!(
        line(&report, 1),
        "manifest not found: set HERDR_PLUGIN_ROOT to the plugin checkout to read it",
        "a report that cannot find a manifest still says how to point it at one"
    );
    assert!(!report.is_empty());
}

#[test]
fn an_absent_or_empty_stamp_degrades_to_a_word_rather_than_a_blank() {
    for (commit, built) in [
        (None, None),
        (Some(""), Some("")),
        (Some("   "), Some("\n")),
    ] {
        let build = Build::new("0.5.0", commit, built);
        assert_eq!(build.commit, UNKNOWN, "{:?}", commit);
        assert_eq!(build.built, UNKNOWN, "{:?}", built);
        let report = version::report("watch", &build, &Manifest::NoRoot, &Provenance::Compiled);
        assert_eq!(line(&report, 0), "watch 0.5.0 (unknown, built unknown)");
    }
}

// --- reading the filesystem -------------------------------------------------

#[test]
fn reading_a_manifest_tells_the_failures_apart() {
    let dir = TempDir::new("manifest");
    assert_eq!(version::read_manifest(None), Manifest::NoRoot);
    assert_eq!(
        version::read_manifest(Some(dir.path())),
        Manifest::Unreadable(dir.join(MANIFEST_FILE))
    );

    dir.write(MANIFEST_FILE, "version = \"unterminated\n");
    assert_eq!(
        version::read_manifest(Some(dir.path())),
        Manifest::Unparsed(dir.join(MANIFEST_FILE))
    );

    dir.write(MANIFEST_FILE, "id = \"x\"\n");
    assert_eq!(
        version::read_manifest(Some(dir.path())),
        Manifest::NoVersion(dir.join(MANIFEST_FILE)),
        "this manifest parsed, so calling it unparsed names a fault that is not there"
    );

    dir.write(MANIFEST_FILE, "version = 3\n");
    assert_eq!(
        version::read_manifest(Some(dir.path())),
        Manifest::NoVersion(dir.join(MANIFEST_FILE)),
        "a version that is not a string is not a version this can report"
    );

    dir.write(MANIFEST_FILE, "version = \"1.2.3\"\n");
    assert_eq!(
        version::read_manifest(Some(dir.path())),
        Manifest::Found {
            version: "1.2.3".to_string(),
            path: dir.join(MANIFEST_FILE)
        }
    );
}

#[test]
fn a_note_beside_the_binary_is_what_tells_a_fetch_from_a_compile() {
    let dir = TempDir::new("provenance");
    let binary = dir.write("watch", "");
    assert_eq!(version::provenance_of(Some(&binary)), Provenance::Compiled);

    let note = dir.write(
        &format!("watch{}", PROVENANCE_SUFFIX),
        "version=0.5.0\nasset=watch-macos-arm64-6c55e13a5445\n\
         sha256=abc\nurl=https://example.test/a\n",
    );
    assert_eq!(
        version::provenance_of(Some(&binary)),
        Provenance::Fetched {
            asset: Some("watch-macos-arm64-6c55e13a5445".to_string()),
            url: Some("https://example.test/a".to_string()),
        }
    );

    std::fs::remove_file(&note).unwrap();
    std::fs::create_dir(&note).unwrap();
    assert_eq!(
        version::provenance_of(Some(&binary)),
        Provenance::Compiled,
        "the shim asks `[ -f ]` of this path, so a directory of that name is not a \
         note to either of them"
    );

    assert_eq!(
        version::provenance_of(None),
        Provenance::Compiled,
        "a binary that cannot find itself reads as compiled, which is what the shim \
         assumes when no note is there"
    );
}

#[test]
fn a_thin_note_still_proves_the_binary_was_fetched() {
    let dir = TempDir::new("thin");
    let binary = dir.write("watch", "");
    dir.write(&format!("watch{}", PROVENANCE_SUFFIX), "version=0.5.0\n");
    assert_eq!(
        version::provenance_of(Some(&binary)),
        Provenance::Fetched {
            asset: None,
            url: None
        },
        "the note's existence is the fact that decides rebuild from reinstall"
    );

    dir.write(&format!("watch{}", PROVENANCE_SUFFIX), "asset=\nurl=\n");
    assert_eq!(
        version::provenance_of(Some(&binary)),
        Provenance::Fetched {
            asset: None,
            url: None
        },
        "a key set to nothing names nothing"
    );
}

/// ⚠️ **Unix only**, because it works by making a file unreadable, and Windows
/// read-only permissions do not stop a read. Windows here is compile-verified
/// and nothing more.
///
/// This test exists because a mutation survived without it. Replacing the
/// `FetchedUnreadable` branch with `unwrap_or_default()` left the whole suite
/// green, since every other test constructs that variant by hand instead of
/// reaching it.
#[cfg(unix)]
#[test]
fn a_note_that_cannot_be_read_is_never_mistaken_for_no_note_at_all() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new("unreadable");
    let binary = dir.write("watch", "");
    let note = dir.write(
        &format!("watch{}", PROVENANCE_SUFFIX),
        "version=0.5.0\nasset=watch-macos-arm64-6c55e13a5445\n",
    );
    std::fs::set_permissions(&note, std::fs::Permissions::from_mode(0o000)).unwrap();

    // The whole test rests on this, so it is asserted rather than assumed. It
    // fails when the suite runs as root, which no CI runner here does.
    assert!(
        std::fs::read_to_string(&note).is_err(),
        "the note is still readable, so this test can prove nothing. Running as root \
         would do that."
    );

    let provenance = version::provenance_of(Some(&binary));
    std::fs::set_permissions(&note, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(provenance, Provenance::FetchedUnreadable);
    assert_ne!(
        provenance,
        Provenance::Compiled,
        "a note that exists is evidence the binary was fetched, so reading this as \
         compiled would tell a user with no toolchain to run cargo"
    );

    let report = version::report("watch", &stamped("0.5.0"), &found("9.9.9"), &provenance);
    assert!(
        !report.contains("cargo"),
        "the remedy must follow the note's existence, not its contents:\n{}",
        report
    );
}

#[test]
fn the_root_comes_from_the_environment_first_and_the_binary_second() {
    let dir = TempDir::new("root");
    let env = Environment::from_pairs(&[("HERDR_PLUGIN_ROOT", dir.path().to_str().unwrap())]);
    assert_eq!(version::root_of(&env, None), Some(dir.path().to_path_buf()));

    let empty = Environment::from_pairs(&[("HERDR_PLUGIN_ROOT", "")]);
    assert_eq!(
        version::root_of(&empty, None),
        None,
        "an empty value is Herdr saying nothing, not Herdr naming the current directory"
    );

    // <root>/target/release/<name> puts the root three ancestors up.
    let exe = dir.write("target/release/watch", "");
    assert_eq!(
        version::root_of(&Environment::default(), Some(&exe)),
        None,
        "no manifest is there, so the shape of the path alone must not be believed"
    );

    dir.write(MANIFEST_FILE, "version = \"1.2.3\"\n");
    assert_eq!(
        version::root_of(&Environment::default(), Some(&exe)),
        Some(dir.path().to_path_buf())
    );

    assert_eq!(
        version::root_of(&env, Some(&exe)),
        Some(dir.path().to_path_buf()),
        "the environment is authoritative, because it is the only answer left once \
         the binary has been moved"
    );
}

// --- the boundary the two crates exist to keep ------------------------------

#[test]
fn the_kit_never_depends_on_the_build_stamp_crate() {
    // SCOPE.md §2: two crates rather than one buys exactly this. If the stamp
    // helper ever lands in [dependencies], every plugin carries it at runtime
    // and the second crate has stopped paying for itself.
    //
    // ⚠️ Parsed rather than grepped. This manifest's own comment contains the
    // string "[build-dependencies]", which satisfied a text search and made the
    // first version of this test fail for a reason that had nothing to do with
    // the boundary. A grep over a whole file answers a question about prose.
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("cannot read this crate's own manifest")
            .parse::<toml::Table>()
            .expect("this crate's own manifest does not parse");

    assert!(
        !manifest.contains_key("build-dependencies"),
        "this crate carries no build script and no build dependencies: {:?}",
        manifest.keys().collect::<Vec<_>>()
    );

    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(entries) = manifest.get(table).and_then(toml::Value::as_table) else {
            continue;
        };
        assert!(
            !entries.contains_key("herdr-plugin-kit-build"),
            "the stamp crate is a build-dependency of a *plugin*, never a {} of the \
             kit: {:?}",
            table,
            entries.keys().collect::<Vec<_>>()
        );
    }

    // The guard above only bites if it is reading the manifest it thinks it is.
    assert_eq!(
        manifest
            .get("package")
            .and_then(toml::Value::as_table)
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str),
        Some("herdr-plugin-kit"),
        "this test read the wrong manifest, so it was proving nothing"
    );
}

// --- the claim that needs a real consumer to prove --------------------------

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the kit lives two directories below the workspace root")
        .to_path_buf()
}

/// ⚠️ **Compiles a real crate**, so it is slower than everything above it.
///
/// It is the only test that can fail if the macro is turned back into a plain
/// function. Expanded inside the kit, `env!("CARGO_PKG_VERSION")` answers the
/// kit's version whether the design is right or wrong, so no test living here
/// alone can discriminate. The consumer declares `9.9.9` and the kit declares
/// something else, which is what makes the answer decisive.
#[test]
fn the_macro_captures_the_consuming_crate_and_not_the_kit() {
    let dir = TempDir::new("consumer");
    let root = workspace_root();
    let kit = root.join("crates/herdr-plugin-kit");
    let stamp = root.join("crates/herdr-plugin-kit-build");

    dir.write(
        "Cargo.toml",
        &format!(
            "[workspace]\n\
             [package]\n\
             name = \"consumer\"\n\
             version = \"9.9.9\"\n\
             edition = \"2021\"\n\n\
             [dependencies]\n\
             herdr-plugin-kit = {{ path = {kit:?} }}\n\n\
             [build-dependencies]\n\
             herdr-plugin-kit-build = {{ path = {stamp:?} }}\n",
            kit = kit,
            stamp = stamp
        ),
    );
    dir.write(
        "build.rs",
        "fn main() { herdr_plugin_kit_build::stamp(); }\n",
    );
    dir.write("herdr-plugin.toml", "version = \"9.9.9\"\n");
    dir.write(
        "src/main.rs",
        "fn main() {\n    \
         let environment = herdr_plugin_kit::env::Environment::from_process();\n    \
         print!(\"{}\", herdr_plugin_kit::version_report!(\"consumer\", &environment));\n\
         }\n",
    );

    let target = dir.join("target");
    let built = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "--manifest-path"])
        .arg(dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .current_dir(dir.path())
        .output()
        .expect("cannot run cargo");
    assert!(
        built.status.success(),
        "the consumer did not build: {}",
        String::from_utf8_lossy(&built.stderr)
    );

    let consumer = target.join("debug/consumer");
    let ask = |root: Option<&Path>| -> String {
        let mut command = Command::new(&consumer);
        command.env_remove("HERDR_PLUGIN_ROOT");
        if let Some(root) = root {
            command.env("HERDR_PLUGIN_ROOT", root);
        }
        let run = command.output().expect("cannot run the consumer");
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        String::from_utf8_lossy(&run.stdout).to_string()
    };

    let report = ask(Some(dir.path()));

    assert!(
        report.starts_with("consumer 9.9.9 ("),
        "the macro reported a version that is not the consumer's:\n{}",
        report
    );
    assert!(
        !report.contains(env!("CARGO_PKG_VERSION")),
        "the kit's own version ({}) reached a consumer's report, which is the exact \
         failure the macro exists to prevent:\n{}",
        env!("CARGO_PKG_VERSION"),
        report
    );
    assert!(
        !report.contains("(unknown, built unknown)"),
        "the consumer's build.rs called stamp(), so neither value may be unknown:\n{}",
        report
    );

    // The consumer is not a git checkout, so the commit degrades to a word
    // while the build instant is still real. Both come from the consumer's own
    // compilation rather than the kit's.
    let inside = report
        .lines()
        .next()
        .and_then(|first| first.split_once(" (").map(|(_, rest)| rest.to_string()))
        .expect(&report);
    let (commit, built_at) = inside
        .trim_end_matches(')')
        .split_once(", built ")
        .expect(&inside);
    assert_eq!(commit, UNKNOWN, "no git here, so the commit is a word");
    assert_eq!(
        built_at.len(),
        "0000-00-00T00:00:00Z".len(),
        "the build instant came from the consumer's own stamp: {}",
        built_at
    );

    assert_eq!(
        report.lines().nth(1).unwrap(),
        format!("manifest 9.9.9 at {}", dir.join(MANIFEST_FILE).display())
    );
    assert!(
        !report.contains("STALE"),
        "9.9.9 agrees with 9.9.9, so there is no verdict to give:\n{}",
        report
    );

    // With no HERDR_PLUGIN_ROOT the macro has to reach the manifest through the
    // binary's own location, which is the only thing left. The consumer sits at
    // <root>/target/debug/consumer, so the walk is what finds it.
    //
    // ⚠️ This half exists because a mutation survived without it. Passing
    // `None` for the executable left the whole suite green, since the run above
    // sets the variable and so never needs the walk.
    let walked = ask(None);
    assert_eq!(
        walked.lines().nth(1).unwrap(),
        format!("manifest 9.9.9 at {}", dir.join(MANIFEST_FILE).display()),
        "the macro did not pass the running binary through to the root walk:\n{}",
        walked
    );

    // And the same executable is what locates the provenance note.
    std::fs::write(
        consumer.with_file_name(format!("consumer{}", PROVENANCE_SUFFIX)),
        "version=9.9.9\nasset=consumer-macos-arm64-6c55e13a5445\n\
         url=https://example.test/a\n",
    )
    .unwrap();
    let fetched = ask(Some(dir.path()));
    assert_eq!(
        fetched.lines().nth(2).unwrap(),
        "fetched consumer-macos-arm64-6c55e13a5445 from https://example.test/a",
        "the same binary must answer for the note beside it:\n{}",
        fetched
    );
    assert_ne!(
        report.lines().nth(2).unwrap(),
        fetched.lines().nth(2).unwrap(),
        "the note changed nothing, so provenance is not being read at run time"
    );
}
