#!/usr/bin/env python3
"""Tests for the one piece of logic that cannot live in ``tools/``.

🚨 **Everything else in CI lives here for a reason** (SCOPE.md §11.2.1): YAML
cannot be run, so nothing that can be wrong belongs in it. This block is the
forced exception, because it decides **which kit to check out** and therefore
runs before there is a ``tools/`` to call.

It exists in three places, and all three must agree:

* ``.github/workflows/plugin-release.yml``, which runs it.
* ``SCOPE.md`` §11.3's recipe, which a plugin copies into its own CI.
* ``README.md``, the same recipe where a consumer actually reads.

So this module extracts the block from all three, proves they are identical,
and then **runs the real text** under ``sh`` against fabricated ``cargo
metadata`` output.

➕ **A fourth kind of copy lives outside the kit**: every plugin carries one in
its own CI, and nothing held it until ``plugin_gate.py pin-block``. The last
part of this module drives that check against two real copies, taken from
herdr-plugin-recent-spaces and kept under ``tools/fixtures/kit_pin/`` so the
suite never reads another repository:

* ``recent-spaces-899ac27.ci.yml`` carries kit 0.5.1's text exactly.
* ``recent-spaces-963c508.ci.yml`` is the drifted copy a review found by hand:
  its comments paraphrased and its markers gone, every executable line intact.
* ``kit-0.5.1.block`` is kit 0.5.1's own block, taken from its
  ``plugin-release.yml``, so the first fixture can be shown to pass against the
  text it was copied from.

✅ The same arrangement ``tools/test_plugin_gate.py`` uses for the asset name:
execute the shipped text rather than a copy of it, because a copy agrees with
itself while both sides are wrong together.

⚠️ **Why the block reads a file instead of asking the Actions context.**
Measured 2026-09-12: a called workflow is told nothing about which of its own
versions a caller pinned. It does not need to be. ``github.repository`` is the
caller's, so the plugin is already checked out and its ``Cargo.toml`` carries
the pin. §11.2.1 records both the measurement and the too-strong conclusion
first drawn from it.

No network, and no dependency beyond the standard library. Run with::

    python3 tools/test_kit_pin.py

Python 3.9 is the floor. This machine has no other interpreter.
"""

import json
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest import mock
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "tools"))

import plugin_gate  # noqa: E402

GATE = REPO_ROOT / "tools" / "plugin_gate.py"
FIXTURES = REPO_ROOT / "tools" / "fixtures" / "kit_pin"
CARRIERS = (
    REPO_ROOT / ".github" / "workflows" / "plugin-release.yml",
    REPO_ROOT / "SCOPE.md",
    REPO_ROOT / "README.md",
)

#: What a plugin pins, and the only shape the block accepts.
KIT = "herdr-plugin-kit"
REPOSITORY = "https://github.com/mike-bronner/herdr-plugin-kit"


def extract(path):
    """Return the one resolution block in *path*, dedented to column zero.

    🔑 Through the gate's own reader, so the kit's copies and a plugin's copy
    are cut out of their files the same way. Exactly one per carrier: a second
    block in one of them would be a copy this module never compared.
    """
    blocks = plugin_gate.pin_blocks(path.read_text(encoding="utf-8"))
    assert len(blocks) == 1, f"{path.name} carries {len(blocks)} blocks"
    return blocks[0]


class TheThreeCopiesAgree(unittest.TestCase):
    """One is run, two are copied by hand. Nothing else keeps them in step."""

    def test_the_block_is_byte_identical_everywhere_it_appears(self):
        blocks = {path.name: extract(path) for path in CARRIERS}
        first = blocks[CARRIERS[0].name]
        for name, block in blocks.items():
            self.assertEqual(first, block, name)

    def test_each_carrier_checks_out_the_tag_the_block_resolved(self):
        # A copy that resolved the pin and then checked out something else
        # would pass the comparison above and still take the wrong kit.
        for path in CARRIERS:
            self.assertIn(
                "ref: ${{ steps.kit.outputs.tag }}",
                path.read_text(encoding="utf-8"),
                path.name,
            )


