#!/usr/bin/env python3
"""Tests for the shell templates and their sync task.

These templates are the only code in this repository that runs on a user's
machine at install time, on a PATH of ``/usr/bin:/bin:/usr/sbin:/sbin``, with
no terminal and no toolchain guaranteed. Nothing else here can fail in front of
somebody who has not asked to be a developer, so the shims get tested against
real fixture trees and real stub binaries rather than by reading.

Run with::

    python3 templates/test_templates.py

No network, and nothing beyond the standard library. Python 3.9 is the floor.

**What cannot be tested here, and why it is not an oversight.** PowerShell is
not installed on this machine and nobody on this project has Windows hardware,
so ``bin/*.ps1`` is never executed by anything below — not even parsed. The
checks that touch it are structural: that the open ``.exe`` question is decided
in exactly one place, and that the PowerShell mirror still declares a
counterpart for every shell function. Those are the only honest claims
available, and pretending to more would be worse than the gap.
"""

import filecmp
import os
import pty
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from sync_bin import EXECUTABLE, TEMPLATES, SyncError, main, sync  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parent.parent
TEMPLATE_DIR = REPO_ROOT / "templates" / "bin"

#: Herdr's own launchd PATH. Every fixture runs on it, plus a stub directory,
#: because a shim that only works with Homebrew on the PATH does not work.
LAUNCHD_PATH = "/usr/bin:/bin:/usr/sbin:/sbin"

PAYLOAD = b"a published binary"
PAYLOAD_SHA = "5e4dc3a40ca0b0c1dbbef50b0d52e0dba58e3e78b2a40d2cbdeae9ab94e5a5db"


def sha256_of(data: bytes) -> str:
    import hashlib

    return hashlib.sha256(data).hexdigest()


class Plugin:
    """A throwaway plugin checkout with the templates synced into it.

    The manifest carries decoys on purpose. Two of the three real plugins have
    further ``id`` keys further down — ``id = "picker"`` and ``id = "apply"`` —
    and a reader that takes the first or last one it finds is wrong in two of
    three cases while looking entirely plausible.
    """

    def __init__(
        self, directory, binary="decoy-binary", bins=None, version="1.2.3", cargo_version=None
    ):
        self.root = Path(directory)
        self.stubs = self.root.parent / "stubs"
        self.stubs.mkdir(exist_ok=True)
        (self.root / "src").mkdir(parents=True, exist_ok=True)
        (self.root / "src" / "main.rs").write_text("fn main() {}\n")

        self.write_manifest(version=version)
        self.write_cargo(
            binary if bins is None else None,
            bins=bins,
            version=version if cargo_version is None else cargo_version,
        )

        (self.root / "bin").mkdir(exist_ok=True)
        for name in TEMPLATES:
            shutil.copy2(TEMPLATE_DIR / name, self.root / "bin" / name)

        self.binary = binary
        self.bin_path = self.root / "target" / "release" / binary
        self.mark = Path(str(self.bin_path) + ".download")

    # ---- fixture construction -------------------------------------------

    def write_manifest(self, version="1.2.3", plugin_id="mikebronner.decoy-plugin"):
        (self.root / "herdr-plugin.toml").write_text(
            f'id = "{plugin_id}"\n'
            'name = "Decoy Plugin"\n'
            f'version = "{version}"\n'
            'min_herdr_version = "9.9.9"\n'
            "\n"
            "[[panes]]\n"
            'id = "picker"\n'
            'version = "8.8.8"\n'
            'name = "Decoy Pane"\n'
            "\n"
            "[[actions]]\n"
            'id = "apply"\n'
        )

    def write_cargo(self, binary, bins=None, version="1.2.3"):
        text = f'[package]\nname = "decoy-package"\nversion = "{version}"\nedition = "2021"\n\n'
        for name in [binary] if bins is None else bins:
            text += f'[[bin]]\nname = "{name}"\npath = "src/main.rs"\n\n'
        # 🔑 A decoy `name` key after the [[bin]] section, in a table cargo
        # ignores. It sits in [package.metadata] rather than [dependencies] so
        # that the file stays valid TOML *and* valid cargo: tools/plugin_gate.py
        # asks the real cargo what it builds, and cannot ask an invalid one.
        text += '[package.metadata.decoy]\nname = "not-a-binary"\n'
        (self.root / "Cargo.toml").write_text(text)

    def put_binary(self, body="#!/bin/sh\necho hi\n"):
        """Writes the binary and ages the tree so the result is deterministic.

        🪤 Aged rather than raced for. Two files written in the same kernel
        clock tick share an mtime to the nanosecond on Linux, and a binary
        that merely *ties* with its sources is stale by the shim's rule —
        correctly, because nothing can say which came first. A real compile
        takes sixty seconds, so everything else is aged by a minute. That is
        the state cargo actually leaves behind, and it answers the same on
        APFS and on ext4.

        ⚠️ **The binary keeps the mtime the kernel gave it, and that is not an
        omission.** ``time.time()`` reads the fine-grained clock while a file
        timestamp comes from a coarse one that lags it, so stamping the binary
        from Python puts it slightly *ahead* of anything a later
        ``os.utime(path, None)`` can produce — and every "touch a source, now
        it is stale" test then silently inverts. The binary has to stay on the
        same clock as the files it is compared against.
        """
        self.bin_path.parent.mkdir(parents=True, exist_ok=True)
        self.bin_path.write_text(body)
        self.bin_path.chmod(0o755)
        older = time.time() - 60
        for path in self.root.rglob("*"):
            if path != self.bin_path:
                os.utime(path, (older, older))

    def stub(self, name, body, mode=0o755):
        path = self.stubs / name
        path.write_text(body)
        path.chmod(mode)
        return path

    def no_toolchain(self):
        """Replaces bin/find-cargo with one that finds nothing.

        🔑 This is why find-cargo is a file rather than a function. Its
        candidate list holds four absolute paths, and the machine this was
        written on carries /opt/homebrew/opt/rustup/bin/cargo, so no amount of
        PATH control can make the real one fail. Every no-toolchain behaviour
        hangs off this answer.
        """
        (self.root / "bin" / "find-cargo").write_text("#!/bin/sh\nexit 1\n")

    def fake_cargo(self, succeeds=True):
        target = self.root / "target" / "release"
        body = "#!/bin/sh\n"
        if succeeds:
            body += (
                f'mkdir -p "{target}"\n'
                f"printf '#!/bin/sh\\necho compiled\\n' > \"{target}/{self.binary}\"\n"
                f'chmod 755 "{target}/{self.binary}"\n'
                "echo 'fake cargo: compiled' >&2\n"
            )
        else:
            body += "echo 'fake cargo: error[E0001]: it broke' >&2\nexit 1\n"
        cargo = self.stub("cargo-stub", body)
        (self.root / "bin" / "find-cargo").write_text(
            f'#!/bin/sh\nprintf "%s\\n" "{cargo}"\n'
        )

    def fake_curl(self, payload=PAYLOAD, digest=None, exit_code=0):
        """A curl that serves *payload* and a checksum file beside it."""
        if exit_code:
            self.stub("curl", f"#!/bin/sh\nexit {exit_code}\n")
            return
        digest = sha256_of(payload) if digest is None else digest
        self.stub(
            "curl",
            "#!/bin/sh\n"
            "out=''; url=''\n"
            "while [ $# -gt 0 ]; do\n"
            '  case "$1" in\n'
            '    --output) out="$2"; shift 2 ;;\n'
            '    --) shift; url="$1"; shift ;;\n'
            "    *) shift ;;\n"
            "  esac\n"
            "done\n"
            'case "$url" in\n'
            f"  *.sha256) printf '{digest}  asset\\n' > \"$out\" ;;\n"
            f"  *) printf '%s' '{payload.decode()}' > \"$out\" ;;\n"
            "esac\n",
        )

    def fake_herdr(self, kind="local", exit_code=0, pretty=False):
        if exit_code:
            self.stub("herdr", f"#!/bin/sh\nexit {exit_code}\n")
        elif pretty:
            self.stub(
                "herdr",
                '#!/bin/sh\nprintf \'{\\n  "source": {\\n    "kind": "github"\\n  }\\n}\\n\'\n',
            )
        else:
            self.stub(
                "herdr",
                "#!/bin/sh\n"
                f'printf \'{{"plugins":[{{"plugin_id":"x","source":{{"kind":"{kind}"}}}}]}}\\n\'\n',
            )
        return self.stubs / "herdr"

    def git_init(self, remote="https://github.com/mike-bronner/decoy.git"):
        run = lambda *a: subprocess.run(
            ["git", "-C", str(self.root), *a], capture_output=True, check=True
        )
        run("init", "-q")
        run("config", "user.email", "t@example.com")
        run("config", "user.name", "Test")
        if remote:
            run("remote", "add", "origin", remote)
        run("add", "-A")
        run("commit", "-qm", "init")
        return (
            subprocess.run(
                ["git", "-C", str(self.root), "rev-parse", "HEAD"],
                capture_output=True,
                text=True,
                check=True,
            )
            .stdout.strip()[:12]
        )

    # ---- running --------------------------------------------------------

    def environment(self, **extra):
        env = {
            "PATH": f"{self.stubs}:{LAUNCHD_PATH}",
            "HERDR_PLUGIN_ROOT": str(self.root),
            "HOME": str(self.root),
        }
        env.update({key: value for key, value in extra.items() if value is not None})
        return env

    def run(self, script, *args, stderr=None, **env):
        return subprocess.run(
            ["sh", str(self.root / "bin" / script), *args],
            capture_output=stderr is None,
            stderr=stderr,
            stdout=subprocess.PIPE if stderr is not None else None,
            text=True,
            cwd=str(self.root),
            env=self.environment(**env),
        )

    def evaluate(self, snippet, **env):
        """Sources bin/common and runs *snippet* against it."""
        return subprocess.run(
            ["sh", "-c", f'. "$1/bin/common"; {snippet}', "sh", str(self.root)],
            capture_output=True,
            text=True,
            cwd=str(self.root),
            env=self.environment(**env),
        )


