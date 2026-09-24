#!/usr/bin/env python3
"""Mutation testing: break one thing on purpose, and check the suite notices.

    python3 tools/mutate.py tools/mutations/dialog.json

A test only counts if it fails when the behaviour it guards regresses. This
applies each mutation in the spec, runs the suite, and reports whether the
suite caught it. Anything it could not catch is a test that asserts trivia.

Why this exists as a tool rather than a shell one-liner
=======================================================

🚨 **Two harnesses written inline for this project misclassified a compile
outcome, and both failed silently in the flattering direction.**

1. The first treated cargo's ``error: test failed`` as a build failure, so five
   real kills were reported as invalid mutations.
2. The second matched ``error[``, the form carrying a diagnostic code. A plain
   syntax error prints ``error:`` with no code, so a mutation that did not
   compile fell through to "uncovered" — a gap that was not there.

Both decided what happened by pattern-matching cargo's English prose. That is
the defect, not the particular pattern each got wrong: every message shape
nobody anticipated silently becomes the wrong category.

So this classifies on **exit codes and ``--message-format=json``**, which cargo
emits precisely so that tools do not have to read its prose. And an outcome it
cannot recognise gets its own loud category rather than defaulting into a
bucket, because both earlier defects were silent defaults rather than wrong
branches.
"""

import json
import pathlib
import subprocess
import sys

#: The mutation did not compile, so it measured nothing. Not a coverage result.
BUILD_ERROR = "build-error"
#: The suite failed, so it noticed. This is what a guarded behaviour looks like.
KILLED = "killed"
#: The suite passed with the behaviour broken. **A gap.**
SURVIVED = "survived"
#: Neither could be established. Never a default — always a failure to report.
UNKNOWN = "unknown"

#: The two that mean the run did not do its job.
BAD = (SURVIVED, UNKNOWN)


def build_broke(exit_code, stdout):
    """Whether the mutated tree failed to compile.

    Two independent signals, because either alone has been wrong before: a
    non-zero exit from the build step, and any ``compiler-message`` at level
    ``error`` in cargo's JSON output. Neither reads prose.
    """
    if exit_code != 0:
        return True
    for line in stdout.splitlines():
        try:
            message = json.loads(line)
        except ValueError:
            # Not every line is JSON; cargo interleaves its own status output.
            continue
        if message.get("reason") != "compiler-message":
            continue
        if message.get("message", {}).get("level") == "error":
            return True
    return False


def classify(build_failed, test_exit):
    """Turn a build outcome and a test exit code into one of the four states.

    ``test_exit`` is ``None`` when the suite could not be run to completion,
    which is the case that must never be guessed at.
    """
    if build_failed:
        return BUILD_ERROR
    if test_exit is None:
        return UNKNOWN
    return SURVIVED if test_exit == 0 else KILLED


def apply_mutation(path, find, replace):
    """Swap the first occurrence, or raise if the anchor is gone.

    A spec whose anchor no longer matches is stale rather than passing. Failing
    loudly here is what stops a refactor quietly retiring a mutation.
    """
    original = path.read_text()
    if find not in original:
        raise LookupError(f"anchor not found in {path}: {find!r}")
    path.write_text(original.replace(find, replace, 1))
    return original


def build_command(spec):
    """The command that decides whether a mutated file still builds.

    cargo's by default: the suite's own command with ``--no-run`` and JSON
    messages, so the compile is judged apart from the tests. A spec over a
    file cargo does not build names its own ``build`` instead.

    🪤 **A Python spec cannot use the default.** Appending cargo's flags to a
    Python suite fails it on the flags, before a line of it runs, and every
    mutation would then read as a broken build. So such a spec compiles the
    mutated file on its own, and a mutation that breaks the syntax is still a
    broken build rather than a kill.
    """
    if "build" in spec:
        return spec["build"]
    return spec["command"] + ["--no-run", "--message-format=json"]


def run_one(path, command, mutation, build_with):
    """Apply one mutation, build it, run the suite, and restore the file."""
    original = apply_mutation(path, mutation["find"], mutation["replace"])
    try:
        build = subprocess.run(build_with, capture_output=True, text=True)
        if build_broke(build.returncode, build.stdout):
            return BUILD_ERROR
        tests = subprocess.run(command, capture_output=True, text=True)
        return classify(False, tests.returncode)
    except OSError as error:
        print(f"  could not run the suite: {error}", file=sys.stderr)
        return UNKNOWN
    finally:
        # Restored on every path, including an exception or a Ctrl-C, because a
        # harness that leaves a mutation behind corrupts the tree it tests.
        path.write_text(original)


def main(argv):
    if len(argv) != 2:
        print(__doc__.splitlines()[2].strip(), file=sys.stderr)
        return 2

    root = pathlib.Path(__file__).resolve().parent.parent
    spec = json.loads(pathlib.Path(argv[1]).read_text())
    path = root / spec["file"]
    command = spec["command"]
    build_with = build_command(spec)

    results = []
    for mutation in spec["mutations"]:
        outcome = run_one(path, command, mutation, build_with)
        results.append((outcome, mutation["name"]))
        print(f"{outcome.upper():<12} {mutation['name']}", flush=True)

    print()
    for state in (KILLED, BUILD_ERROR, SURVIVED, UNKNOWN):
        count = sum(1 for outcome, _ in results if outcome == state)
        print(f"{state:<12} {count}")

    bad = [name for outcome, name in results if outcome in BAD]
    if bad:
        print(f"\n{len(bad)} mutation(s) the suite did not catch:", file=sys.stderr)
        for name in bad:
            print(f"  {name}", file=sys.stderr)
        return 1

    # ⚠️ A build error is not a pass. It means that mutation measured nothing,
    # so reporting it alongside real kills as "all caught" would overstate what
    # the run established — the same flattering-direction error the two earlier
    # harnesses made.
    broken = [name for outcome, name in results if outcome == BUILD_ERROR]
    if broken:
        print(f"\n{len(broken)} mutation(s) measured nothing, because they did "
              "not compile. Fix the spec:")
        for name in broken:
            print(f"  {name}")
    caught = sum(1 for outcome, _ in results if outcome == KILLED)
    print(f"\n{caught} mutation(s) caught, {len(broken)} invalid, 0 missed")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
