#!/usr/bin/env python3
"""Tests for the one piece of logic that has to live in YAML.

🚨 **Everything else in CI lives in ``tools/`` for a reason** (SCOPE.md
§11.2.1): YAML cannot be run, so nothing that can be wrong belongs in it. This
block is the exception, and the exception is forced: it decides **which kit to
check out**, so it runs before there is a ``tools/`` to call.

It is therefore the only logic in this repository that ships untested by
construction — unless the tests come to it. So they do. This module extracts
the block from both workflows, proves the two copies are identical, and then
**runs the real text** under ``sh`` against fabricated GitHub contexts.

✅ That is the same arrangement ``tools/test_plugin_gate.py`` uses for the
asset-naming line: execute the shipped text rather than a copy of it, because
a copy agrees with itself while both sides are wrong together.

The cases below are not hypothetical. ⚠️ **The first real call of
``plugin-ci.yml`` failed here** (recent-spaces run 34708262498, 2026-09-12):
``github.job_workflow_sha`` was empty for a caller pinned at an annotated tag.

No network, and no dependency beyond the standard library. Run with::

    python3 tools/test_workflow_ref.py

Python 3.9 is the floor. This machine has no other interpreter.
"""

import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = (
    REPO_ROOT / ".github" / "workflows" / "plugin-ci.yml",
    REPO_ROOT / ".github" / "workflows" / "plugin-release.yml",
)

#: What marks the block in each workflow. Both markers are inside a comment,
#: so the shell ignores them and only this module reads them.
OPEN_MARKER = "# ---8<--- kit-ref resolution"
CLOSE_MARKER = "# --->8--- end kit-ref resolution"

#: The repository both workflows name, in the step's env and again in the
#: checkout. A test holds the two together, because the second decides what
#: code runs.
KIT_REPO = "mike-bronner/herdr-plugin-kit"

#: A 40-character hexadecimal string, which is what a commit looks like.
A_COMMIT = "326645d0" + "0" * 32


def extract(path):
    """Return the resolution block from *path*, dedented to column zero."""
    text = path.read_text(encoding="utf-8")
    start = text.index(OPEN_MARKER)
    end = text.index(CLOSE_MARKER, start) + len(CLOSE_MARKER)
    return textwrap.dedent(text[start:end])


class TheTwoCopiesAgree(unittest.TestCase):
    """Two files carry this block, and nothing else can keep them in step."""

    def test_the_block_is_byte_identical_in_both_workflows(self):
        first, second = (extract(path) for path in WORKFLOWS)
        self.assertEqual(first, second)

    def test_both_workflows_name_the_same_repository_twice(self):
        # The step's env decides what the block will accept, and the checkout
        # decides what is actually fetched. They have to be the same string.
        #
        # ⚠️ Whole lines, never a substring. `assertIn` on the repository name
        # passes against `…herdr-plugin-kit-typo`, which is a real way for the
        # two to drift apart while a test watches.
        for path in WORKFLOWS:
            named = set()
            for line in path.read_text(encoding="utf-8").splitlines():
                stripped = line.strip()
                for key in ("KIT_REPO:", "repository:"):
                    if stripped.startswith(key):
                        named.add(stripped[len(key):].strip())
            self.assertEqual(named, {KIT_REPO}, path.name)

    def test_both_workflows_check_out_what_the_block_resolved(self):
        for path in WORKFLOWS:
            text = path.read_text(encoding="utf-8")
            self.assertIn("ref: ${{ steps.kit.outputs.ref }}", text, path.name)