class Fixture(unittest.TestCase):
    """Gives every test its own plugin checkout, removed afterwards."""

    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.plugin = Plugin(Path(self.directory.name) / "plugin")

    def make(self, **kwargs):
        root = Path(self.directory.name) / f"plugin-{len(os.listdir(self.directory.name))}"
        return Plugin(root, **kwargs)


class ReadingThePluginsOwnFiles(Fixture):
    """The two facts everything else rests on, and the decoys around them."""

    def test_the_slug_comes_from_the_top_level_id_not_a_pane_entry(self):
        # ⚠️ The failure this pins is silent: `picker` is a perfectly plausible
        # log prefix, and it is wrong in two of the three real plugins.
        self.assertEqual("decoy-plugin", self.plugin.evaluate('printf %s "$SLUG"').stdout)

    def test_the_version_comes_from_the_top_level_table(self):
        # Two decoys at once: min_herdr_version sits beside it in the same
        # table, and a [[panes]] entry carries its own version further down.
        self.assertEqual(
            "1.2.3", self.plugin.evaluate('printf %s "$(manifest_version)"').stdout
        )

    def test_a_later_section_cannot_answer_for_the_top_level_table(self):
        plugin = self.make()
        (plugin.root / "herdr-plugin.toml").write_text(
            'name = "No Id Up Here"\n\n[[panes]]\nid = "picker"\n'
        )
        # No top-level id at all, so the pane's must not be borrowed. The slug
        # degrades to a word, because it only ever names log lines.
        self.assertEqual("herdr-plugin", plugin.evaluate('printf %s "$SLUG"').stdout)

    def test_the_binary_is_the_bin_name_not_the_package_name(self):
        # They agree in two of the three real plugins and differ in the third,
        # which is exactly the shape that survives a careless reading.
        self.assertEqual(
            "decoy-binary", self.plugin.evaluate('printf %s "$BINARY"').stdout
        )

    def test_a_name_key_outside_the_bin_section_is_not_the_binary(self):
        # The fixture's [package.metadata.decoy] table carries
        # name = "not-a-binary", after the [[bin]] section rather than before.
        self.assertNotIn("not-a-binary", self.plugin.evaluate('printf %s "$BINARY"').stdout)

    def test_no_bin_section_at_all_refuses_rather_than_guessing(self):
        plugin = self.make(bins=[])
        result = plugin.evaluate('printf %s "$BINARY"')
        self.assertEqual(1, result.returncode)
        self.assertIn("no single [[bin]] name", result.stderr)

    def test_two_bin_sections_refuse_rather_than_picking_one(self):
        # 🔑 Fails closed. Guessing between two binaries would fetch the asset
        # for one and execute the other.
        plugin = self.make(bins=["first", "second"])
        result = plugin.evaluate('printf %s "$BINARY"')
        self.assertEqual(1, result.returncode)
        self.assertIn("no single [[bin]] name", result.stderr)


