#!/usr/bin/env python3
"""Copy the kit's shell templates into a plugin's ``bin/`` directory.

Two entry points run exactly this module, so the documented one and the tested
one cannot drift apart::

    just sync-bin ../herdr-plugin-recent-spaces
    python3 templates/sync_bin.py ../herdr-plugin-recent-spaces

The second exists because ``just`` is not installed on every machine that has
to be able to do this, and a task nobody can run is a task nobody tests.

**Nothing is substituted.** The files land byte-identical in every plugin, so a
``diff`` between two plugins' ``bin/`` directories shows drift and nothing
else. That is the property this whole arrangement buys: three hand-maintained
copies diverge in silence, one template diverges in a diff somebody has to
read.

Every plugin-specific fact is therefore read by the shims at run time, from the
plugin's own files. There are exactly two, and ``--check`` proves both by
running the synced ``bin/common`` rather than by parsing anything itself. A
second parser here would be a second thing to keep in step with the shell one.

``--check`` writes nothing and exits non-zero on any difference. That is what a
plugin's CI runs, and it is what turns drift into a failing build instead of a
discovery.

Python 3.9 is the floor. This machine has no other interpreter.
"""

import argparse
import filecmp
import shutil
import subprocess
import sys
from pathlib import Path
from typing import List, Tuple

REPO_ROOT = Path(__file__).resolve().parent.parent
TEMPLATE_DIR = REPO_ROOT / "templates" / "bin"

#: Everything a plugin's ``bin/`` gets, in reading order rather than
#: alphabetical: the shared floor, then the two entry points, then the
#: PowerShell mirror of each.
TEMPLATES = (
    "common",
    "find-cargo",
    "progress",
    "build",
    "launcher",
    "common.ps1",
    "build.ps1",
    "launcher.ps1",
)

#: Read by a `sh` that Herdr invokes, so these have to carry the bit.
EXECUTABLE = ("build", "launcher", "find-cargo")


class SyncError(Exception):
    """Something that stops the sync, with a message worth reading."""


def check_target(target: Path) -> None:
    """Refuse anything that is not a Herdr plugin checkout.

    Fails closed. Writing eight shell scripts into a directory that turns out
    to be the wrong one is not something a later step can undo.
    """
    if not target.is_dir():
        raise SyncError(f"{target} is not a directory")
    for required in ("herdr-plugin.toml", "Cargo.toml"):
        if not (target / required).is_file():
            raise SyncError(
                f"{target} has no {required}, so it is not a Herdr plugin checkout"
            )


def differences(target: Path) -> List[str]:
    """Names the templates that are missing from *target* or differ from it."""
    drifted = []
    for name in TEMPLATES:
        landed = target / "bin" / name
        if not landed.is_file():
            drifted.append(f"{name}: missing")
        elif not filecmp.cmp(TEMPLATE_DIR / name, landed, shallow=False):
            drifted.append(f"{name}: differs from the kit's template")
    return drifted


def copy(target: Path) -> None:
    """Writes every template into ``<target>/bin/``, mode included.

    ``copy2`` carries the mode across, so the executable bit is decided once,
    in the kit, rather than re-derived per plugin from a list that could rot.
    """
    destination = target / "bin"
    destination.mkdir(parents=True, exist_ok=True)
    for name in TEMPLATES:
        shutil.copy2(TEMPLATE_DIR / name, destination / name)


def ask_common(target: Path, *expressions: str) -> List[str]:
    """Asks the *synced* ``bin/common`` to evaluate shell expressions, in order.

    🔑 It runs the real shell rather than reading the TOML here. Every fact the
    templates infer has exactly one implementation, and asking it is a check of
    that implementation against this plugin's actual files, never a second
    opinion about them. A second parser on this side would agree with itself
    while disagreeing with the shell, which is the failure it exists to catch.

    Each expression is evaluated inside double quotes, so ``$BINARY`` reads a
    variable and ``$(manifest_version)`` calls a function. One line comes back
    per expression, empty ones included, and the caller decides which emptiness
    is a defect.
    """
    for expression in expressions:
        if '"' in expression:
            raise SyncError(f"a double quote cannot survive this quoting: {expression}")
    fields = " ".join(f'"{expression}"' for expression in expressions)
    script = f'. "$1/bin/common"; printf "%s\\n" {fields}'
    finished = subprocess.run(
        ["sh", "-c", script, "sh", str(target)],
        capture_output=True,
        text=True,
        env={"HERDR_PLUGIN_ROOT": str(target), "PATH": "/usr/bin:/bin:/usr/sbin:/sbin"},
    )
    if finished.returncode != 0:
        raise SyncError(
            "the synced bin/common cannot read this plugin's own files:\n"
            f"{finished.stderr.strip()}"
        )
    answers = finished.stdout.splitlines()
    if len(answers) != len(expressions):
        raise SyncError(
            f"the synced bin/common answered {len(answers)} lines to "
            f"{len(expressions)} questions, so one of them did not run or an "
            "answer carried a newline"
        )
    return answers


def inferred_identity(target: Path) -> Tuple[str, str]:
    """The two facts the templates infer: the binary's name, and the slug.

    A wrong answer here is the failure this design is most exposed to, and it
    is silent at run time: a plugin that infers the wrong binary name fetches
    one asset and executes another.
    """
    binary, slug = ask_common(target, "$BINARY", "$SLUG")
    if not binary or not slug:
        raise SyncError(
            "the synced bin/common answered nothing for the binary name or the "
            "plugin slug, so the shims would not know what to build"
        )
    return binary, slug


def sync(target: Path, check_only: bool) -> int:
    check_target(target)

    if check_only:
        drifted = differences(target)
        if drifted:
            print(f"sync-bin: {target} has drifted from the kit:", file=sys.stderr)
            for line in drifted:
                print(f"  {line}", file=sys.stderr)
            print(
                f"sync-bin: run `python3 templates/sync_bin.py {target}` to fix it",
                file=sys.stderr,
            )
            return 1
        binary, slug = inferred_identity(target)
        print(f"sync-bin: {target} matches the kit ({len(TEMPLATES)} files)")
        print(f"sync-bin: it infers binary={binary} slug={slug}")
        return 0

    copy(target)
    binary, slug = inferred_identity(target)
    print(f"sync-bin: wrote {len(TEMPLATES)} files to {target / 'bin'}")
    print(f"sync-bin: they infer binary={binary} slug={slug}")
    print("sync-bin: review the diff before committing it in that repository")
    return 0


def main(argv: List[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("plugin", help="the plugin checkout to sync into")
    parser.add_argument(
        "--check",
        action="store_true",
        help="report drift and write nothing; exits non-zero when they differ",
    )
    args = parser.parse_args(argv)

    try:
        return sync(Path(args.plugin).resolve(), args.check)
    except SyncError as error:
        print(f"sync-bin: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
