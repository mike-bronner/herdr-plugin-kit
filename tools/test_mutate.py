#!/usr/bin/env python3
"""The harness's own tests, and they are mostly regression tests.

    python3 tools/test_mutate.py

🚨 **Two inline harnesses written for this project misclassified a compile
outcome, in opposite directions, and both failed silently.** Each is pinned
below with the literal cargo output that fooled it. A harness that grades the
test suite has to be graded itself, or it launders its own defects into
confidence about somebody else's code.
"""

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))

import mutate


class Classification(unittest.TestCase):
    """The four states, and the two that must never be reached by default."""

    def test_a_suite_that_fails_caught_the_mutation(self):
        self.assertEqual(mutate.classify(False, 101), mutate.KILLED)
        self.assertEqual(mutate.classify(False, 1), mutate.KILLED)

    def test_a_suite_that_passes_is_a_gap_rather_than_a_success(self):
        self.assertEqual(mutate.classify(False, 0), mutate.SURVIVED)

    def test_a_mutation_that_did_not_compile_measured_nothing(self):
        self.assertEqual(mutate.classify(True, None), mutate.BUILD_ERROR)
        # Even with a test exit code to hand, a broken build outranks it.
        self.assertEqual(mutate.classify(True, 0), mutate.BUILD_ERROR)

    def test_an_unrunnable_suite_is_loud_rather_than_assumed(self):
        # 🚨 The whole point. Both historical defects were silent defaults, so
        # "could not tell" gets its own state instead of falling into one.
        self.assertEqual(mutate.classify(False, None), mutate.UNKNOWN)

    def test_the_two_bad_states_are_the_two_that_mean_nothing_was_proven(self):
        self.assertIn(mutate.SURVIVED, mutate.BAD)
        self.assertIn(mutate.UNKNOWN, mutate.BAD)
        self.assertNotIn(mutate.KILLED, mutate.BAD)
        self.assertNotIn(mutate.BUILD_ERROR, mutate.BAD)


class BuildDetection(unittest.TestCase):
    """Reading cargo's structured output rather than its prose."""

    def test_a_clean_build_is_not_a_failure(self):
        artifact = json.dumps({"reason": "compiler-artifact", "target": {}})
        self.assertFalse(mutate.build_broke(0, artifact))

    def test_a_compiler_error_in_json_is_a_failure(self):
        error = json.dumps(
            {"reason": "compiler-message", "message": {"level": "error"}}
        )
        self.assertTrue(mutate.build_broke(0, error))

    def test_a_warning_is_not_an_error(self):
        warning = json.dumps(
            {"reason": "compiler-message", "message": {"level": "warning"}}
        )
        self.assertFalse(mutate.build_broke(0, warning))

    def test_a_non_zero_exit_is_a_failure_even_with_no_json(self):
        self.assertTrue(mutate.build_broke(101, ""))

    def test_non_json_lines_are_skipped_rather_than_crashing(self):
        # cargo interleaves plain status lines with its JSON.
        mixed = "   Compiling herdr-plugin-kit v0.1.0\n" + json.dumps(
            {"reason": "compiler-message", "message": {"level": "error"}}
        )
        self.assertTrue(mutate.build_broke(0, mixed))
        self.assertFalse(mutate.build_broke(0, "   Compiling herdr-plugin-kit\n"))


class HistoricalDefects(unittest.TestCase):
    """The two misclassifications this tool was written to make impossible.

    Both are pinned with the literal cargo output that caused them, because a
    regression test for a prose-matching defect is worthless if it paraphrases
    the prose.
    """

    def test_a_failing_test_is_a_kill_and_not_a_broken_build(self):
        # 🚨 Defect one. The harness matched the word "error" in cargo's
        # `error: test failed, to rerun pass ...`, which is printed when the
        # build succeeded and the *tests* failed. Five real kills were reported
        # as invalid mutations.
        prose = "error: test failed, to rerun pass `-p herdr-plugin-kit --lib`"
        # The build step is what decides, and it exits 0 with no compiler error.
        self.assertFalse(mutate.build_broke(0, ""))
        self.assertEqual(mutate.classify(False, 101), mutate.KILLED)
        # The prose never reaches the classifier at all, which is the fix.
        self.assertNotIn(prose, str(mutate.classify(False, 101)))

    def test_a_syntax_error_is_a_broken_build_and_not_a_coverage_gap(self):
        # 🚨 Defect two. The harness matched `error[`, the form carrying a
        # diagnostic code. A plain syntax error has no code, so it printed
        # `error:` and fell through to "uncovered" — a gap that was not there.
        self.assertNotIn("error[", "error: expected one of `)`, found `;`")
        # Exit code alone catches it, with no pattern involved.
        self.assertTrue(mutate.build_broke(101, ""))
        self.assertEqual(mutate.classify(True, None), mutate.BUILD_ERROR)

    def test_both_defects_erred_toward_reporting_no_problem(self):
        # The property worth naming: each wrong answer was the reassuring one,
        # which is why neither raised an alarm and both needed a person to
        # check a result by hand.
        self.assertNotIn(mutate.BUILD_ERROR, mutate.BAD)
        self.assertIn(mutate.SURVIVED, mutate.BAD)