class TheTermFix(Fixture):
    """The bug found once and then fixed three times, now fixed in one place."""

    def test_it_will_not_draw_when_stderr_is_a_pipe(self):
        # This is the measured [[startup]] case: no terminal at all.
        self.assertEqual(
            "no", self.plugin.evaluate('can_draw && printf yes || printf no').stdout
        )

    def test_it_draws_on_a_terminal_that_has_a_term(self):
        self.assertEqual("yes", self._on_a_terminal(term="xterm-256color"))

    def test_a_terminal_with_no_term_still_will_not_draw(self):
        # 🔑 The discriminating case. A tty alone is not enough: a Herdr
        # [[startup]] command is handed no TERM, and escape sequences written
        # where no terminfo exists are drawn as themselves.
        self.assertEqual("no", self._on_a_terminal(term=""))

    def test_a_dumb_terminal_will_not_draw(self):
        self.assertEqual("no", self._on_a_terminal(term="dumb"))

    def _on_a_terminal(self, term):
        """Runs can_draw with a real pty on stderr, so `[ -t 2 ]` is true."""
        primary, secondary = pty.openpty()
        try:
            finished = subprocess.run(
                [
                    "sh",
                    "-c",
                    f'. "$1/bin/common"; can_draw && printf yes || printf no',
                    "sh",
                    str(self.plugin.root),
                ],
                stdout=subprocess.PIPE,
                stderr=secondary,
                text=True,
                cwd=str(self.plugin.root),
                env=self.plugin.environment(TERM=term),
            )
        finally:
            os.close(primary)
            os.close(secondary)
        return finished.stdout

    def test_a_build_off_a_terminal_writes_no_escape_sequences(self):
        # The whole consequence of the bug: bytes nobody can read, in the
        # server log, once per server start.
        self.plugin.fake_cargo()
        result = self.plugin.run("build")
        self.assertEqual(0, result.returncode)
        self.assertNotIn("\033", result.stderr)
        self.assertIn("Decoy Plugin is compiling", result.stderr)


class Staleness(Fixture):
    """needs_build, which decides whether anything happens at all."""

    def ask(self, plugin=None):
        plugin = plugin or self.plugin
        return plugin.evaluate('needs_build && printf yes || printf no').stdout

    def build_a_binary(self, plugin=None):
        (plugin or self.plugin).put_binary()

    def test_no_binary_means_build(self):
        self.assertEqual("yes", self.ask())

    def test_a_binary_newer_than_its_sources_is_current(self):
        self.build_a_binary()
        self.assertEqual("no", self.ask())

    def test_a_touched_source_file_makes_it_stale(self):
        self.build_a_binary()
        os.utime(self.plugin.root / "src" / "main.rs", None)
        self.assertEqual("yes", self.ask())

    def test_a_source_exactly_as_old_as_the_binary_is_stale(self):
        # 🪤 The defect this class missed for two stages, and the reason the
        # rule reads "not older" rather than "newer". Linux caches the wall
        # clock per timer tick, so a binary and a source written in the same
        # tick get a byte-identical mtime — measured 2026-09-11 as
        # 1789159317.075108009 for both. A strictly-newer comparison reads
        # that as current and runs a stale binary, saying nothing.
        #
        # macOS gave the same pair of writes timestamps 2.2ms apart, which is
        # why every test here passed on this machine for two stages. The tie
        # is therefore *set* rather than raced for, so this discriminates on
        # APFS as well as on ext4.
        self.build_a_binary()
        # ⚠️ Nanoseconds, not st_mtime. A Unix timestamp at nanosecond
        # resolution needs more significant digits than a float carries, so
        # reading and writing it as one quietly lands a few nanoseconds *under*
        # the binary and tests the wrong state. That is the same precision trap
        # this whole defect is about, one layer up.
        stamp = os.stat(self.plugin.bin_path).st_mtime_ns
        os.utime(self.plugin.root / "src" / "main.rs", ns=(stamp, stamp))
        self.assertEqual("yes", self.ask())

    def test_a_cargo_manifest_exactly_as_old_as_the_binary_is_stale(self):
        # The same rule against the other shape of input. A plain file and a
        # directory are separate walks, and an enumeration with no fixture
        # pinning each member degrades one member at a time.
        self.build_a_binary()
        stamp = os.stat(self.plugin.bin_path).st_mtime_ns
        os.utime(self.plugin.root / "Cargo.toml", ns=(stamp, stamp))
        self.assertEqual("yes", self.ask())

    def test_a_source_a_moment_older_than_the_binary_is_current(self):
        # The other side of the rule, so "not older" cannot be satisfied by
        # answering stale to everything. One second, because the margin the
        # rule needs is any margin at all.
        self.build_a_binary()
        stamp = os.stat(self.plugin.bin_path).st_mtime_ns - 1_000_000_000
        os.utime(self.plugin.root / "src" / "main.rs", ns=(stamp, stamp))
        self.assertEqual("no", self.ask())

    def test_a_touched_rust_toolchain_toml_makes_it_stale(self):
        # ⚠️ rust-toolchain and rust-toolchain.toml are both in the list and
        # neither is redundant: git pathspecs match whole path components, so
        # one does not match the other. This pins the .toml half.
        self.build_a_binary()
        (self.plugin.root / "rust-toolchain.toml").write_text('[toolchain]\n')
        self.assertEqual("yes", self.ask())

    def test_a_touched_readme_does_not(self):
        # An edited README cannot change the binary and must not force a
        # sixty-second compile.
        self.build_a_binary()
        (self.plugin.root / "README.md").write_text("# later\n")
        self.assertEqual("no", self.ask())

    def test_a_downloaded_binary_is_judged_by_version_not_by_mtime(self):
        # 🔑 The case mtimes get wrong. On any commit past a release the source
        # is permanently newer, so an mtime check says "rebuild" forever — on
        # the one machine with no toolchain to rebuild with.
        self.build_a_binary()
        self.plugin.mark.write_text("version=1.2.3\nasset=a\n")
        os.utime(self.plugin.root / "src" / "main.rs", None)
        self.assertEqual("no", self.ask())

    def test_a_downloaded_binary_from_an_older_version_is_stale(self):
        self.build_a_binary()
        self.plugin.mark.write_text("version=1.0.0\nasset=a\n")
        self.assertEqual("yes", self.ask())

    def test_an_unreadable_manifest_version_fails_closed_and_builds(self):
        self.build_a_binary()
        self.plugin.mark.write_text("version=1.2.3\n")
        (self.plugin.root / "herdr-plugin.toml").write_text('id = "a.b"\n')
        self.assertEqual("yes", self.ask())