class Resolution(unittest.TestCase):
    """The shipped text, run against contexts GitHub might hand it."""

    def resolve(self, **context):
        """Run the real block and report (exit code, stdout+stderr, output)."""
        environment = {
            "KIT_REPO": KIT_REPO,
            "JOB_WORKFLOW_SHA": "",
            "JOB_WORKFLOW_REF": "",
            "WORKFLOW_REF": "",
            "WORKFLOW_SHA": "",
            "EVENT_SHA": "",
            "EVENT_REF": "",
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        }
        environment.update(context)
        with tempfile.TemporaryDirectory() as directory:
            written = Path(directory) / "github_output"
            written.touch()
            environment["GITHUB_OUTPUT"] = str(written)
            finished = subprocess.run(
                ["sh", "-e", "-c", extract(WORKFLOWS[0])],
                capture_output=True,
                text=True,
                env=environment,
            )
            return (
                finished.returncode,
                finished.stdout + finished.stderr,
                written.read_text(encoding="utf-8").strip(),
            )

    # ---- what it accepts -------------------------------------------------

    def test_a_commit_of_the_workflow_file_answers(self):
        code, _, output = self.resolve(JOB_WORKFLOW_SHA=A_COMMIT)
        self.assertEqual(code, 0)
        self.assertEqual(output, f"ref={A_COMMIT}")

    def test_a_ref_path_this_kit_owns_answers(self):
        # 🚨 The case the first real call needed and did not get. The runner
        # resolved this file to exactly this string; everything after the last
        # '@' is a ref `actions/checkout` takes as it stands.
        code, _, output = self.resolve(
            JOB_WORKFLOW_REF=f"{KIT_REPO}/.github/workflows/plugin-ci.yml@refs/tags/0.3.0"
        )
        self.assertEqual(code, 0)
        self.assertEqual(output, "ref=refs/tags/0.3.0")

    def test_workflow_ref_answers_when_job_workflow_ref_does_not(self):
        code, _, output = self.resolve(
            WORKFLOW_REF=f"{KIT_REPO}/.github/workflows/plugin-ci.yml@refs/heads/main"
        )
        self.assertEqual(code, 0)
        self.assertEqual(output, "ref=refs/heads/main")

    def test_a_branch_a_tag_and_a_sha_are_all_taken_as_they_come(self):
        # The ref form is not inspected on purpose: the runner already
        # resolved it, and a check here would be a second opinion about what
        # it meant.
        for ref in ("refs/heads/main", "refs/tags/0.3.0", A_COMMIT):
            _, _, output = self.resolve(
                JOB_WORKFLOW_REF=f"{KIT_REPO}/.github/workflows/plugin-ci.yml@{ref}"
            )
            self.assertEqual(output, f"ref={ref}", ref)

    def test_the_commit_wins_when_both_are_present(self):
        code, _, output = self.resolve(
            JOB_WORKFLOW_SHA=A_COMMIT,
            JOB_WORKFLOW_REF=f"{KIT_REPO}/.github/workflows/plugin-ci.yml@refs/heads/main",
        )
        self.assertEqual(code, 0)
        self.assertEqual(output, f"ref={A_COMMIT}")

    # ---- what it refuses -------------------------------------------------

    def test_an_empty_context_stops_the_job(self):
        # ✅ What actually happened on 2026-09-12. It failing closed is why
        # that run cost a red build rather than a wrong answer.
        code, said, output = self.resolve()
        self.assertEqual(code, 1)
        self.assertEqual(output, "")
        self.assertIn("no context value named the kit", said)

    def test_a_ref_path_another_repository_owns_is_refused(self):
        # 🚨 The fail-open this guard exists for, and the reason the
        # repository check is not decoration. `refs/heads/main` exists in this
        # kit too, so taking a caller's own ref would check the plugin against
        # the kit's default branch and *pass*.
        code, _, output = self.resolve(
            WORKFLOW_REF="mike-bronner/herdr-plugin-recent-spaces"
            "/.github/workflows/ci.yml@refs/heads/main"
        )
        self.assertEqual(code, 1)
        self.assertEqual(output, "")

    def test_a_repository_whose_name_merely_starts_the_same_is_refused(self):
        # `herdr-plugin-kit-evil` starts with the kit's name, and the '/' in
        # the pattern is what keeps it out.
        code, _, _ = self.resolve(
            WORKFLOW_REF=f"{KIT_REPO}-evil/.github/workflows/plugin-ci.yml@refs/heads/main"
        )
        self.assertEqual(code, 1)

    def test_a_sha_that_is_not_a_commit_is_refused(self):
        for not_a_commit in ("abc", A_COMMIT + "0", "z" * 40, "refs/heads/main"):
            code, _, _ = self.resolve(JOB_WORKFLOW_SHA=not_a_commit)
            self.assertEqual(code, 1, not_a_commit)

    def test_a_ref_path_with_no_at_sign_is_refused(self):
        code, _, _ = self.resolve(
            WORKFLOW_REF=f"{KIT_REPO}/.github/workflows/plugin-ci.yml"
        )
        self.assertEqual(code, 1)

    def test_the_other_contexts_are_reported_but_never_used(self):
        # `github.sha` and `github.ref` belong to the *caller's* event, so
        # they say nothing about which kit to take. They are printed because
        # the next failure will be some other context being empty.
        code, said, _ = self.resolve(EVENT_SHA=A_COMMIT, EVENT_REF="refs/heads/main")
        self.assertEqual(code, 1)
        self.assertIn(A_COMMIT, said)

    # ---- what it says ----------------------------------------------------

    def test_every_candidate_is_printed_on_a_run_that_worked(self):
        # ⚠️ Not only on failure. A guard that speaks only when it refuses
        # teaches nothing about the run that worked, and the next time this
        # breaks it will be some other context that is empty.
        _, said, _ = self.resolve(JOB_WORKFLOW_SHA=A_COMMIT)
        for name in (
            "github.job_workflow_sha",
            "github.job_workflow_ref",
            "github.workflow_ref",
            "github.workflow_sha",
            "github.sha",
            "github.ref",
        ):
            self.assertIn(name, said, name)

    def test_an_empty_candidate_is_printed_as_empty_rather_than_as_a_blank(self):
        _, said, _ = self.resolve(JOB_WORKFLOW_SHA=A_COMMIT)
        self.assertIn("<empty>", said)

    def test_it_says_which_candidate_answered(self):
        _, said, _ = self.resolve(JOB_WORKFLOW_SHA=A_COMMIT)
        self.assertIn("from github.job_workflow_sha", said)

        _, said, _ = self.resolve(
            JOB_WORKFLOW_REF=f"{KIT_REPO}/.github/workflows/plugin-ci.yml@refs/tags/0.3.0"
        )
        self.assertIn("from a ref path this kit owns", said)

    def test_the_refusal_names_what_would_have_answered(self):
        _, said, _ = self.resolve()
        self.assertIn(KIT_REPO, said)
        self.assertIn("40-character commit", said)
        # And why it will not guess.
        self.assertIn("wrong kit", said)


if __name__ == "__main__":
    unittest.main(verbosity=2)