class Resolution(unittest.TestCase):
    """The shipped text, run against what cargo actually answers."""

    def resolve(self, source, name=KIT):
        """Run the real block against a fabricated `cargo metadata` answer."""
        metadata = {
            "packages": [
                {
                    "name": "a-plugin",
                    "dependencies": [
                        {"name": "serde", "source": "registry+https://example.invalid"},
                        {"name": name, "source": source},
                    ],
                }
            ]
        }
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            # A `cargo` that answers what the real one answers for this pin.
            cargo = directory / "cargo"
            cargo.write_text(
                "#!/bin/sh\ncat <<'JSON'\n" + json.dumps(metadata) + "\nJSON\n",
                encoding="utf-8",
            )
            cargo.chmod(0o755)
            written = directory / "github_output"
            written.touch()
            finished = subprocess.run(
                ["sh", "-e", "-c", extract(CARRIERS[0])],
                capture_output=True,
                text=True,
                env={
                    "PATH": f"{directory}:/usr/bin:/bin:/usr/sbin:/sbin",
                    "GITHUB_OUTPUT": str(written),
                },
            )
            return (
                finished.returncode,
                finished.stdout + finished.stderr,
                written.read_text(encoding="utf-8").strip(),
            )

    # ---- the one form it accepts -----------------------------------------

    def test_a_tag_pin_answers_the_tag(self):
        code, _, output = self.resolve(f"git+{REPOSITORY}?tag=0.4.0")
        self.assertEqual(code, 0)
        self.assertEqual(output, "tag=0.4.0")

    def test_a_pre_release_tag_survives_its_punctuation(self):
        code, _, output = self.resolve(f"git+{REPOSITORY}?tag=0.4.1-rc.1")
        self.assertEqual(code, 0)
        self.assertEqual(output, "tag=0.4.1-rc.1")

    # ---- the four it refuses ---------------------------------------------

    def test_a_branch_pin_stops_the_job(self):
        # ✅ All four refusals measured against real `cargo metadata` output on
        # 2026-09-12. A branch is a real thing to pin during development, and
        # it has no version to check against.
        code, said, output = self.resolve(f"git+{REPOSITORY}?branch=main")
        self.assertEqual(code, 1)
        self.assertEqual(output, "")
        self.assertIn("does not pin", said)
        # The refusal names what it found, or nobody can act on it.
        self.assertIn("?branch=main", said)

    def test_a_commit_pin_stops_the_job(self):
        code, _, output = self.resolve(f"git+{REPOSITORY}?rev=1aa6479")
        self.assertEqual(code, 1)
        self.assertEqual(output, "")

    def test_a_git_source_with_no_pin_at_all_stops_the_job(self):
        code, _, output = self.resolve(f"git+{REPOSITORY}")
        self.assertEqual(code, 1)
        self.assertEqual(output, "")

    def test_a_path_dependency_stops_the_job(self):
        # cargo reports no source at all for a path dependency, which is what
        # a developer working on both repositories at once has.
        code, said, output = self.resolve(None)
        self.assertEqual(code, 1)
        self.assertEqual(output, "")
        self.assertIn("<none>", said)

    def test_a_plugin_that_does_not_depend_on_the_kit_stops_the_job(self):
        code, _, output = self.resolve(f"git+{REPOSITORY}?tag=0.4.0", name="something-else")
        self.assertEqual(code, 1)
        self.assertEqual(output, "")

    # ---- what it must not confuse ----------------------------------------

    def test_another_dependency_pinned_to_a_tag_is_not_mistaken_for_the_kit(self):
        # 🚨 The block walks every dependency of every package. A plugin with
        # its own tagged git dependency must not have that tag answer for the
        # kit's, which would check out a ref that does not exist here.
        metadata_source = "git+https://github.com/somebody/other?tag=9.9.9"
        code, _, output = self.resolve(metadata_source, name="other")
        self.assertEqual(code, 1, output)


def published_recipe(path):
    """The whole §11.3 recipe out of *path*: the ``ci.yml`` a plugin copies."""
    text = path.read_text(encoding="utf-8")
    found = re.search(r"```yaml\n(# \.github/workflows/ci\.yml in the plugin\n.*?)```", text, re.S)
    assert found, f"{path.name} carries no §11.3 recipe"
    return found.group(1)