class Provenance(Fixture):
    """How the binary arrived, which the version report reads back."""

    def setUp(self):
        super().setUp()
        self.commit = self.plugin.git_init()
        self.plugin.fake_curl()
        self.plugin.fake_cargo()

    def test_a_fetch_writes_the_note_beside_the_binary(self):
        result = self.plugin.run("build", "--install")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue(self.plugin.mark.is_file())
        note = dict(
            line.split("=", 1)
            for line in self.plugin.mark.read_text().splitlines()
            if "=" in line
        )
        self.assertEqual("1.2.3", note["version"])
        self.assertEqual(f"decoy-binary-{self._platform()}-{self.commit}", note["asset"])
        self.assertEqual(sha256_of(PAYLOAD), note["sha256"])
        self.assertTrue(note["url"].endswith(note["asset"]))

    def test_the_note_carries_every_key_the_rust_side_reads(self):
        # 🔑 A cross-language contract with no compiler to check it. version.rs
        # names the keys it reads; this asserts the shell writes each one, so
        # renaming either side reddens here instead of at a user's install.
        self.plugin.run("build", "--install")
        written = {
            line.split("=", 1)[0]
            for line in self.plugin.mark.read_text().splitlines()
            if "=" in line
        }
        source = (
            REPO_ROOT / "crates" / "herdr-plugin-kit" / "src" / "version.rs"
        ).read_text()
        for key in re.findall(r'value\("(\w+)"\)', source):
            self.assertIn(key, written, f"version.rs reads {key} and the shim never writes it")
        self.assertIn("value(\"asset\")", source)
        suffix = re.search(r'PROVENANCE_SUFFIX: &str = "([^"]+)"', source).group(1)
        self.assertEqual(str(self.plugin.bin_path) + suffix, str(self.plugin.mark))

    def test_compiling_removes_a_note_left_by_an_earlier_fetch(self):
        # 🔑 The other half of the pair. A compiled binary keeping a stale note
        # reports itself as downloaded, and then tells somebody holding a stale
        # binary to reinstall rather than rebuild.
        self.plugin.mark.parent.mkdir(parents=True, exist_ok=True)
        self.plugin.mark.write_text("version=0.0.1\nasset=old\n")
        (self.plugin.root / "BUILD_FROM_SOURCE").touch()
        result = self.plugin.run("build", "--install")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(self.plugin.mark.exists())

    def test_a_failed_fetch_leaves_no_note_at_all(self):
        # No note means "built from source", and that has to stay true.
        self.plugin.fake_curl(exit_code=22)
        self.plugin.run("build", "--install")
        self.assertFalse(self.plugin.mark.exists())

    def _platform(self):
        machine = os.uname().machine
        arch = "arm64" if machine in ("arm64", "aarch64") else "x64"
        system = "macos" if os.uname().sysname == "Darwin" else "linux"
        return f"{system}-{arch}"


class TheAssetUrlIsPinnedWhole(Fixture):
    """➕ SCOPE.md §12.2 requires this test, and requires this shape of it.

    🚨 A test that built the URL the same way the shim builds it could not
    catch a wrong tag form, because it would make the same mistake twice and
    agree with itself. So the expected string is stated, in full, with the tag
    form visible in it.

    A wrong tag form here is silent: the fetch 404s, the plugin compiles, and
    it still works. It just stops using the prebuilt binary the whole
    mechanism exists to deliver.
    """

    def setUp(self):
        super().setUp()
        # Stubbed so the whole string can be a literal. The host's own
        # platform would put a variable back into the one assertion whose
        # value is being pinned.
        self.plugin.stub(
            "uname",
            "#!/bin/sh\ncase \"$1\" in\n  -s) printf 'Darwin\\n' ;;\n"
            "  -m) printf 'arm64\\n' ;;\nesac\n",
        )
        self.commit = self.plugin.git_init()

    def ask(self):
        return self.plugin.evaluate(
            'asset_url; printf %s "$ASSET_URL"',
            PATH=f"{self.plugin.stubs}:{LAUNCHD_PATH}",
        ).stdout

    def test_it_is_this_exact_string(self):
        self.assertEqual(
            "https://github.com/mike-bronner/decoy/releases/download/1.2.3/"
            f"decoy-binary-macos-arm64-{self.commit}",
            self.ask(),
        )

    def test_the_tag_carries_no_v_however_the_version_is_written(self):
        # ⚠️ §12.1: Mike's tags are the bare version and Herdr's keep their
        # prefix. recent-spaces' own bin/build hardcoded `v$version` and will
        # break on the first unprefixed tag it cuts. The template prefixes
        # nothing, so a migrated plugin inherits the right convention by
        # construction, and this is what says so.
        self.assertIn("/releases/download/1.2.3/", self.ask())
        self.assertNotIn("/download/v", self.ask())


