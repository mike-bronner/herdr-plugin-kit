#!/usr/bin/env python3
"""The conformance checks a plugin's CI and release runs, and the matrix both build.

    python3 tools/plugin_gate.py matrix
    python3 tools/plugin_gate.py versions ../herdr-plugin-project-finder
    python3 tools/plugin_gate.py versions ../herdr-plugin-project-finder --tag 0.8.0
    python3 tools/plugin_gate.py pin-block ../herdr-plugin-project-finder

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
import difflib
import json
import re
import subprocess
import sys
import textwrap
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


#: This kit's own repository, as a consumer names it in a pin.
#:
#: ⚠️ **Named again in `.github/workflows/plugin-release.yml`**, and
#: `tools/test_plugin_gate.py` holds every mention there to this value by whole
#: value rather than by substring, because `herdr-plugin-kit-fork` contains
#: this string and is a different kit.
#:
#: 🪤 That comment claimed the test before the test existed, in the commit that
#: closes a hole made by unchecked duplication. **A claim is not a check**, and
#: it counted the copies wrongly while it was at it. The rule the test asserts
#: names no count, so a fourth copy is covered the day it appears.
KIT_REPOSITORY = "mike-bronner/herdr-plugin-kit"

#: A cargo source naming this kit, and the tag it pins.
#:
#: Anchored at both ends on purpose: a fork whose name merely starts the same
#: is a different kit, and taking its tag would compare two unrelated things.
KIT_SOURCE = re.compile(
    r"^git\+https://github\.com/"
    + re.escape(KIT_REPOSITORY)
    + r"(?:\.git)?(?:\?(?P<query>[^#]*))?(?:#|$)"
)

#: A `uses:` line calling one of this kit's reusable workflows.
KIT_USES = re.compile(
    r"^\s*(?:-\s*)?uses:\s*[\'\"]?"
    + re.escape(KIT_REPOSITORY)
    + r"/\.github/workflows/(?P<file>[^@\'\"]+)@(?P<ref>[^\s\'\"]+)"
)

#: Where GitHub runs a workflow from, and the only place it looks.
WORKFLOW_DIRECTORY = "workflows"


def workflow_files(target: Path) -> List[Path]:
    """Every workflow GitHub would run in *target*, under both extensions."""
    directory = target / ".github" / WORKFLOW_DIRECTORY
    return sorted(directory.glob("*.yml")) + sorted(directory.glob("*.yaml"))


def kit_pins(target: Path) -> List[Tuple[str, str]]:
    """Every pin in *target* naming this kit, as (what named it, its ref).

    🚨 **A plugin names this kit in more than one place and nothing made them
    agree.** project-finder carries three: the runtime crate, the build crate,
    and the `uses:` calling the release workflow. A release built by one kit
    version while the crate pins another publishes assets from a tree the
    plugin does not depend on, and nothing said so until that plugin wrote its
    own test for it.

    A ref of `""` means the pin exists and names no tag.
    """
    found = []
    for package in cargo_metadata(target).get("packages", []):
        for dependency in package.get("dependencies", []):
            matched = KIT_SOURCE.match(dependency.get("source") or "")
            if matched is None:
                continue
            query = matched.group("query") or ""
            tag = ""
            for field in query.split("&"):
                if field.startswith("tag="):
                    tag = field[len("tag="):]
            found.append((f"Cargo.toml's {dependency['name']} dependency", tag))

    for path in workflow_files(target):
        for line in path.read_text(encoding="utf-8").splitlines():
            matched = KIT_USES.match(line)
            if matched is not None:
                found.append(
                    (f"{path.name}'s call to {matched.group('file')}", matched.group("ref"))
                )
    return found


def check_kit_pins(target: Path, tag: Optional[str]) -> List[str]:
    """Every pin naming this kit has to name the same version of it.

    *tag* is what the release workflow resolved and checked the kit out at, so
    it is the anchor when it is given. Without one, the pins only have to agree
    with each other, which is what a human running this by hand wants.
    """
    pins = kit_pins(target)
    if not pins:
        return [
            f"nothing in {target} pins {KIT_REPOSITORY}, so there is no kit "
            f"version to agree about. A plugin reaching this check names it at "
            f"least once, in Cargo.toml or in a workflow's `uses:`."
        ]

    problems = [
        f"{what} names {KIT_REPOSITORY} without pinning a tag, so nothing can "
        f"say which kit it means"
        for what, ref in pins
        if not ref
    ]

    anchor = tag or pins[0][1]
    named_by = "the kit this release checked out" if tag else pins[0][0]
    for what, ref in pins:
        if ref and ref != anchor:
            problems.append(
                f"{what} pins {ref!r} and {named_by} pins {anchor!r}. Every pin "
                f"naming {KIT_REPOSITORY} has to name the same version of it: a "
                f"release built by one kit while the crate depends on another "
                f"publishes assets from a tree this plugin does not use. Change "
                f"whichever is wrong so that all {len(pins)} agree."
            )
    return problems


#: The lines that open and close the kit pin resolution, wherever it is copied.
PIN_BLOCK_OPEN = "# ---8<--- kit pin resolution"
PIN_BLOCK_CLOSE = "# --->8--- end kit pin resolution"

#: The kit's own copy, and the one a plugin's copy is held to. It is the copy
#: that runs, and ``tools/test_kit_pin.py`` holds the two published copies in
#: SCOPE.md and the README to it.
KIT_PIN_CARRIER = REPO_ROOT / ".github" / WORKFLOW_DIRECTORY / "plugin-release.yml"


def pin_blocks(text: str) -> List[str]:
    """Every kit pin resolution block in *text*, each dedented to column zero.

    🪤 **The slice starts at the beginning of the marker's line, not at the
    marker.** Starting at the marker leaves the first line with no indentation,
    so ``textwrap.dedent`` finds a common prefix of nothing and removes nothing.
    The block still *runs*, because leading whitespace is harmless in shell. It
    is not harmless in the Python the block pipes into, which is what surfaced
    it.

    Fails closed on a block that opens and never closes, because the text
    after the marker cannot be told apart from the text that belongs to it.
    """
    blocks = []
    position = 0
    while True:
        marker = text.find(PIN_BLOCK_OPEN, position)
        if marker == -1:
            return blocks
        start = text.rfind("\n", 0, marker) + 1
        close = text.find(PIN_BLOCK_CLOSE, marker)
        if close == -1:
            raise SyncError(
                f"a kit pin resolution block opens and never closes: "
                f"no {PIN_BLOCK_CLOSE!r} follows its first line"
            )
        end = close + len(PIN_BLOCK_CLOSE)
        blocks.append(textwrap.dedent(text[start:end]))
        position = end


def kit_pin_block() -> str:
    """The kit pin resolution as this checkout of the kit publishes it."""
    blocks = pin_blocks(KIT_PIN_CARRIER.read_text(encoding="utf-8"))
    if len(blocks) != 1:
        raise SyncError(
            f"{KIT_PIN_CARRIER.name} carries {len(blocks)} kit pin resolution "
            f"blocks, and exactly one is the kit's own"
        )
    return blocks[0]


def check_pin_block(target: Path, kit_block: str) -> List[str]:
    """Every copy of the kit pin resolution in *target* is the kit's, byte for byte.

    🚨 **A plugin copies this block by hand, and nothing held the copy.**
    recent-spaces' copy drifted between kit 0.4.2 and 0.5.1: its comment lines
    became a paraphrase and its markers went with them, while every executable
    line stayed intact. A human reviewer found it. So the comparison is the
    whole block, comments included, with only the plugin's indentation taken
    off. A check of the executable lines alone would have passed that copy.

    ⚠️ A workflow with no copy at all fails too, because that is what the
    drifted copy looked like once its markers were gone. Every plugin running
    §11.3's recipe carries one.
    """
    copies = [
        (path.name, block)
        for path in workflow_files(target)
        for block in pin_blocks(path.read_text(encoding="utf-8"))
    ]
    if not copies:
        return [
            f"no workflow under {target / '.github' / WORKFLOW_DIRECTORY} carries "
            f"the kit pin resolution between its markers, so nothing says which "
            f"kit it resolves or whether it is the kit's text. Copy the block "
            f"from SCOPE.md §11.3 at the tag this plugin pins, markers included."
        ]
    return [
        f"{name}'s kit pin resolution differs from the kit's. Copy the block "
        f"whole from SCOPE.md §11.3 at the tag this plugin pins, and edit no "
        f"line of it, comments included:\n"
        + "".join(
            difflib.unified_diff(
                kit_block.splitlines(keepends=True),
                block.splitlines(keepends=True),
                "the kit's",
                name,
                n=0,
            )
        )
        for name, block in copies
        if block != kit_block
    ]


def is_windows(target: str) -> bool:
    return "windows" in target


#: How many characters of the commit a release asset's name carries.
#:
#: 🔑 The **producing** half of §9.6's asset convention. The consuming half is
#: ``asset_url`` in ``templates/bin/common``, which is what a shim asks GitHub
#: for at install time, and ``tools/test_plugin_gate.py`` runs both and
#: compares their answers.
#:
#: ⚠️ **One runnable place rather than a line three plugins copy.**
#: ``.github/workflows/plugin-release.yml`` calls ``asset-name`` rather than
#: building the name itself, so a release and a shim cannot disagree about it
#: by drifting apart.
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

    pins = subcommands.add_parser(
        "kit-pins", help="check every pin naming this kit agrees with the others"
    )
    pins.add_argument("plugin", type=Path, help="a plugin checkout")
    pins.add_argument(
        "--tag",
        help="the kit version this run checked out, which every pin must match",
    )

    block = subcommands.add_parser(
        "pin-block",
        help="check every copy of the kit pin resolution matches this kit's",
    )
    block.add_argument("plugin", type=Path, help="a plugin checkout")

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
        if arguments.command == "kit-pins":
            found = kit_pins(arguments.plugin)
            problems = check_kit_pins(arguments.plugin, arguments.tag)
            for problem in problems:
                print(f"plugin-gate: {problem}", file=sys.stderr)
            if problems:
                return 1
            print(
                f"plugin-gate: {len(found)} pins name {KIT_REPOSITORY}, "
                f"all at {found[0][1]}"
            )
            return 0
        if arguments.command == "pin-block":
            problems = check_pin_block(arguments.plugin, kit_pin_block())
            for problem in problems:
                print(f"plugin-gate: {problem}", file=sys.stderr)
            if problems:
                return 1
            print(
                f"plugin-gate: the kit pin resolution in {arguments.plugin} "
                f"matches this kit's"
            )
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