class APluginsCopy(unittest.TestCase):
    """``plugin_gate.py pin-block``: the check that holds a plugin's copy."""

    #: Kit 0.5.1's block, the text the passing fixture was copied from.
    KIT_0_5_1 = (FIXTURES / "kit-0.5.1.block").read_text(encoding="utf-8").rstrip("\n")
    #: Carries 0.5.1's block exactly.
    MATCHING = (FIXTURES / "recent-spaces-899ac27.ci.yml").read_text(encoding="utf-8")
    #: The drifted copy: comments paraphrased, markers gone.
    DRIFTED = (FIXTURES / "recent-spaces-963c508.ci.yml").read_text(encoding="utf-8")

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def plugin(self, **workflows):
        """A checkout whose ``.github/workflows`` holds *workflows* by file name."""
        directory = self.root / ".github" / "workflows"
        directory.mkdir(parents=True, exist_ok=True)
        for name, text in workflows.items():
            (directory / name.replace("_", ".")).write_text(text, encoding="utf-8")
        return self.root

    def workflow(self, block, indent=10):
        """A minimal workflow carrying *block* inside a ``run: |`` step."""
        return (
            "jobs:\n  kit-gates:\n    steps:\n      - id: kit\n        run: |\n"
            + textwrap.indent(block, " " * indent)
            + "\n"
        )

    def check(self, root, kit_block):
        return plugin_gate.check_pin_block(root, kit_block)

    # ---- the two recent-spaces commits -----------------------------------

    def test_the_copy_of_0_5_1_passes_against_0_5_1(self):
        self.assertEqual(self.check(self.plugin(ci_yml=self.MATCHING), self.KIT_0_5_1), [])

    def test_the_copy_of_0_5_1_fails_against_this_kit(self):
        # 🔑 The comment was corrected in 0.5.2, so every plugin re-copies the
        # block once when it moves. This is the check that makes it.
        problems = self.check(self.plugin(ci_yml=self.MATCHING), plugin_gate.kit_pin_block())
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("ci.yml's kit pin resolution differs", problems[0])
        # The diff names a line that changed, so a reader knows what to fix.
        self.assertIn("-# ---8<--- kit pin resolution. Copy it whole", problems[0])
        self.assertIn("+# ---8<--- kit pin resolution. This exact text", problems[0])

    def test_the_drifted_copy_fails_against_0_5_1(self):
        problems = self.check(self.plugin(ci_yml=self.DRIFTED), self.KIT_0_5_1)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("no workflow under", problems[0])

    def test_the_drifted_copy_fails_against_this_kit(self):
        problems = self.check(self.plugin(ci_yml=self.DRIFTED), plugin_gate.kit_pin_block())
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("no workflow under", problems[0])

    # ---- what the comparison is ------------------------------------------

    def test_a_paraphrased_comment_fails_with_its_markers_kept(self):
        # 🚨 The drift that was found was in comment lines. A check of the
        # executable lines only would have passed this.
        kit = plugin_gate.kit_pin_block()
        edited = kit.replace("# 🔑 It reads the pin", "# 🔑 This reads the pin", 1)
        self.assertNotEqual(edited, kit)
        problems = self.check(self.plugin(ci_yml=self.workflow(edited)), kit)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("+# 🔑 This reads the pin", problems[0])

    def test_an_edited_executable_line_fails(self):
        kit = plugin_gate.kit_pin_block()
        edited = kit.replace("| head -1)", "| tail -1)", 1)
        self.assertNotEqual(edited, kit)
        self.assertEqual(len(self.check(self.plugin(ci_yml=self.workflow(edited)), kit)), 1)

    def test_the_plugins_indentation_is_not_part_of_the_text(self):
        kit = plugin_gate.kit_pin_block()
        for indent in (6, 10, 14):
            with self.subTest(indent=indent):
                root = self.plugin(ci_yml=self.workflow(kit, indent))
                self.assertEqual(self.check(root, kit), [])

    def test_one_line_indented_apart_from_the_rest_fails(self):
        # Only the indentation the whole block shares is removed. The Python
        # heredoc inside it is indentation-sensitive.
        kit = plugin_gate.kit_pin_block()
        edited = kit.replace("\nimport json, sys\n", "\n  import json, sys\n", 1)
        self.assertNotEqual(edited, kit)
        self.assertEqual(len(self.check(self.plugin(ci_yml=self.workflow(edited)), kit)), 1)

    # ---- every copy, not the first ---------------------------------------

    def test_a_second_drifted_copy_in_the_same_file_fails(self):
        kit = plugin_gate.kit_pin_block()
        drifted = kit.replace("| head -1)", "| tail -1)", 1)
        both = self.workflow(kit) + self.workflow(drifted)
        self.assertEqual(len(self.check(self.plugin(ci_yml=both), kit)), 1)

    def test_a_drifted_copy_in_another_workflow_fails(self):
        kit = plugin_gate.kit_pin_block()
        drifted = kit.replace("| head -1)", "| tail -1)", 1)
        root = self.plugin(ci_yml=self.workflow(kit), release_yml=self.workflow(drifted))
        problems = self.check(root, kit)
        self.assertEqual(len(problems), 1, problems)
        self.assertIn("release.yml's", problems[0])

    def test_a_yaml_extension_is_read_too(self):
        kit = plugin_gate.kit_pin_block()
        drifted = kit.replace("| head -1)", "| tail -1)", 1)
        root = self.plugin(ci_yml=self.workflow(kit), other_yaml=self.workflow(drifted))
        self.assertEqual(len(self.check(root, kit)), 1)

    # ---- failing closed --------------------------------------------------

    def test_a_block_that_never_closes_is_refused(self):
        kit = plugin_gate.kit_pin_block()
        opened = kit[: kit.index(plugin_gate.PIN_BLOCK_CLOSE)]
        with self.assertRaises(plugin_gate.SyncError):
            self.check(self.plugin(ci_yml=self.workflow(opened)), kit)

    def test_a_checkout_with_no_workflows_fails(self):
        self.assertEqual(len(self.check(self.root, plugin_gate.kit_pin_block())), 1)

    # ---- the command a plugin's CI runs ----------------------------------

    def gate(self, root):
        return subprocess.run(
            [sys.executable, str(GATE), "pin-block", str(root)],
            capture_output=True,
            text=True,
        )

    def test_the_published_recipe_passes_its_own_check(self):
        # 🔑 The recipe is what a plugin copies, so it has to pass the check it
        # tells the plugin to run, against the kit that published it.
        for carrier in CARRIERS[1:]:
            with self.subTest(carrier=carrier.name):
                root = self.plugin(ci_yml=published_recipe(carrier))
                finished = self.gate(root)
                self.assertEqual(finished.returncode, 0, finished.stderr)
                self.assertIn("matches this kit's", finished.stdout)

    def test_the_command_fails_on_the_copy_of_0_5_1(self):
        finished = self.gate(self.plugin(ci_yml=self.MATCHING))
        self.assertEqual(finished.returncode, 1, finished.stdout)
        self.assertIn("differs from the kit's", finished.stderr)

    def test_the_command_fails_closed_on_a_block_that_never_closes(self):
        kit = plugin_gate.kit_pin_block()
        opened = kit[: kit.index(plugin_gate.PIN_BLOCK_CLOSE)]
        finished = self.gate(self.plugin(ci_yml=self.workflow(opened)))
        self.assertEqual(finished.returncode, 1, finished.stdout)
        self.assertIn("never closes", finished.stderr)

    def test_the_published_recipe_runs_the_check(self):
        # 🪤 A gate nobody runs passes every test written about it (§11.2.1).
        # The invocation lives in each plugin, so the kit pins the line it
        # tells them to copy.
        for carrier in CARRIERS[1:]:
            with self.subTest(carrier=carrier.name):
                self.assertIn(
                    "      - run: python3 kit/tools/plugin_gate.py pin-block .\n",
                    published_recipe(carrier),
                )

    def test_the_kit_carries_exactly_one_block_of_its_own(self):
        self.assertEqual(plugin_gate.kit_pin_block(), extract(CARRIERS[0]))

    def test_a_kit_carrying_none_or_two_blocks_is_refused(self):
        # Two would leave the check comparing against whichever came first,
        # and none against nothing. Neither is the kit's own text.
        kit = plugin_gate.kit_pin_block()
        for count in (0, 2):
            with self.subTest(blocks=count):
                carrier = self.root / f"carrier-{count}.yml"
                carrier.write_text(self.workflow(kit) * count or "jobs: {}\n", encoding="utf-8")
                with mock.patch.object(plugin_gate, "KIT_PIN_CARRIER", carrier):
                    with self.assertRaises(plugin_gate.SyncError):
                        plugin_gate.kit_pin_block()


if __name__ == "__main__":
    unittest.main(verbosity=2)