class TheFetchGate(Fixture):
    """The move is the gate: nothing unverified reaches the executed path."""

    def setUp(self):
        super().setUp()
        self.plugin.git_init()
        self.plugin.fake_cargo()

    def test_a_verified_asset_lands_and_is_executable(self):
        self.plugin.fake_curl()
        result = self.plugin.run("build", "--install")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(PAYLOAD, self.plugin.bin_path.read_bytes())
        self.assertTrue(os.access(self.plugin.bin_path, os.X_OK))

    def test_a_checksum_mismatch_never_reaches_the_executed_path(self):
        # 🚨 The security property. A mismatch still falls back to compiling,
        # because building from the cloned source is safe — but the bad file
        # must never occupy the path the launcher execs.
        self.plugin.fake_curl(digest="0" * 64)
        self.plugin.no_toolchain()
        result = self.plugin.run("build", "--install")
        self.assertEqual(1, result.returncode)
        self.assertFalse(self.plugin.bin_path.exists())
        # ⚠️ Not just the exec path: the payload must not survive anywhere
        # under target/, or a later run finds it and the gate has only moved.
        survivors = [
            path
            for path in (self.plugin.root / "target").rglob("*")
            if path.is_file() and path.read_bytes() == PAYLOAD
        ]
        self.assertEqual([], survivors)

    def test_a_checksum_mismatch_is_loud(self):
        self.plugin.fake_curl(digest="0" * 64)
        self.plugin.no_toolchain()
        stderr = self.plugin.run("build", "--install").stderr
        self.assertIn("SECURITY", stderr)
        self.assertIn("expected: " + "0" * 64, stderr)
        self.assertIn("actual:   " + sha256_of(PAYLOAD), stderr)

    def test_a_checksum_mismatch_caches_nothing(self):
        # ⚠️ The bad artifact must never be reused, so no fetch directory may
        # survive for a second attempt to find.
        self.plugin.fake_curl(digest="0" * 64)
        self.plugin.no_toolchain()
        self.plugin.run("build", "--install")
        leftovers = list((self.plugin.root / "target").glob("fetch.*"))
        self.assertEqual([], leftovers)

    def test_a_checksum_that_is_not_a_digest_is_refused(self):
        self.plugin.fake_curl(digest="not-a-digest")
        self.plugin.no_toolchain()
        result = self.plugin.run("build", "--install")
        self.assertIn("not a sha256 digest", result.stderr)
        self.assertFalse(self.plugin.bin_path.exists())

    def test_a_short_digest_is_refused(self):
        # Hex, but 63 characters. The length check is separate from the
        # character check and needs its own fixture.
        self.plugin.fake_curl(digest="a" * 63)
        self.plugin.no_toolchain()
        result = self.plugin.run("build", "--install")
        self.assertIn("not a sha256 digest", result.stderr)
        self.assertFalse(self.plugin.bin_path.exists())

    def test_an_http_error_falls_back_and_says_where(self):
        self.plugin.fake_curl(exit_code=22)
        result = self.plugin.run("build", "--install")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("nothing is published at", result.stderr)

    def test_an_unreachable_network_is_reported_differently(self):
        # A release-process bug and a train tunnel are different problems and
        # collapsing them costs whoever reads the log the diagnosis.
        self.plugin.fake_curl(exit_code=7)
        result = self.plugin.run("build", "--install")
        self.assertIn("could not be reached", result.stderr)

    def test_the_state_directory_records_the_outcome(self):
        state = self.plugin.root / "state"
        self.plugin.fake_curl(exit_code=22)
        self.plugin.run("build", "--install", HERDR_PLUGIN_STATE_DIR=str(state))
        recorded = (state / "last-fetch").read_text()
        self.assertIn("outcome=http-error", recorded)
        self.assertIn("url=https://github.com/", recorded)

    def test_a_dirty_compiler_path_refuses_to_fetch_and_says_so(self):
        # The URL asserts the binary was built from the source in this folder,
        # and a dirty tree cannot make that claim.
        (self.plugin.root / "src" / "main.rs").write_text("fn main() { /* edited */ }\n")
        self.plugin.fake_curl()
        result = self.plugin.run("build", "--install")
        self.assertIn("not a clean checkout", result.stderr)
        self.assertFalse(self.plugin.mark.exists())

    def test_a_dirty_readme_does_not_stop_a_fetch(self):
        # 🔑 The narrowing that makes the whole thing usable. git status would
        # call this tree dirty; the compiler does not care.
        (self.plugin.root / "README.md").write_text("# edited after the commit\n")
        self.plugin.fake_curl()
        result = self.plugin.run("build", "--install")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue(self.plugin.mark.is_file())

    def test_no_github_origin_means_no_fetch(self):
        plugin = self.make()
        plugin.git_init(remote="git@gitlab.com:someone/else.git")
        plugin.fake_curl()
        plugin.fake_cargo()
        result = plugin.run("build", "--install")
        self.assertIn("no GitHub origin", result.stderr)


