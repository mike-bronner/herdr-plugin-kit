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

One thing here is **not** a copy, and it is the only file this task touches that
the plugin owns: the root ``.gitignore`` gains an entry for the developer
override, appended when absent and never rewritten. Without it a developer who
uses the override leaves an untracked marker in the plugin root, and one
careless ``git add .`` commits it — after which every install of that release
compiles from source and nothing says so above a line in a server log.

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

#: The developer override the shims look for in the plugin root. It has to
#: agree with ``OVERRIDE_FILE`` in ``templates/bin/common``, which is what
#: reads it. See SCOPE.md section 9.4.2.
OVERRIDE_FILE = "BUILD_FROM_SOURCE"

#: What a plugin's root ``.gitignore`` has to carry, verbatim.
#:
#: 🚨 **Committing the override turns every install of that release into a
#: source build**, and says so only in a line of a server log during an install
#: nobody is watching. The plugin still works, which is why nothing complains.
#: §12.2 and §11.4 exist for the same failure shape.
IGNORE_BLOCK = (
    "# The kit's developer override: its presence forces a source build.\n"
    f"/{OVERRIDE_FILE}\n"
)

#: The file that carries it. ⚠️ The plugin **root**, not ``bin/``: the marker is
#: meant to be findable in a directory listing, which is the whole reason
#: ``bin/common`` made it a file rather than an environment variable.
IGNORE_FILE = ".gitignore"


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


def ignores_the_override(target: Path) -> bool:
    """Whether the plugin's root ``.gitignore`` already ignores the override.

    Either anchoring counts, because git ignores the root file under both. A
    plugin that already handles it its own way is left alone rather than given
    a second entry saying the same thing.
    """
    path = target / IGNORE_FILE
    if not path.is_file():
        return False
    wanted = {OVERRIDE_FILE, "/" + OVERRIDE_FILE}
    return any(line.strip() in wanted for line in path.read_text(encoding="utf-8").splitlines())


def ignore_the_override(target: Path) -> bool:
    """Append the entry when it is absent, and report whether it wrote.

    🔑 **Append only.** This is the one file the sync touches that the plugin
    owns and the kit does not, so nothing already in it is rewritten, reordered,
    or reformatted — not even a missing trailing newline, which is completed
    rather than corrected. A second run adds nothing.
    """
    if ignores_the_override(target):
        return False

    path = target / IGNORE_FILE
    existing = path.read_text(encoding="utf-8") if path.is_file() else ""
    if not existing:
        text = IGNORE_BLOCK
    elif existing.endswith("\n\n"):
        text = existing + IGNORE_BLOCK
    elif existing.endswith("\n"):
        text = existing + "\n" + IGNORE_BLOCK
    else:
        text = existing + "\n\n" + IGNORE_BLOCK
    path.write_text(text, encoding="utf-8")
    return True


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
        if not ignores_the_override(target):
            drifted.append(
                f"{IGNORE_FILE}: no /{OVERRIDE_FILE} entry, so a developer's "
                f"override can be committed by accident"
            )
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
        print(f"sync-bin: its {IGNORE_FILE} carries the {OVERRIDE_FILE} entry")
        print(f"sync-bin: it infers binary={binary} slug={slug}")
        return 0

    copy(target)
    wrote_ignore = ignore_the_override(target)
    binary, slug = inferred_identity(target)
    print(f"sync-bin: wrote {len(TEMPLATES)} files to {target / 'bin'}")
    print(
        f"sync-bin: {IGNORE_FILE} "
        + ("gained the" if wrote_ignore else "already carried the")
        + f" {OVERRIDE_FILE} entry"
    )
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
