#!/usr/bin/env python3
"""The conformance checks a plugin's CI and release runs, and the matrix both build.

    python3 tools/plugin_gate.py matrix
    python3 tools/plugin_gate.py versions ../herdr-plugin-project-finder
    python3 tools/plugin_gate.py versions ../herdr-plugin-project-finder --tag 0.8.0

🔑 **This is here rather than inline in the workflows because YAML cannot be
run.** Two reusable workflows need the same six targets and the same version
rules; a copy in each would be two things to keep in step, and neither copy
could be tested until a release was already going wrong. The workflows are
wrappers around this module, the same arrangement ``codegen/`` and
``templates/`` already have with the justfile.

What ``versions`` asserts, and why each one is load-bearing
===========================================================

Under download-by-default a plugin's ``bin/build`` fetches a published binary
and only compiles when it cannot, so every one of these failures is silent:
the install still works, it just stops using the prebuilt binary the whole
mechanism exists to deliver. SCOPE.md §11.4 and §12.2.

1. **The shim and cargo agree on the binary's name.** The shim builds the
   asset name from ``Cargo.toml``'s ``[[bin]]`` entry with its own reader. If
   that answer and cargo's differ, the release publishes one name and every
   install requests another.
2. **``herdr-plugin.toml`` and ``Cargo.toml`` state the same version.** The
   shim reads the release tag out of the plugin manifest; cargo builds what
   the crate manifest says.
3. **No release tag carries a ``v``** for the version being declared. §12.1:
   Mike's tags are the bare semantic version and Herdr's keep their prefix,
   and a gate tolerating both forms cannot catch them crossing.
4. **No release exists above the declared version.** A manifest left behind a
   release means every install at that release asks for a tag that names an
   older version. A manifest *ahead* of its tags is the ordinary state between
   a version bump and its release, and passes.

⚠️ Nothing here parses TOML. The shim's answers come from running the plugin's
own synced ``bin/common``, and cargo's come from ``cargo metadata``. Both are
the readers that actually run at install time and at build time, so this
compares two live implementations rather than adding a third opinion about the
files. SCOPE.md §10.0 records that rule; this is the same rule applied one
directory over.

Python 3.9 is the floor, as it is for every other script here.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Dict, List, Optional, Tuple

REPO_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO_ROOT / "templates"))

from sync_bin import SyncError, ask_common  # noqa: E402

#: Mike's own release tags are the bare semantic version, from project-finder
#: 0.8.0 onward. Herdr's own tags keep their ``v`` and are a different
#: convention about a different repository; nothing here ever checks one.
#:
#: 🚨 One form, deliberately. Accepting both is how two conventions drift
#: apart and then disagree without saying so. SCOPE.md §12.1 and §12.2.
TAG_PREFIX = ""

#: A release tag, either convention, so that the wrong one can be *named*
#: rather than ignored. Anything else — a pre-release suffix, a moving
#: pointer, a name — is not a release of a declared version and is passed over.
RELEASE_TAG = re.compile(r"^(?P<prefix>v?)(?P<version>[0-9]+\.[0-9]+\.[0-9]+)$")

#: The six targets that ship, and the runner each is built on natively.
#: SCOPE.md §11.6. There is no ``ubuntu-latest-arm`` alias: arm64 runners carry
#: versioned labels only, so both Linux rows name a version.
#:
#: 🔑 Linux is **musl**, not gnu, and that is what project-finder 0.8.0 already
#: publishes. A gnu binary carries the runner's glibc floor, and a user on an
#: older distribution gets a download that passes its checksum and then refuses
#: to start. That failure is worse than the 404 this design is built around,
#: because the fetch succeeded.
TARGETS = (
    {"target": "aarch64-apple-darwin", "runner": "macos-latest", "platform": "macos-arm64"},
    {"target": "x86_64-apple-darwin", "runner": "macos-latest", "platform": "macos-x64"},
    {
        "target": "aarch64-unknown-linux-musl",
        "runner": "ubuntu-24.04-arm",
        "platform": "linux-arm64",
    },
    {"target": "x86_64-unknown-linux-musl", "runner": "ubuntu-24.04", "platform": "linux-x64"},
    {"target": "aarch64-pc-windows-msvc", "runner": "windows-11-arm", "platform": "windows-arm64"},
    {"target": "x86_64-pc-windows-msvc", "runner": "windows-latest", "platform": "windows-x64"},
)

# ---- THE ONE UNCONFIRMED WINDOWS ANSWER ----------------------------------

#: Does a Windows release asset's name carry ``.exe``?
#:
#: RECOMMENDED, NOT CONFIRMED. Nobody on this project has Windows hardware,
#: and nothing has ever produced or consumed a Windows asset. SCOPE.md §14.2
#: holds the question and now records the recommendation: a file without this
#: extension is not executable on Windows, and somebody downloading from the
#: releases page should get something that runs.
#:
#: 🔑 This is the **producing** half of the answer. The consuming half is
#: ``$AssetNameExtension`` in ``templates/bin/common.ps1``, and those two are
#: the only places in this repository that decide it. Reversing the
#: recommendation is one line here and one line there.
#: ``tools/test_plugin_gate.py`` fails if they ever disagree, because a
#: producer and a consumer disagreeing about this name is a 404 and a silent
#: compile on every Windows install.
#:
#: ⚠️ Being wrong here costs that 404, never a wrong binary: the checksum gate
#: in ``bin/build`` does not care what the file was called.
WINDOWS_ASSET_EXTENSION = ".exe"

#: Cargo's own fact, and a different one. Cargo always writes ``.exe`` on
#: Windows whatever the asset ends up being called, so reversing the
#: recommendation above must not touch this.
CARGO_EXE_SUFFIX = ".exe"


def is_windows(target: str) -> bool:
    return "windows" in target


#: How many characters of the commit a release asset's name carries.
#:
#: 🔑 The **producing** half of §9.6's asset convention. The consuming half is
#: ``asset_url`` in ``templates/bin/common``, which is what a shim asks GitHub
#: for at install time, and ``tools/test_plugin_gate.py`` runs both and
#: compares their answers.
#:
#: ⚠️ It lived in ``.github/workflows/plugin-release.yml`` until 2026-09-12,
#: when the kit stopped running CI for other repositories (§11). A plugin's
#: own release job calls ``asset-name`` here instead, so the convention stays
#: in one runnable place rather than becoming a line three plugins copy.
COMMIT_CHARACTERS = 12


def asset_name(binary: str, target: str, commit: str) -> str:
    """The file a release publishes, and the file a shim asks for.

    🚨 A producer and a consumer disagreeing about this name is silent in both
    directions: GitHub answers 404, ``bin/build`` falls back to compiling, and
    the plugin still works. Nobody finds out.
    """
    row = next((row for row in TARGETS if row["target"] == target), None)
    if row is None:
        raise SyncError(
            f"{target!r} is not one of the six targets this kit publishes. "
            f"Run `plugin_gate.py targets` for the list."
        )
    suffix = WINDOWS_ASSET_EXTENSION if is_windows(target) else ""
    return f"{binary}-{row['platform']}-{commit[:COMMIT_CHARACTERS]}{suffix}"


def matrix() -> List[Dict[str, str]]:
    """The build matrix, as the rows a workflow's ``matrix.include`` takes.

    ``exe_suffix`` names what cargo wrote. ``asset_suffix`` names what the
    asset is called. They agree today and are separate on purpose.
    """
    rows = []
    for row in TARGETS:
        windows = is_windows(row["target"])
        rows.append(
            dict(
                row,
                exe_suffix=CARGO_EXE_SUFFIX if windows else "",
                asset_suffix=WINDOWS_ASSET_EXTENSION if windows else "",
            )
        )
    return rows


def cargo_metadata(target: Path) -> dict:
    finished = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=str(target),
        capture_output=True,
        text=True,
    )
    if finished.returncode != 0:
        raise SyncError(f"cargo could not read this plugin's manifest:\n{finished.stderr.strip()}")
    return json.loads(finished.stdout)


def cargo_facts(target: Path) -> Tuple[str, str]:
    """Cargo's own answer for the version and the binary's name.

    ⚠️ Exactly one ``[[bin]]`` across the whole workspace, and zero or several
    fails closed. ``bin/common`` applies the same rule for the same reason:
    guessing between two binaries would fetch the asset for one and execute
    the other.
    """
    packages = cargo_metadata(target)["packages"]
    binaries = [
        (package["version"], entry["name"])
        for package in packages
        for entry in package["targets"]
        if "bin" in entry["kind"]
    ]
    if len(binaries) != 1:
        names = ", ".join(sorted(name for _, name in binaries)) or "none"
        raise SyncError(
            f"cargo reports {len(binaries)} [[bin]] targets ({names}); exactly "
            "one is required, because the shims fetch an asset named for one "
            "binary and execute another"
        )
    return binaries[0]


def head_commit(target: Path) -> str:
    """The commit a plugin checkout is sitting on.

    Fails closed like every other reader here: a name built from an unknown
    commit would be published and then never requested.
    """
    finished = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=str(target),
        capture_output=True,
        text=True,
    )
    if finished.returncode != 0:
        raise SyncError(
            f"git could not read this plugin's HEAD:\n{finished.stderr.strip()}"
        )
    return finished.stdout.strip()


def release_tags(target: Path) -> List[Tuple[str, str, str, Tuple[int, ...]]]:
    """Every tag that names a release, as (raw, prefix, version, sortable)."""
    finished = subprocess.run(
        ["git", "tag", "--list"],
        cwd=str(target),
        capture_output=True,
        text=True,
    )
    if finished.returncode != 0:
        raise SyncError(f"git could not list this plugin's tags:\n{finished.stderr.strip()}")
    found = []
    for line in finished.stdout.splitlines():
        matched = RELEASE_TAG.match(line.strip())
        if matched:
            version = matched.group("version")
            parts = tuple(int(part) for part in version.split("."))
            found.append((line.strip(), matched.group("prefix"), version, parts))
    return found


def check_versions(target: Path, tag: Optional[str]) -> Tuple[List[str], str]:
    """Every disagreement found, in reading order, and what was established.

    An empty list means agreement. The second half is printed on a pass, so
    that a green run states the facts it checked rather than only its silence.
    """
    problems = []

    declared, binary = ask_common(target, "$(manifest_version)", "$BINARY")
    cargo_version, cargo_binary = cargo_facts(target)

    if not declared:
        problems.append(
            "herdr-plugin.toml declares no top-level version that bin/common "
            "can read, so bin/build could not name a release at all"
        )
    if binary != cargo_binary:
        problems.append(
            f"bin/common infers the binary is {binary!r} and cargo builds "
            f"{cargo_binary!r}; the release would publish one name and every "
            "install would request the other"
        )
    if declared and declared != cargo_version:
        problems.append(
            f"herdr-plugin.toml says {declared} and Cargo.toml says "
            f"{cargo_version}; the shims read the first and cargo builds the "
            "second"
        )

    summary = f"{binary} {declared or '(no version)'}"

    if not declared:
        return problems, summary

    if not RELEASE_TAG.match(declared):
        problems.append(
            f"herdr-plugin.toml's version {declared!r} is not a bare "
            "MAJOR.MINOR.PATCH, so no release tag can match it"
        )
        return problems, summary

    if tag is not None:
        problems.extend(check_tag(tag, declared))
        return problems, f"{binary} {declared}, published as {tag}"

    problems.extend(check_history(target, declared))
    return problems, summary


def check_tag(tag: str, declared: str) -> List[str]:
    """The release path: this tag is being published right now."""
    matched = RELEASE_TAG.match(tag)
    if not matched:
        return [f"the tag {tag!r} is not a release tag: it is not a bare MAJOR.MINOR.PATCH"]
    if matched.group("prefix") != TAG_PREFIX:
        return [
            f"the tag {tag!r} carries a {matched.group('prefix')!r} prefix and "
            f"this project's tags carry {TAG_PREFIX!r} (SCOPE.md §12.1). "
            f"bin/build would request releases/download/{declared}/… and get a "
            "404, then compile without saying why"
        ]
    if matched.group("version") != declared:
        return [
            f"the tag is {tag} and the manifests declare {declared}; the "
            "assets would be published under a tag no install ever asks for"
        ]
    return []


def check_history(target: Path, declared: str) -> List[str]:
    """The CI path: no release has been published that this version misses.

    ⚠️ A version *ahead* of every tag is the ordinary state between a bump
    landing and its tag being pushed, and passes. That window is the one thing
    this gate cannot close; SCOPE.md §11.4 says to push the bump and the tag
    together.
    """
    problems = []
    tags = release_tags(target)
    declared_parts = tuple(int(part) for part in declared.split("."))

    crossed = [
        raw for raw, prefix, version, _ in tags if prefix != TAG_PREFIX and version == declared
    ]
    if crossed:
        problems.append(
            f"the tag {crossed[0]} carries a prefix this project's tags do not, "
            f"and names the version the manifests declare; every install would "
            f"request releases/download/{declared}/… and get a 404, then "
            "compile without saying why (SCOPE.md §12.2)"
        )

    ahead = [raw for raw, _, _, parts in tags if parts > declared_parts]
    if ahead:
        newest = max(tags, key=lambda item: item[3])[0]
        problems.append(
            f"the release {newest} is newer than the declared version "
            f"{declared}; an install at that release reads {declared} out of "
            "herdr-plugin.toml, requests a release that is not the one it is "
            "checked out at, and compiles"
        )
    return problems


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    subcommands = parser.add_subparsers(dest="command", required=True)

    subcommands.add_parser("matrix", help="print the build matrix as one line of JSON")
    subcommands.add_parser("targets", help="print the six target triples, one per line")

    versions = subcommands.add_parser("versions", help="check a plugin's versions agree")
    versions.add_argument("plugin", type=Path, help="a plugin checkout")
    versions.add_argument(
        "--tag",
        help="the tag being released, which the manifests and the tag form must match",
    )

    facts = subcommands.add_parser("binary-name", help="print the binary cargo builds")
    facts.add_argument("plugin", type=Path, help="a plugin checkout")

    asset = subcommands.add_parser("asset-name", help="print the asset a release publishes")
    asset.add_argument("plugin", type=Path, help="a plugin checkout")
    asset.add_argument("--target", required=True, help="one of the six target triples")
    asset.add_argument(
        "--commit",
        help="the commit being published, defaulting to the plugin's own HEAD",
    )

    arguments = parser.parse_args(argv)

    try:
        if arguments.command == "matrix":
            print(json.dumps(matrix(), separators=(",", ":")))
            return 0
        if arguments.command == "targets":
            print("\n".join(row["target"] for row in TARGETS))
            return 0
        if arguments.command == "binary-name":
            print(cargo_facts(arguments.plugin)[1])
            return 0
        if arguments.command == "asset-name":
            commit = arguments.commit or head_commit(arguments.plugin)
            print(asset_name(cargo_facts(arguments.plugin)[1], arguments.target, commit))
            return 0
        problems, summary = check_versions(arguments.plugin, arguments.tag)
    except SyncError as error:
        print(f"plugin-gate: {error}", file=sys.stderr)
        return 1

    for problem in problems:
        print(f"plugin-gate: {problem}", file=sys.stderr)
    if problems:
        return 1

    print(f"plugin-gate: {summary}, and every manifest agrees")
    return 0


if __name__ == "__main__":
    sys.exit(main())
