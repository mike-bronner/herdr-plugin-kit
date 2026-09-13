#!/usr/bin/env python3
"""Tests for the conformance gate the reusable workflows run.

    python3 tools/test_plugin_gate.py

🚨 **Every failure this gate exists to catch is silent in production.** Under
download-by-default a plugin whose asset name is wrong by one character still
installs, still works, and simply compiles for sixty to ninety seconds on every
machine that was supposed to download a binary. So a gate that passes for the
wrong reason is worse than no gate: it certifies the thing it never checked.

Each test therefore drives the real readers — the plugin's own synced
``bin/common`` and the real ``cargo metadata`` — against a throwaway checkout,
and the fixture is the same one the shell templates are tested with. The
platform-name tests stub ``uname`` and ask the shim what it would fetch, rather
than comparing two lists this repository wrote.

⚠️ Nothing here runs PowerShell or a Windows runner. The Windows half is pinned
structurally: the producer's constant and the consumer's constant must agree.
Nobody on this project can check more than that.
"""

import json
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "templates"))
sys.path.insert(0, str(REPO_ROOT / "tools"))

from test_templates import LAUNCHD_PATH, Plugin  # noqa: E402

import plugin_gate  # noqa: E402

GATE = REPO_ROOT / "tools" / "plugin_gate.py"


def git(root, *arguments):
    # ⚠️ `tag.gpgSign` is set on the machine this was written on, which turns
    # `git tag <name>` into a signed tag and then fails asking for a message.
    # A fixture that inherits a developer's own git config is a fixture that
    # passes on one machine.
    return subprocess.run(
        ["git", "-C", str(root), "-c", "tag.gpgSign=false", *arguments],
        capture_output=True,
        text=True,
        check=True,
    )


