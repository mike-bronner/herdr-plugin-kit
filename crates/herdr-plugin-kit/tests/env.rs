//! What the `env` module promises, tested through its public surface.

use std::path::PathBuf;

use herdr_plugin_kit::env::{
    parse_env_file, read_env_file, Environment, BIN_PATH_VAR, CONFIG_PATH_VAR,
    PLUGIN_CONFIG_DIR_VAR, PLUGIN_EVENT_JSON_VAR, PLUGIN_EVENT_VAR, PLUGIN_ROOT_VAR,
    PLUGIN_STATE_DIR_VAR, SOCKET_PATH_VAR,
};

/// A directory that removes itself, so a failing test leaks nothing.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "herdr-plugin-kit-env-{}-{}-{:?}",
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
        std::fs::write(&path, text).expect("cannot write the fixture");
        path
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_eight_constants_are_the_eight_variables_a_plugin_actually_reads() {
    assert_eq!(
        [
            SOCKET_PATH_VAR,
            BIN_PATH_VAR,
            CONFIG_PATH_VAR,
            PLUGIN_ROOT_VAR,
            PLUGIN_CONFIG_DIR_VAR,
            PLUGIN_STATE_DIR_VAR,
            PLUGIN_EVENT_VAR,
            PLUGIN_EVENT_JSON_VAR,
        ],
        [
            "HERDR_SOCKET_PATH",
            "HERDR_BIN_PATH",
            "HERDR_CONFIG_PATH",
            "HERDR_PLUGIN_ROOT",
            "HERDR_PLUGIN_CONFIG_DIR",
            "HERDR_PLUGIN_STATE_DIR",
            "HERDR_PLUGIN_EVENT",
            "HERDR_PLUGIN_EVENT_JSON",
        ],
        "these names are a wire contract with Herdr, so a typo here is a variable that \
         is silently never read"
    );
}

#[test]
fn the_five_variables_measured_to_have_no_readers_are_not_here() {
    // SCOPE.md §5.1 cut these after counting readers across all three plugin
    // repositories and finding zero for each. This test is what stops one
    // creeping back in on a guess about what Herdr injects.
    let declared = [
        SOCKET_PATH_VAR,
        BIN_PATH_VAR,
        CONFIG_PATH_VAR,
        PLUGIN_ROOT_VAR,
        PLUGIN_CONFIG_DIR_VAR,
        PLUGIN_STATE_DIR_VAR,
        PLUGIN_EVENT_VAR,
        PLUGIN_EVENT_JSON_VAR,
    ];
    for cut in [
        "HERDR_ENV",
        "HERDR_PLUGIN_ID",
        "HERDR_PLUGIN_CONTEXT_JSON",
        "HERDR_PLUGIN_ACTION_ID",
        "HERDR_PLUGIN_ENTRYPOINT_ID",
    ] {
        assert!(
            !declared.contains(&cut),
            "{} was measured to have no readers",
            cut
        );
    }
}

#[test]
fn a_variable_that_was_never_set_and_one_set_to_nothing_are_different_answers() {
    let env = Environment::from_pairs(&[(PLUGIN_ROOT_VAR, ""), (PLUGIN_STATE_DIR_VAR, "/s")]);

    assert_eq!(env.get(PLUGIN_ROOT_VAR), Some(""));
    assert_eq!(env.get(PLUGIN_STATE_DIR_VAR), Some("/s"));
    assert_eq!(env.get(SOCKET_PATH_VAR), None);

    assert_eq!(
        env.get(PLUGIN_ROOT_VAR).filter(|value| !value.is_empty()),
        None,
        "the documented idiom has to collapse the two, because most callers want it to"
    );
}

#[test]
fn from_process_reads_the_real_environment_rather_than_an_empty_one() {
    let env = Environment::from_process();
    let path = std::env::var("PATH").expect("this test needs PATH set");
    assert_eq!(env.get("PATH"), Some(path.as_str()));
    assert!(!path.is_empty());
}