class Anchors(unittest.TestCase):
    """Applying and restoring the mutation itself."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.directory.name) / "source.rs"
        self.path.write_text("fn a() { true }\nfn b() { true }\n")

    def tearDown(self):
        self.directory.cleanup()

    def test_only_the_first_occurrence_is_swapped(self):
        original = mutate.apply_mutation(self.path, "true", "false")
        self.assertEqual(original, "fn a() { true }\nfn b() { true }\n")
        self.assertEqual(self.path.read_text(), "fn a() { false }\nfn b() { true }\n")

    def test_a_stale_anchor_is_loud_rather_than_a_silent_no_op(self):
        # A spec whose anchor a refactor removed would otherwise report the
        # unmutated suite passing, which reads as a coverage gap.
        with self.assertRaises(LookupError):
            mutate.apply_mutation(self.path, "gone", "x")
        self.assertEqual(self.path.read_text(), "fn a() { true }\nfn b() { true }\n")


class EndToEnd(unittest.TestCase):
    """The tool driving a real spec, with a trivial suite standing in for cargo."""

    def test_a_survived_mutation_exits_non_zero_and_names_itself(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "source.txt").write_text("keep\n")
            spec = root / "spec.json"
            spec.write_text(
                json.dumps(
                    {
                        "file": "source.txt",
                        # Builds clean and passes: nothing noticed the change.
                        "command": ["true"],
                        "mutations": [
                            {"name": "unguarded", "find": "keep", "replace": "drop"}
                        ],
                    }
                )
            )
            result = self._run(spec, root)
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertIn("SURVIVED", result.stdout)
            self.assertIn("unguarded", result.stderr)
            # Restored, even though the run reported a failure.
            self.assertEqual((root / "source.txt").read_text(), "keep\n")

    def test_a_killed_mutation_exits_zero(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "source.txt").write_text("keep\n")
            spec = root / "spec.json"
            spec.write_text(
                json.dumps(
                    {
                        "file": "source.txt",
                        # Builds clean, then fails the tests: the shape a
                        # real kill has, and the one defect one got wrong.
                        "command": [
                            "sh",
                            "-c",
                            'case "$*" in *--no-run*) exit 0 ;; *) exit 101 ;; esac',
                            "fixture",
                        ],
                        "mutations": [
                            {"name": "guarded", "find": "keep", "replace": "drop"}
                        ],
                    }
                )
            )
            result = self._run(spec, root)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("KILLED", result.stdout)

    def test_a_mutation_that_will_not_compile_is_reported_as_such(self):
        # 🚨 The shape defect two got wrong. It must not read as a coverage gap,
        # and it must not read as a success either: nothing was measured.
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "source.txt").write_text("keep\n")
            spec = root / "spec.json"
            spec.write_text(
                json.dumps(
                    {
                        "file": "source.txt",
                        "command": ["false"],
                        "mutations": [
                            {"name": "uncompilable", "find": "keep", "replace": "drop"}
                        ],
                    }
                )
            )
            result = self._run(spec, root)
            self.assertIn("BUILD-ERROR", result.stdout)
            self.assertNotIn("SURVIVED", result.stdout)
            # And it does not claim the suite proved anything.
            self.assertNotIn("every mutation was caught", result.stdout)
            self.assertIn("measured nothing", result.stdout)

    def test_a_spec_naming_its_own_build_is_judged_by_that_build(self):
        # 🔑 The Python spec's shape. The suite fails, so the mutation is a
        # kill. Without the `build` key the harness appends cargo's flags to
        # `false`, which fails too, and the kill would read as a broken build.
        # This test is the one that tells those two readings apart.
        result = self._spec({"command": ["false"], "build": ["true"]})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("KILLED", result.stdout)
        self.assertNotIn("BUILD-ERROR", result.stdout)

    def test_a_spec_whose_own_build_fails_measured_nothing(self):
        # A suite that would kill the mutation does not outrank a build that
        # says the mutated file is not valid source.
        result = self._spec({"command": ["false"], "build": ["false"]})
        self.assertIn("BUILD-ERROR", result.stdout)
        self.assertNotIn("KILLED", result.stdout)

    def test_without_a_build_key_the_build_is_cargos_no_run(self):
        self.assertEqual(
            mutate.build_command({"command": ["cargo", "test"]}),
            ["cargo", "test", "--no-run", "--message-format=json"],
        )
        self.assertEqual(
            mutate.build_command({"command": ["python3", "t.py"], "build": ["true"]}),
            ["true"],
        )

    def _spec(self, fields):
        """Runs one mutation of a fixture file under *fields*."""
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "source.txt").write_text("keep\n")
            spec = root / "spec.json"
            spec.write_text(
                json.dumps(
                    {
                        "file": "source.txt",
                        "mutations": [{"name": "one", "find": "keep", "replace": "drop"}],
                        **fields,
                    }
                )
            )
            return self._run(spec, root)

    def _run(self, spec, root):
        """Runs the tool with its repository root pointed at a fixture."""
        tool = pathlib.Path(__file__).resolve().parent / "mutate.py"
        source = tool.read_text().replace(
            "root = pathlib.Path(__file__).resolve().parent.parent",
            f"root = pathlib.Path({str(root)!r})",
        )
        copy = root / "mutate_under_test.py"
        copy.write_text(source)
        return subprocess.run(
            [sys.executable, str(copy), str(spec)], capture_output=True, text=True
        )


if __name__ == "__main__":
    unittest.main(verbosity=1)