class WhichWayRound(Fixture):
    """SCOPE section 8.3: fetch, or compile, and who decides."""

    def setUp(self):
        super().setUp()
        self.plugin.git_init()
        self.plugin.fake_curl()
        self.plugin.fake_cargo()

    def fetched(self):
        return self.plugin.mark.is_file()

    def test_the_install_flag_fetches_without_asking_the_socket(self):
        # [[build]] fires only on `herdr plugin install`, never on a link, so
        # the install is github by construction there. No socket call at all.
        herdr = self.plugin.fake_herdr(kind="local")
        result = self.plugin.run("build", "--install", HERDR_BIN_PATH=str(herdr))
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue(self.fetched())

    def test_a_local_install_compiles(self):
        herdr = self.plugin.fake_herdr(kind="local")
        result = self.plugin.run("build", HERDR_BIN_PATH=str(herdr))
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(self.fetched())
        self.assertIn("local install", result.stderr)

    def test_a_github_install_fetches(self):
        herdr = self.plugin.fake_herdr(kind="github")
        result = self.plugin.run("build", HERDR_BIN_PATH=str(herdr))
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue(self.fetched())

    def test_no_herdr_binary_fails_closed_to_compiling(self):
        result = self.plugin.run("build")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertFalse(self.fetched())

    def test_a_failing_herdr_call_fails_closed_to_compiling(self):
        herdr = self.plugin.fake_herdr(exit_code=1)
        self.plugin.run("build", HERDR_BIN_PATH=str(herdr))
        self.assertFalse(self.fetched())

    def test_an_unrecognised_response_shape_fails_closed_to_compiling(self):
        # ⚠️ Pretty-printed JSON says github and does not match. That is the
        # safe direction, and pinning it is what makes the substring match
        # honest rather than lucky.
        herdr = self.plugin.fake_herdr(pretty=True)
        self.plugin.run("build", HERDR_BIN_PATH=str(herdr))
        self.assertFalse(self.fetched())

    def test_the_override_file_beats_a_github_install(self):
        # The developer escape hatch, and it has to win over everything.
        herdr = self.plugin.fake_herdr(kind="github")
        (self.plugin.root / "BUILD_FROM_SOURCE").touch()
        result = self.plugin.run("build", HERDR_BIN_PATH=str(herdr))
        self.assertFalse(self.fetched())
        self.assertIn("BUILD_FROM_SOURCE", result.stderr)

    def test_the_override_file_beats_the_install_flag(self):
        # 🔑 The install context is the one place fetching is unconditional, so
        # this is the case the override exists for.
        (self.plugin.root / "BUILD_FROM_SOURCE").touch()
        result = self.plugin.run("build", "--install")
        self.assertFalse(self.fetched())

    def test_an_unknown_argument_is_refused(self):
        result = self.plugin.run("build", "--prefer-download")
        self.assertEqual(1, result.returncode)
        self.assertIn("unknown argument", result.stderr)
        self.assertFalse(self.plugin.bin_path.exists())

    def test_a_current_binary_costs_no_network_and_no_git(self):
        # Step one of the order, and the reason it is step one: this runs at
        # every server start.
        self.plugin.fake_cargo()
        self.plugin.run("build", "--install")
        self.plugin.stub("curl", "#!/bin/sh\necho 'curl must not run' >&2\nexit 99\n")
        self.plugin.stub("git", "#!/bin/sh\necho 'git must not run' >&2\nexit 99\n")
        result = self.plugin.run("build")
        self.assertEqual(0, result.returncode)
        self.assertEqual("", result.stderr)


class NoToolchain(Fixture):
    """What a machine with no Rust installed is told."""

    def test_it_names_the_remedy_rather_than_failing_silently(self):
        self.plugin.no_toolchain()
        result = self.plugin.run("build")
        self.assertEqual(1, result.returncode)
        self.assertIn("no Rust toolchain here", result.stderr)
        self.assertIn("rustup.rs", result.stderr)

    def test_a_failed_compile_shows_cargo_s_own_output(self):
        self.plugin.fake_cargo(succeeds=False)
        result = self.plugin.run("build")
        self.assertEqual(1, result.returncode)
        self.assertIn("error[E0001]", result.stderr)


class TheLauncher(Fixture):
    """bin/launcher, which is what a manifest entry actually points at."""

    def setUp(self):
        super().setUp()
        self.plugin.fake_cargo()

    def install_a_binary(self, body="#!/bin/sh\necho ran \"$@\"\n"):
        self.plugin.put_binary(body)

    def test_it_execs_the_binary_and_forwards_arguments(self):
        self.install_a_binary()
        result = self.plugin.run("launcher", "--from-event", "x")
        self.assertEqual("ran --from-event x\n", result.stdout)

    def test_it_builds_first_when_the_binary_is_missing(self):
        result = self.plugin.run("launcher")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual("compiled\n", result.stdout)

    def test_version_does_not_rebuild_first(self):
        # 🔑 --version exists to diagnose a binary that disagrees with its
        # manifest. A rebuild running first would replace that binary with a
        # current one, so the command could never show the state it is for.
        self.install_a_binary('#!/bin/sh\necho "version report"\n')
        os.utime(self.plugin.root / "src" / "main.rs", None)
        self.plugin.stub("cargo-stub", "#!/bin/sh\necho 'must not compile' >&2\nexit 9\n")
        result = self.plugin.run("launcher", "--version")
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual("version report\n", result.stdout)

    def test_version_without_a_binary_says_so_rather_than_building(self):
        result = self.plugin.run("launcher", "--version")
        self.assertEqual(1, result.returncode)
        self.assertIn("no decoy-binary binary", result.stderr)

    def test_a_failed_build_still_runs_a_stale_binary_and_says_so(self):
        self.install_a_binary('#!/bin/sh\necho stale\n')
        os.utime(self.plugin.root / "src" / "main.rs", None)
        self.plugin.fake_cargo(succeeds=False)
        result = self.plugin.run("launcher")
        self.assertEqual("stale\n", result.stdout)
        self.assertIn("may be stale", result.stderr)

    def test_a_failed_build_with_no_binary_at_all_fails(self):
        self.plugin.fake_cargo(succeeds=False)
        result = self.plugin.run("launcher")
        self.assertEqual(1, result.returncode)
        self.assertIn("could not be built", result.stderr)

    def test_it_sends_no_toast(self):
        # A shell shim firing a notification and discarding the answer is what
        # SCOPE section 7.1 was retracted over, and section 13 holds the report
        # module until a real consumer proves the boundary.
        text = (TEMPLATE_DIR / "launcher").read_text()
        self.assertNotIn("notification", text)