#[test]
fn home_prefers_home_then_userprofile_then_the_root() {
    let cases: [(&[(&str, &str)], &str); 6] = [
        (&[("HOME", "/u/mike")], "/u/mike"),
        (
            &[("HOME", "/u/mike"), ("USERPROFILE", "C:\\Users\\mike")],
            "/u/mike",
        ),
        (&[("USERPROFILE", "C:\\Users\\mike")], "C:\\Users\\mike"),
        (
            &[("HOME", ""), ("USERPROFILE", "C:\\Users\\mike")],
            "C:\\Users\\mike",
        ),
        (&[("HOME", ""), ("USERPROFILE", "")], "/"),
        (&[], "/"),
    ];
    for (pairs, expected) in cases {
        assert_eq!(
            Environment::from_pairs(pairs).home(),
            PathBuf::from(expected),
            "{:?}",
            pairs
        );
    }
}

#[test]
fn an_empty_home_never_becomes_a_relative_path() {
    let home = Environment::from_pairs(&[("HOME", "")]).home();
    assert!(
        home.join("state").is_absolute(),
        "an empty HOME that reached PathBuf::from would make every join relative: {:?}",
        home
    );
}

#[test]
fn parsing_keeps_the_pairs_and_skips_everything_malformed() {
    let parsed = parse_env_file(
        "\
# a comment
   # an indented comment

HERDR_PLUGIN_ROOT=/p
  SPACED  =  /q
QUOTED=\"/r\"
SINGLE='/s'
EMPTY=
WITH_EQUALS=a=b
no equals sign here
=orphan value
   =
MISMATCHED=\"/t'
JUST_ONE_QUOTE=\"
",
    );

    assert_eq!(
        parsed,
        vec![
            ("HERDR_PLUGIN_ROOT".to_string(), "/p".to_string()),
            ("SPACED".to_string(), "/q".to_string()),
            ("QUOTED".to_string(), "/r".to_string()),
            ("SINGLE".to_string(), "/s".to_string()),
            ("EMPTY".to_string(), String::new()),
            ("WITH_EQUALS".to_string(), "a=b".to_string()),
            ("MISMATCHED".to_string(), "\"/t'".to_string()),
            ("JUST_ONE_QUOTE".to_string(), "\"".to_string()),
        ]
    );
}

#[test]
fn only_a_matching_pair_of_wrapping_quotes_is_stripped() {
    // A single character cannot be a matching pair, and a quote at one end
    // only is part of the value. Getting either wrong silently changes a path.
    for (line, expected) in [
        ("K=\"\"", ""),
        ("K=''", ""),
        ("K=\"", "\""),
        ("K='", "'"),
        ("K=\"a", "\"a"),
        ("K=a\"", "a\""),
        ("K=\"a\"b\"", "a\"b"),
        ("K='a'", "a"),
        ("K=\"a'", "\"a'"),
    ] {
        assert_eq!(
            parse_env_file(line),
            vec![("K".to_string(), expected.to_string())],
            "{}",
            line
        );
    }
}

#[test]
fn a_key_that_trims_to_nothing_is_dropped_but_an_empty_value_is_kept() {
    assert_eq!(parse_env_file("=v"), vec![]);
    assert_eq!(parse_env_file("   =v"), vec![]);
    assert_eq!(
        parse_env_file("K="),
        vec![("K".to_string(), String::new())],
        "a key set to nothing is a fact the caller may need, so it survives"
    );
}

#[test]
fn reading_a_file_that_is_not_there_answers_nothing_rather_than_failing() {
    let dir = TempDir::new("missing");
    assert_eq!(read_env_file(&dir.join("absent")), vec![]);
    assert_eq!(
        read_env_file(dir.join("absent").as_path()),
        read_env_file(&dir.write("empty", "")),
        "absent and empty both mean this file set nothing"
    );
}

#[test]
fn reading_a_file_parses_it_the_same_way_the_string_form_does() {
    let dir = TempDir::new("read");
    let text = "A=1\n# skip\nB=\"2\"\n";
    let path = dir.write("shim.env", text);
    assert_eq!(read_env_file(&path), parse_env_file(text));
    assert_eq!(
        read_env_file(&path),
        vec![
            ("A".to_string(), "1".to_string()),
            ("B".to_string(), "2".to_string()),
        ]
    );
}

#[test]
fn a_directory_where_a_file_was_expected_answers_nothing_rather_than_panicking() {
    let dir = TempDir::new("directory");
    let path = dir.join("a-directory");
    std::fs::create_dir_all(&path).unwrap();
    assert_eq!(read_env_file(&path), vec![]);
}