class GateFixture(unittest.TestCase):
    """A throwaway plugin checkout, synced, committed, and tagged on demand."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.plugin = self.make()

    def make(self, **kwargs):
        index = len(list(Path(self.directory.name).glob("plugin*")))
        plugin = Plugin(Path(self.directory.name) / f"plugin-{index}", **kwargs)
        plugin.git_init()
        return plugin

    def tag(self, *names, plugin=None):
        for name in names:
            git((plugin or self.plugin).root, "tag", name)

    def gate(self, *arguments, plugin=None):
        return subprocess.run(
            [sys.executable, str(GATE), "versions", str((plugin or self.plugin).root), *arguments],
            capture_output=True,
            text=True,
        )


class TheMatrixIsOneTable(unittest.TestCase):
    """Six targets, and the runner labels that actually exist."""

    def test_it_carries_the_six_targets_that_ship(self):
        # 🔑 Swept whole rather than spot-checked. An enumeration with no
        # fixture pinning each member degrades one member at a time, and a
        # missing row here is a platform that silently compiles forever.
        self.assertEqual(
            [
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "aarch64-unknown-linux-musl",
                "x86_64-unknown-linux-musl",
                "aarch64-pc-windows-msvc",
                "x86_64-pc-windows-msvc",
            ],
            [row["target"] for row in plugin_gate.matrix()],
        )

    def test_every_arm64_runner_carries_a_versioned_label(self):
        # 🪤 There is no `ubuntu-latest-arm` and no `windows-latest-arm`. A
        # job naming one never starts, and the release publishes five assets.
        arm = [row["runner"] for row in plugin_gate.matrix() if row["target"].startswith("aarch64")]
        self.assertEqual(
            ["macos-latest", "ubuntu-24.04-arm", "windows-11-arm"], sorted(set(arm))
        )
        for runner in arm:
            self.assertNotIn("latest-arm", runner)

    def test_linux_is_musl_so_the_binary_carries_no_glibc_floor(self):
        # A gnu build downloads, passes its checksum, and then refuses to start
        # on an older distribution. That is worse than the 404 this design is
        # built around, because the fetch succeeded.
        linux = [row["target"] for row in plugin_gate.matrix() if "linux" in row["target"]]
        self.assertEqual(2, len(linux))
        for target in linux:
            self.assertTrue(target.endswith("-musl"), target)

    def test_the_subcommand_prints_the_same_table_as_one_line(self):
        # The workflows read this through fromJSON, so a second line would be
        # a parse error at the point six runners are about to start.
        finished = subprocess.run(
            [sys.executable, str(GATE), "matrix"], capture_output=True, text=True, check=True
        )
        self.assertEqual(1, len(finished.stdout.splitlines()))
        self.assertEqual(plugin_gate.matrix(), json.loads(finished.stdout))


class TheFactsTheWorkflowsReadRatherThanCarry(unittest.TestCase):
    """Anything a workflow would otherwise have to repeat in YAML."""

    def test_the_triples_are_the_matrix_and_nothing_else(self):
        finished = subprocess.run(
            [sys.executable, str(GATE), "targets"], capture_output=True, text=True, check=True
        )
        self.assertEqual(
            [row["target"] for row in plugin_gate.matrix()], finished.stdout.split()
        )


class TheWindowsAssetNameIsDecidedInTwoPlacesThatAgree(unittest.TestCase):
    """The producer and the consumer of a name nobody has ever exchanged.

    ⚠️ Structural, and it cannot be anything else. Nobody on this project has
    Windows hardware, so the strongest available claim is that the workflow
    publishes the name the PowerShell shim asks for.
    """

    def _assignment(self):
        text = (REPO_ROOT / "templates" / "bin" / "common.ps1").read_text()
        found = re.search(r'^\s*\$AssetNameExtension\s*=\s*\'([^\']*)\'', text, re.M)
        self.assertIsNotNone(found, "common.ps1 no longer assigns $AssetNameExtension")
        return found.group(1)

    def test_the_producer_and_the_consumer_name_the_same_file(self):
        # 🚨 The whole failure in one line: the release publishes
        # `plugin-windows-x64-abc123.exe`, the shim asks for
        # `plugin-windows-x64-abc123`, GitHub answers 404, and every Windows
        # install compiles instead. Nothing in either place would say so.
        self.assertEqual(self._assignment(), plugin_gate.WINDOWS_ASSET_EXTENSION)

    def test_every_windows_row_carries_it_and_no_other_row_does(self):
        for row in plugin_gate.matrix():
            expected = plugin_gate.WINDOWS_ASSET_EXTENSION if "windows" in row["target"] else ""
            self.assertEqual(expected, row["asset_suffix"], row["target"])

    def test_cargos_own_suffix_is_a_separate_fact_from_the_asset_name(self):
        # 🔑 Cargo writes an .exe on Windows whatever the asset is called, so
        # reversing the recommendation must not silently change where the
        # workflow looks for the binary it just built.
        for row in plugin_gate.matrix():
            expected = ".exe" if "windows" in row["target"] else ""
            self.assertEqual(expected, row["exe_suffix"], row["target"])


class ThePlatformNamesAreTheOnesTheShimAsksFor(unittest.TestCase):
    """Driven through the real shim, not compared against a second list.

    🔑 This is the producer/consumer agreement for the four names that have
    actually shipped. ``bin/common``'s ``platform()`` is what a user's install
    runs; the matrix is what the release publishes. A test comparing two
    literals in this repository would pass while both were wrong together.
    """

    HOSTS = {
        ("Darwin", "arm64"): "macos-arm64",
        ("Darwin", "x86_64"): "macos-x64",
        ("Linux", "aarch64"): "linux-arm64",
        ("Linux", "x86_64"): "linux-x64",
    }

    def test_the_shim_answers_every_unix_name_the_matrix_publishes(self):
        published = {
            row["platform"] for row in plugin_gate.matrix() if "windows" not in row["target"]
        }
        self.assertEqual(4, len(published))
        answered = set()
        with tempfile.TemporaryDirectory() as directory:
            plugin = Plugin(Path(directory) / "plugin")
            for (system, machine), expected in self.HOSTS.items():
                plugin.stub("uname", f'#!/bin/sh\ncase "$1" in\n  -s) printf \'{system}\\n\' ;;\n'
                            f'  -m) printf \'{machine}\\n\' ;;\nesac\n')
                result = plugin.evaluate(
                    'printf %s "$(platform)"', PATH=f"{plugin.stubs}:{LAUNCHD_PATH}"
                )
                self.assertEqual(expected, result.stdout, f"{system} {machine}")
                answered.add(result.stdout)
        self.assertEqual(published, answered)


class TheReleaseNamesWhatTheShimAsksFor(GateFixture):
    """Both sides run, and their answers are compared.

    🚨 This is the failure the whole stage is built to prevent, and it is
    silent in both directions: the release publishes
    ``plugin-macos-arm64-abc123def456`` while the shim asks for something one
    character different, GitHub answers 404, and every install compiles. The
    plugin still works, so nothing surfaces.

    ⚠️ **Both sides are executed, never read.** The producing side is
    ``plugin_gate.py asset-name``, run the way the release workflow runs it,
    and the consuming side is the real shim. A test asserting that a file
    *contains* a string would pass against a line that never runs, and a test
    rebuilding the name itself would agree with itself.

    🔑 **The agreement is two-sided and stays that way**: Python on the
    producing side, shell on the consuming side, neither reading the other.
    ``plugin-release.yml`` calls the producer rather than carrying its own copy
    of the name, which is what keeps this comparison worth running.
    """

    def published_name(self, target):
        return subprocess.run(
            [
                sys.executable,
                str(REPO_ROOT / "tools" / "plugin_gate.py"),
                "asset-name",
                str(self.plugin.root),
                "--target",
                target,
            ],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()

    def requested_name(self, system, machine):
        self.plugin.stub(
            "uname",
            f'#!/bin/sh\ncase "$1" in\n  -s) printf \'{system}\\n\' ;;\n'
            f'  -m) printf \'{machine}\\n\' ;;\nesac\n',
        )
        url = self.plugin.evaluate(
            'asset_url; printf %s "$ASSET_URL"',
            PATH=f"{self.plugin.stubs}:{LAUNCHD_PATH}",
        ).stdout
        self.assertTrue(url, "the shim refused to build a URL at all")
        return url.rsplit("/", 1)[1]

    def test_every_unix_platform_gets_the_file_its_shim_requests(self):
        hosts = {
            "macos-arm64": ("Darwin", "arm64"),
            "macos-x64": ("Darwin", "x86_64"),
            "linux-arm64": ("Linux", "aarch64"),
            "linux-x64": ("Linux", "x86_64"),
        }
        rows = [row for row in plugin_gate.matrix() if "windows" not in row["target"]]
        self.assertEqual(sorted(hosts), sorted(row["platform"] for row in rows))
        for row in rows:
            system, machine = hosts[row["platform"]]
            self.assertEqual(
                self.requested_name(system, machine),
                self.published_name(row["target"]),
                row["platform"],
            )

    def test_the_published_name_carries_the_commit_it_was_built_from(self):
        # 🔑 The URL asserts that the binary came from the source in the folder
        # asking for it. A checkout one commit past the tag asks for a file
        # that does not exist, and compiling is the correct answer.
        head = git(self.plugin.root, "rev-parse", "HEAD").stdout.strip()
        published = self.published_name("aarch64-apple-darwin")
        self.assertTrue(published.endswith(head[:12]), published)
        self.assertNotIn(head[:13], published)

    def test_a_windows_asset_carries_the_recommended_extension(self):
        # The one platform whose name nothing has ever exchanged. Structural on
        # the consuming side, but the producing side really runs here.
        row = next(row for row in plugin_gate.matrix() if row["platform"] == "windows-x64")
        published = self.published_name(row["target"])
        self.assertTrue(published.endswith(plugin_gate.WINDOWS_ASSET_EXTENSION), published)


class EveryKitPinAgrees(GateFixture):
    """One plugin names this kit three times, and nothing compared them.

    🚨 project-finder pins the runtime crate, the build crate, and the `uses:`
    calling the release workflow. A release built by one kit version while the
    crate depends on another publishes assets from a tree the plugin does not
    use, and the only thing that noticed was a test that plugin wrote itself.
    """

    KIT = "https://github.com/mike-bronner/herdr-plugin-kit"

    def a_plugin(self, crate="0.4.1", build="0.4.1", workflow="0.4.1", repository=None):
        """A checkout naming this kit in the three places a plugin does.

        Any argument set to ``None`` leaves that pin out entirely.
        """
        repository = repository or self.KIT
        root = Path(tempfile.mkdtemp(prefix="kit-pins-"))
        self.addCleanup(shutil.rmtree, root, ignore_errors=True)
        (root / "src").mkdir(parents=True)
        (root / "src" / "main.rs").write_text("fn main() {}\n")

        dependencies = ""
        if crate is not None:
            dependencies += f'herdr-plugin-kit = {{ git = "{repository}", {crate} }}\n'
        if build is not None:
            dependencies += f'herdr-plugin-kit-build = {{ git = "{repository}", {build} }}\n'
        (root / "Cargo.toml").write_text(
            '[package]\nname = "a-plugin"\nversion = "0.0.0"\nedition = "2021"\n\n'
            f"[dependencies]\n{dependencies}"
        )

        workflows = root / ".github" / "workflows"
        workflows.mkdir(parents=True)
        if workflow is not None:
            (workflows / "release.yml").write_text(
                "jobs:\n  release:\n    uses: mike-bronner/herdr-plugin-kit"
                f"/.github/workflows/plugin-release.yml@{workflow}\n"
            )
        return root

    def tag(self, version):
        return f'tag = "{version}"'

    # ---- what it accepts -------------------------------------------------

    def test_three_pins_that_agree_are_accepted(self):
        # 🔑 The case every correctly-pinned plugin is in. It must see nothing.
        root = self.a_plugin(self.tag("0.4.1"), self.tag("0.4.1"), "0.4.1")

        self.assertEqual(plugin_gate.check_kit_pins(root, None), [])
        self.assertEqual(plugin_gate.check_kit_pins(root, "0.4.1"), [])

    def test_all_three_pins_are_found(self):
        root = self.a_plugin(self.tag("0.4.1"), self.tag("0.4.1"), "0.4.1")

        found = plugin_gate.kit_pins(root)

        self.assertEqual(sorted(ref for _, ref in found), ["0.4.1"] * 3)
        self.assertEqual(len(found), 3)

    def test_a_fork_whose_name_starts_the_same_is_a_different_kit(self):
        # ⚠️ A prefix test is not a name test. `herdr-plugin-kit-fork` contains
        # this kit's name, and its tags say nothing about this kit's.
        root = self.a_plugin(
            self.tag("9.9.9"), None, None, repository=self.KIT + "-fork"
        )

        self.assertEqual(plugin_gate.kit_pins(root), [])

    # ---- what it refuses -------------------------------------------------

    def test_a_workflow_pinned_apart_from_the_crate_is_refused(self):
        # The hole exactly: the release workflow at one version, the crate at
        # another, and assets built by a kit the plugin does not depend on.
        root = self.a_plugin(self.tag("0.4.1"), self.tag("0.4.1"), "0.4.0")

        problems = plugin_gate.check_kit_pins(root, "0.4.1")

        self.assertEqual(len(problems), 1)
        self.assertIn("release.yml's call to plugin-release.yml", problems[0])
        # Both values, or nobody can act on it.
        self.assertIn("'0.4.0'", problems[0])
        self.assertIn("'0.4.1'", problems[0])

    def test_the_build_crate_pinned_apart_is_refused_too(self):
        # ➕ Wider than the reported hole. The build stamp comes from the kit
        # as much as the runtime crate does.
        root = self.a_plugin(self.tag("0.4.1"), self.tag("0.3.0"), "0.4.1")

        problems = plugin_gate.check_kit_pins(root, "0.4.1")

        self.assertEqual(len(problems), 1)
        self.assertIn("herdr-plugin-kit-build", problems[0])

    def test_pins_that_agree_with_each_other_and_not_with_the_release_are_refused(self):
        # The anchor is what this run actually checked out, so three pins
        # agreeing on the wrong version is still wrong.
        root = self.a_plugin(self.tag("0.4.0"), self.tag("0.4.0"), "0.4.0")

        problems = plugin_gate.check_kit_pins(root, "0.4.1")

        self.assertEqual(len(problems), 3)

    def test_a_pin_naming_no_tag_is_refused(self):
        root = self.a_plugin('branch = "main"', self.tag("0.4.1"), "0.4.1")

        problems = plugin_gate.check_kit_pins(root, "0.4.1")

        self.assertTrue(any("without pinning a tag" in problem for problem in problems))

    #: A `repository:` input, or the owner/repo prefix of a `uses:` path.
    #: Both allow a leading `#`, because a usage comment names it too.
    NAMES_A_REPOSITORY = (
        re.compile(r"^\s*#?\s*repository:\s*(?P<repository>\S+)\s*$"),
        re.compile(
            r"^\s*#?\s*(?:-\s*)?uses:\s*(?P<repository>[^/\s]+/[^/\s]+)/\.github/workflows/"
        ),
    )

    def test_every_mention_of_this_kit_in_the_workflow_is_this_kit(self):
        # 🪤 The comment on KIT_REPOSITORY used to claim a test held these
        # together, and none did — inside the commit that closes a hole made by
        # unchecked duplication. A claim is not a check.
        #
        # ⚠️ A rule rather than a count. Asserting "three" goes stale the moment
        # a fourth is added; asserting that every one equals the constant does
        # not, and it catches the fourth for free.
        workflow = (
            REPO_ROOT / ".github" / "workflows" / "plugin-release.yml"
        ).read_text()

        named = []
        for line in workflow.splitlines():
            for pattern in self.NAMES_A_REPOSITORY:
                matched = pattern.match(line)
                if matched and "herdr-plugin-kit" in matched.group("repository"):
                    named.append(matched.group("repository"))

        self.assertTrue(named, "the workflow names this kit somewhere")
        # Whole value, never a substring: `herdr-plugin-kit-fork` contains it.
        self.assertEqual(set(named), {plugin_gate.KIT_REPOSITORY})

    def test_the_release_guard_actually_runs_this(self):
        # 🪤 Every test above passes while nothing calls the gate. Replacing
        # `python3` with `true` in the workflow left the whole suite green
        # until this existed, which is the shape of a check nobody runs.
        workflow = (
            REPO_ROOT / ".github" / "workflows" / "plugin-release.yml"
        ).read_text()

        self.assertIn("python3 kit/tools/plugin_gate.py kit-pins plugin", workflow)
        # Anchored to the kit this run checked out, not to the release tag,
        # which the step above it uses and which means something else.
        self.assertIn("--tag '${{ steps.kit.outputs.tag }}'", workflow)

    def test_a_plugin_naming_the_kit_nowhere_is_refused(self):
        # Fails closed. A plugin reaching this check called the workflow, so
        # something named the kit; finding nothing means this cannot see it.
        root = self.a_plugin(None, None, None)

        problems = plugin_gate.check_kit_pins(root, "0.4.1")

        self.assertEqual(len(problems), 1)
        self.assertIn("pins", problems[0])


class TheVersionsAgree(GateFixture):
    """The happy path, and the two windows §11.4 says it must leave open."""

    def test_an_agreeing_plugin_passes_and_says_what_it_established(self):
        result = self.gate()
        self.assertEqual(0, result.returncode, result.stderr)
        # A green run that prints nothing is indistinguishable from a green run
        # that checked nothing.
        self.assertIn("decoy-binary 1.2.3", result.stdout)

    def test_a_version_ahead_of_every_tag_passes(self):
        # The ordinary state between a bump landing on main and its tag being
        # pushed. §11.4 names this as the one window the gate cannot close.
        self.tag("1.0.0", "1.1.0")
        self.assertEqual(0, self.gate().returncode)

    def test_a_prefixed_tag_for_an_older_version_passes(self):
        # recent-spaces at migration: its history carries `v` tags that are
        # never rewritten, and its next release is the first unprefixed one.
        self.tag("v1.0.0", "v1.1.0")
        self.assertEqual(0, self.gate().returncode)

    def test_a_tag_that_is_not_a_release_is_passed_over(self):
        self.tag("nightly", "1.2.3-rc1", "release-1.2.3")
        self.assertEqual(0, self.gate().returncode)


class TheVersionsDisagree(GateFixture):
    """Each refusal, and the words that make it actionable."""

    def test_the_two_manifests_stating_different_versions_fails(self):
        plugin = self.make(version="1.2.3", cargo_version="2.0.0")
        result = self.gate(plugin=plugin)
        self.assertEqual(1, result.returncode)
        self.assertIn("herdr-plugin.toml says 1.2.3 and Cargo.toml says 2.0.0", result.stderr)

    def test_the_shim_and_cargo_naming_different_binaries_fails(self):
        # The silent one: the release publishes an asset named for cargo's
        # binary and every install requests the shim's.
        #
        # ⚠️ The two readers are driven apart by replacing bin/common, because
        # they cannot be driven apart by the manifest — they read the same
        # file under compatible rules today. That is the state this branch
        # exists to keep, so the test states it by breaking one reader rather
        # than by hunting for a manifest that splits them.
        (self.plugin.root / "bin" / "common").write_text(
            "#!/bin/sh\nBINARY=other-binary\nmanifest_version() { printf '1.2.3\\n'; }\n"
        )
        result = self.gate()
        self.assertEqual(1, result.returncode)
        self.assertIn("would request the other", result.stderr)

    def test_a_second_auto_discovered_binary_fails_closed(self):
        # 🚨 A plugin that grows src/bin/helper.rs gains a second binary
        # cargo builds and the shim never sees. Guessing between them would
        # fetch the asset for one and execute the other.
        plugin = self.make()
        (plugin.root / "src" / "bin").mkdir()
        (plugin.root / "src" / "bin" / "helper.rs").write_text("fn main() {}\n")
        result = self.gate(plugin=plugin)
        self.assertEqual(1, result.returncode)
        self.assertIn("cargo reports 2 [[bin]] targets", result.stderr)

    def test_a_prefixed_tag_for_the_declared_version_fails(self):
        # 🚨 The two conventions crossing. §12.2: the fetch 404s, the plugin
        # falls back to compiling, and it still works — so nothing surfaces.
        self.tag("v1.2.3")
        result = self.gate()
        self.assertEqual(1, result.returncode)
        self.assertIn("releases/download/1.2.3/", result.stderr)
        self.assertIn("v1.2.3", result.stderr)

    def test_a_release_newer_than_the_declared_version_fails(self):
        self.tag("1.2.3", "1.3.0")
        result = self.gate()
        self.assertEqual(1, result.returncode)
        self.assertIn("1.3.0 is newer than the declared version 1.2.3", result.stderr)

    def test_a_newer_prefixed_release_fails_too(self):
        # Magnitude, not form: a `v` tag above the manifest is the same hole.
        self.tag("v9.9.9")
        self.assertEqual(1, self.gate().returncode)

    def test_no_bin_section_fails_closed(self):
        plugin = self.make(bins=[])
        result = self.gate(plugin=plugin)
        self.assertEqual(1, result.returncode)

    def test_two_bin_sections_fail_closed_rather_than_picking_one(self):
        # The shim refuses first and says so, which is the whole answer: the
        # gate never has to pick, and the message names the file to fix.
        plugin = self.make(bins=["first", "second"])
        result = self.gate(plugin=plugin)
        self.assertEqual(1, result.returncode)
        self.assertIn("no single [[bin]] name", result.stderr)

    def test_a_manifest_with_no_version_fails_closed(self):
        plugin = self.make()
        (plugin.root / "herdr-plugin.toml").write_text('id = "a.b"\n')
        result = self.gate(plugin=plugin)
        self.assertEqual(1, result.returncode)
        self.assertIn("no top-level version", result.stderr)


class TheTagBeingPublished(GateFixture):
    """The release path, where the tag is a fact rather than a search."""

    def test_a_bare_tag_matching_both_manifests_passes(self):
        result = self.gate("--tag", "1.2.3")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("published as 1.2.3", result.stdout)

    def test_a_prefixed_tag_is_refused_rather_than_tolerated(self):
        # ➕ §12.2 requires the release workflow to reject one of the two
        # forms. Tolerating both is exactly how they drift apart.
        result = self.gate("--tag", "v1.2.3")
        self.assertEqual(1, result.returncode)
        self.assertIn("SCOPE.md §12.1", result.stderr)

    def test_a_tag_naming_another_version_is_refused(self):
        result = self.gate("--tag", "1.3.0")
        self.assertEqual(1, result.returncode)
        self.assertIn("no install ever asks for", result.stderr)

    def test_a_tag_that_is_not_a_version_at_all_is_refused(self):
        result = self.gate("--tag", "nightly")
        self.assertEqual(1, result.returncode)
        self.assertIn("not a release tag", result.stderr)

    def test_the_release_path_still_checks_the_two_manifests(self):
        # The tag agreeing with one manifest says nothing about the other.
        plugin = self.make(version="1.2.3", cargo_version="2.0.0")
        result = self.gate("--tag", "1.2.3", plugin=plugin)
        self.assertEqual(1, result.returncode)
        self.assertIn("Cargo.toml says 2.0.0", result.stderr)


class TheGateAsksTheRealReaders(GateFixture):
    """Not a third opinion about the files. SCOPE.md §10.0, one directory over."""

    def test_it_reads_the_version_through_the_plugins_own_bin_common(self):
        # 🔑 The decoys are the point. herdr-plugin.toml carries
        # min_herdr_version beside the version and a [[panes]] version below
        # it, and a reader taking either agrees with itself while disagreeing
        # with the shim that actually builds the URL.
        self.assertIn("1.2.3", self.gate().stdout)
        (self.plugin.root / "bin" / "common").write_text(
            "#!/bin/sh\nBINARY=decoy-binary\nmanifest_version() { printf '9.9.9\\n'; }\n"
        )
        result = self.gate()
        self.assertEqual(1, result.returncode)
        self.assertIn("Cargo.toml says 1.2.3", result.stderr)

    def test_a_plugin_with_no_synced_shims_fails_closed(self):
        for name in ("common", "build"):
            (self.plugin.root / "bin" / name).unlink()
        result = self.gate()
        self.assertEqual(1, result.returncode)
        self.assertIn("bin/common", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=1)