class WindowsIsNamedInExactlyOnePlace(unittest.TestCase):
    """The .exe question, pinned on the consuming side so it cannot spread.

    ⚠️ Nothing in this class runs PowerShell. It cannot: PowerShell is not
    installed here and nobody on this project has Windows hardware. These are
    structural assertions about where a decision lives, which is the one thing
    that can be checked without a Windows machine.

    The producing side lives in ``tools/plugin_gate.py``, and
    ``tools/test_plugin_gate.py`` asserts the two agree. That suite tests the
    pair because it already depends on this one for its fixture, and the
    reverse dependency would be a cycle.
    """

    def test_the_asset_extension_is_assigned_exactly_once_in_the_tree(self):
        # 🔑 Reversing the recommendation is one line on this side and one on
        # the producing side. That is only true while there is one line here.
        assignments = []
        for path in sorted(REPO_ROOT.glob("**/*")):
            if not path.is_file() or ".git/" in str(path) or "target/" in str(path):
                continue
            try:
                text = path.read_text()
            except (UnicodeDecodeError, PermissionError):
                continue
            for line in text.splitlines():
                if re.match(r'^\s*\$AssetNameExtension\s*=', line):
                    assignments.append(f"{path}: {line.strip()}")
        self.assertEqual(1, len(assignments), f"expected one assignment, found {assignments}")

    def test_it_is_marked_unverified_where_it_is_assigned(self):
        # A guess that does not say it is a guess is indistinguishable from a
        # measurement, which is the failure this whole convention exists over.
        text = (TEMPLATE_DIR / "common.ps1").read_text()
        declaration = text.split("$AssetNameExtension")[0]
        self.assertIn("UNVERIFIED", declaration)
        self.assertIn("14.2", declaration)

    def test_no_shell_template_ever_names_a_windows_asset(self):
        # The claim the assertion above depends on. `sh` cannot run on Windows
        # at all, so a windows- asset name in a shell template would be both
        # dead and a second place to correct.
        for name in TEMPLATES:
            if name.endswith(".ps1"):
                continue
            text = (TEMPLATE_DIR / name).read_text()
            self.assertNotIn("windows-arm64", text, name)
            self.assertNotIn("windows-x64", text, name)

    def test_the_shell_platform_refuses_an_unknown_host_rather_than_guessing(self):
        # Behaviour, not grep: a wrong platform name downloads a binary built
        # for another machine and then runs it.
        with tempfile.TemporaryDirectory() as directory:
            stubs = Path(directory) / "stubs"
            stubs.mkdir()
            uname = stubs / "uname"
            uname.write_text("#!/bin/sh\nprintf 'Windows_NT\\n'\n")
            uname.chmod(0o755)
            plugin = Plugin(Path(directory) / "plugin")
            result = plugin.evaluate(
                'platform && printf "answered %s" "$(platform)" || printf refused',
                PATH=f"{stubs}:{LAUNCHD_PATH}",
            )
            self.assertEqual("refused", result.stdout)


class ShellAndPowerShellAgree(unittest.TestCase):
    """The PowerShell mirror still mirrors.

    ⚠️ This is a parity check and nothing more. It cannot show that the
    PowerShell side *works*, because nothing here can run it. What it can show
    is that a function added to bin/common was not silently left out of
    bin/common.ps1, which is how an unverifiable file rots.
    """

    #: Swept as a whole rather than spot-checked: an enumeration with no
    #: fixture pinning each member degrades one member at a time.
    COUNTERPARTS = {
        "toml_top_level": "Get-TomlTopLevel",
        "binary_name": "Get-BinaryName",
        "plugin_slug": "Get-PluginSlug",
        "manifest_version": "Get-ManifestVersion",
        "plugin_id": "Get-PluginId",
        "note": "Write-Note",
        "die": "Stop-WithNote",
        "can_draw": "Test-CanDraw",
        "compiler_input_not_older_than": "Test-CompilerInputNotOlderThan",
        "note_value": "Get-NoteValue",
        "needs_build": "Test-NeedsBuild",
        "platform": "Get-Platform",
        "repo_slug": "Get-RepoSlug",
        "released_commit": "Get-ReleasedCommit",
        "asset_url": "Get-AssetUrl",
    }

    def setUp(self):
        self.shell = (TEMPLATE_DIR / "common").read_text()
        self.powershell = (TEMPLATE_DIR / "common.ps1").read_text()

    def test_every_shell_function_has_a_powershell_counterpart(self):
        for shell_name, powershell_name in self.COUNTERPARTS.items():
            self.assertRegex(
                self.shell, rf"(?m)^{re.escape(shell_name)}\(\)", shell_name
            )
            self.assertRegex(
                self.powershell, rf"function {re.escape(powershell_name)}\b", powershell_name
            )

    def test_the_table_above_covers_every_function_bin_common_declares(self):
        # 🔑 Without this, adding a function to bin/common and forgetting the
        # PowerShell side passes: the table would simply not mention it.
        declared = set(re.findall(r"^([a-z_][a-z0-9_]*)\(\)", self.shell, re.MULTILINE))
        self.assertEqual(set(), declared - set(self.COUNTERPARTS))

    def test_both_sides_carry_the_same_compiler_input_list(self):
        # The list three questions ask. Letting the two platforms differ makes
        # them disagree about whether one tree is releasable.
        shell = re.search(r"COMPILER_INPUTS='([^']+)'", self.shell).group(1).split()
        powershell = re.findall(r"'([^']+)'", re.search(
            r"\$CompilerInputs = @\((.*?)\)", self.powershell, re.S
        ).group(1))
        self.assertEqual(shell, powershell)

    def test_both_sides_use_the_same_provenance_suffix(self):
        self.assertIn("PROVENANCE_SUFFIX='.download'", self.shell)
        self.assertIn("$ProvenanceSuffix = '.download'", self.powershell)

    def test_every_powershell_file_says_it_has_never_been_run(self):
        # The README holds this line for the Rust code. New work must not
        # quietly widen the claim, and PowerShell does not reach even the
        # compile-verified bar because there is no compiler.
        for name in TEMPLATES:
            if not name.endswith(".ps1"):
                continue
            text = (TEMPLATE_DIR / name).read_text()
            self.assertIn("HAS EVER BEEN RUN", text, name)


