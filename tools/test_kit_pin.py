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
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
CARRIERS = (
    REPO_ROOT / ".github" / "workflows" / "plugin-release.yml",
    REPO_ROOT / "SCOPE.md",
    REPO_ROOT / "README.md",
)

OPEN_MARKER = "# ---8<--- kit pin resolution"
CLOSE_MARKER = "# --->8--- end kit pin resolution"

#: What a plugin pins, and the only shape the block accepts.
KIT = "herdr-plugin-kit"
REPOSITORY = "https://github.com/mike-bronner/herdr-plugin-kit"


def extract(path):
    """Return the resolution block from *path*, dedented to column zero.

    🪤 **The slice starts at the beginning of the marker's line, not at the
    marker.** Starting at the marker leaves the first line with no indentation,
    so `textwrap.dedent` finds a common prefix of nothing and removes nothing —
    and the block still *runs*, because leading whitespace is harmless in
    shell. It is not harmless in the Python this block pipes into, which is
    what surfaced it.
    """
    text = path.read_text(encoding="utf-8")
    marker = text.index(OPEN_MARKER)
    start = text.rfind("\n", 0, marker) + 1
    end = text.index(CLOSE_MARKER, marker) + len(CLOSE_MARKER)
    return textwrap.dedent(text[start:end])


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


if __name__ == "__main__":
    unittest.main(verbosity=2)