class TheProgressSeam(Fixture):
    """One implementation point, with the terminal as one implementation.

    ⚠️ The distinction being pinned here is narrow and load-bearing. The
    terminal spinner is *selected by name*, not fallen back to. A Herdr dialog
    is coming for every path that can reach one, and the install compile — the
    longest wait these plugins impose — is measured to reach none, so the drawn
    path stays permanently rather than until the dialog lands.
    """

    def setUp(self):
        super().setUp()
        self.text = (TEMPLATE_DIR / "progress").read_text()

    def test_only_bin_progress_draws(self):
        # 🔑 What makes adding the dialog a contained change. Escape sequences
        # anywhere else would be a second place to find and change.
        for name in TEMPLATES:
            if name in ("progress",) or name.endswith(".ps1"):
                continue
            text = (TEMPLATE_DIR / name).read_text()
            self.assertNotIn("\\033[", text, f"{name} draws, and only bin/progress may")

    def test_the_build_talks_to_the_display_through_two_calls_only(self):
        text = (TEMPLATE_DIR / "build").read_text()
        used = set(
            re.findall(
                r"\b(progress_\w+|terminal_\w+|silent_\w+|paint|spin|layout|pane_size)\b",
                text,
            )
        )
        self.assertEqual({"progress_start", "progress_stop"}, used)

    def test_the_terminal_is_a_named_choice_and_not_the_default(self):
        # 🔑 The shape the ruling asked for. `terminal` has to be something
        # progress_backend returns and progress_start dispatches on, so the
        # dialog is a sibling arm rather than a rewrite of the entry point.
        self.assertRegex(self.text, r"progress_backend\(\) \{")
        self.assertRegex(self.text, r'PROGRESS_BACKEND="\$\(progress_backend\)"')
        self.assertRegex(self.text, r"terminal\) terminal_start ;;")
        self.assertRegex(self.text, r"terminal\) terminal_stop ;;")

    def test_no_display_at_all_is_an_implementation_rather_than_a_failure(self):
        # A [[startup]] run with nothing watching is not a degraded case, and
        # naming it stops the next reader treating it as one.
        self.assertRegex(self.text, r"silent_start\(\)")
        self.assertRegex(self.text, r"silent_stop\(\)")

    def test_the_backend_is_chosen_by_can_draw(self):
        selector = self.text.split("progress_backend() {", 1)[1].split("\n}", 1)[0]
        self.assertIn("can_draw", selector)

    def test_the_seam_records_why_the_install_path_keeps_drawing(self):
        # The measurement is the justification for a second code path. Without
        # it recorded here, the next reader deletes the spinner as redundant
        # the moment the dialog lands.
        self.assertIn("plugin_not_found", self.text)
        self.assertIn("zero HERDR_* variables", self.text)
        self.assertIn("dialog", self.text)

    def test_a_run_with_a_terminal_draws_and_a_run_without_one_does_not(self):
        # Behaviour, not grep, for both arms of the selector.
        self.plugin.fake_cargo()
        piped = self.plugin.run("build")
        self.assertNotIn("\033", piped.stderr)
        self.assertIn("is compiling", piped.stderr)

        primary, secondary = pty.openpty()
        try:
            self.plugin.bin_path.unlink()
            subprocess.run(
                ["sh", str(self.plugin.root / "bin" / "build")],
                stdout=subprocess.PIPE,
                stderr=secondary,
                cwd=str(self.plugin.root),
                env=self.plugin.environment(TERM="xterm-256color"),
            )
            os.set_blocking(primary, False)
            try:
                drawn = os.read(primary, 65536)
            except BlockingIOError:
                drawn = b""
        finally:
            os.close(primary)
            os.close(secondary)
        self.assertIn(b"\033[", drawn)


class Syncing(Fixture):
    """The task that puts these files into a plugin, without `just`."""

    def test_it_lands_every_template_byte_identical(self):
        # 🔑 The whole point of substituting nothing: a diff between two
        # plugins' bin/ directories is drift and nothing else.
        plugin = self.make()
        shutil.rmtree(plugin.root / "bin")
        self.assertEqual(0, sync(plugin.root, check_only=False))
        for name in TEMPLATES:
            self.assertTrue(
                filecmp.cmp(TEMPLATE_DIR / name, plugin.root / "bin" / name, shallow=False),
                name,
            )

    def test_the_entry_points_land_executable(self):
        plugin = self.make()
        shutil.rmtree(plugin.root / "bin")
        sync(plugin.root, check_only=False)
        for name in EXECUTABLE:
            mode = (plugin.root / "bin" / name).stat().st_mode
            self.assertTrue(mode & stat.S_IXUSR, name)

    def test_check_passes_on_a_freshly_synced_plugin(self):
        self.assertEqual(0, sync(self.plugin.root, check_only=True))

    def test_check_notices_an_edited_shim(self):
        # This is what a plugin's CI runs, and it is what turns drift into a
        # failing build rather than a discovery.
        (self.plugin.root / "bin" / "build").write_text("#!/bin/sh\n# edited locally\n")
        self.assertEqual(1, sync(self.plugin.root, check_only=True))

    def test_check_notices_a_missing_shim(self):
        (self.plugin.root / "bin" / "progress").unlink()
        self.assertEqual(1, sync(self.plugin.root, check_only=True))

    def test_check_writes_nothing(self):
        (self.plugin.root / "bin" / "build").write_text("#!/bin/sh\n# edited locally\n")
        sync(self.plugin.root, check_only=True)
        self.assertIn("edited locally", (self.plugin.root / "bin" / "build").read_text())

    def test_it_refuses_a_directory_that_is_not_a_plugin(self):
        # Fails closed. Writing eight shell scripts into the wrong directory is
        # not something a later step can undo.
        stray = Path(self.directory.name) / "not-a-plugin"
        stray.mkdir()
        with self.assertRaises(SyncError) as caught:
            sync(stray, check_only=False)
        self.assertIn("herdr-plugin.toml", str(caught.exception))

    def test_it_refuses_a_plugin_with_no_cargo_manifest(self):
        plugin = self.make()
        (plugin.root / "Cargo.toml").unlink()
        with self.assertRaises(SyncError) as caught:
            sync(plugin.root, check_only=False)
        self.assertIn("Cargo.toml", str(caught.exception))

    def test_it_reports_what_the_synced_shims_infer(self):
        # 🔑 It runs the synced bin/common rather than parsing the TOML itself.
        # A second parser here would be a second thing to keep in step, and it
        # would agree with itself while disagreeing with the shell.
        plugin = self.make()
        result = subprocess.run(
            [sys.executable, str(REPO_ROOT / "templates" / "sync_bin.py"), str(plugin.root)],
            capture_output=True,
            text=True,
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("binary=decoy-binary", result.stdout)
        self.assertIn("slug=decoy-plugin", result.stdout)

    def test_a_plugin_whose_binary_cannot_be_inferred_is_refused(self):
        plugin = self.make(bins=["first", "second"])
        with self.assertRaises(SyncError) as caught:
            sync(plugin.root, check_only=False)
        self.assertIn("cannot read this plugin's own files", str(caught.exception))

    def test_the_command_line_reports_a_refusal_rather_than_raising(self):
        stray = Path(self.directory.name) / "also-not-a-plugin"
        stray.mkdir()
        self.assertEqual(1, main([str(stray)]))


if __name__ == "__main__":
    unittest.main(verbosity=2)
