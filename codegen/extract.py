#!/usr/bin/env python3
"""Extract stage: flatten Herdr's five sub-schemas into one self-contained document.

Herdr publishes one envelope holding five independent sub-schemas. Each keeps
its own ``$defs`` pool and refers to it as ``#/schemas/<name>/$defs/X``.
``cargo-typify`` reads a single self-contained document, so this stage rewrites
every reference to ``#/$defs/X`` and merges the five pools into one.

Merging is safe because the five pools agree: 219 definitions collapse to 183
distinct ones with no conflicting body. Merging is also what makes
``SplitDirection`` one Rust type instead of three, so a value read off an event
can be passed straight into a request.

Three guards run here. Each one stops the pipeline instead of guessing,
because every failure they catch is invisible downstream: the generated Rust
still compiles, and the drift only surfaces as a wrong type at some call site
nobody is looking at.

1. Every ``$ref`` must point inside its own sub-schema. All 384 do today. A
   cross-schema reference would mean Herdr had started sharing definitions
   across sub-schemas, which changes what "merge" means.
2. Every name defined twice must be defined identically. A disagreement means
   either a shared type changed on one side only, or a genuinely new type
   reused a taken name. The error prints both bodies so the reader can tell
   which within seconds.
3. Every rewritten reference must resolve. A dangling reference means the
   envelope lost a definition it still points at.
4. Every ``format: float`` is rewritten to ``double``, and there has to be at
   least one. ✅ **Measured 2026-09-12 against a live server**: Herdr declares
   ``float`` eight times and ``double`` never, typify maps the first to ``f32``,
   and the numbers the server actually sends do not fit one. This is the only
   place the pipeline deliberately disagrees with the published schema, which
   is why it is a guard rather than a quiet rewrite.

Run directly::

    python3 codegen/extract.py herdr-api.schema.json merged.json

Python 3.9 is the floor. This machine has no other interpreter.
"""

import argparse
import difflib
import json
import re
import sys
from typing import Any, Dict, Iterator, List, Tuple

#: Keys stripped from a sub-schema root before it joins the shared pool, so it
#: has the same shape as every other definition. No definition in the envelope
#: carries either key, and a nested ``$schema`` is not valid below a resource
#: root anyway.
ROOT_ONLY_KEYS = ("$schema", "title")

#: The reference style every sub-schema uses for its own definitions.
OWN_REF = re.compile(r"^#/schemas/(?P<schema>[A-Za-z_][A-Za-z0-9_]*)/\$defs/(?P<name>.+)$")

#: What Herdr declares every fractional number as, and what this stage rewrites
#: it to before generation.
#:
#: 🚨 **A deliberate divergence from the published schema, measured 2026-09-12
#: against a live 0.9.0 server.** typify maps ``float`` to ``f32``, correctly,
#: and an ``f32`` cannot hold the numbers this server sends: a `session.snapshot`
#: carrying ``0.69`` came back as ``0.6899999976158142``, and ``0.7`` as
#: ``0.699999988079071``. Deserialization succeeds either way, so the loss is
#: silent, and a plugin that reads a layout and applies it back corrupts every
#: ratio it touches.
#:
#: ⚠️ The same defect is documented from the other side in
#: ``agentic-panes-layout/docs/herdr-behaviour.md``: send ``ratio`` as a JSON
#: double, because serialising an ``f32`` widens it on the wire. One root cause,
#: both directions.
NARROW_FORMAT = "float"
WIDE_FORMAT = "double"

#: Splits a wire discriminator into the words a Rust name is built from.
#: ``server.live_handoff`` becomes ``ServerLiveHandoff``.
WORD_BOUNDARY = re.compile(r"[^0-9A-Za-z]+")


class DriftError(Exception):
    """The published schema no longer has the shape this pipeline was built for."""


def variant_name(discriminator: str) -> str:
    """Derive the Rust variant name for one wire discriminator.

    It lives here, in the module every other stage already imports, because
    three of them need it: the request sweep, the response sweep, and the stage
    that names one type per response variant. It reproduces the transform
    typify applies, and every run checks it against typify's own output by the
    fact that the emitted Rust has to compile.
    """
    words = [word for word in WORD_BOUNDARY.split(discriminator) if word]
    if not words:
        raise DriftError(f"the method {discriminator!r} has no name characters.")
    return "".join(word[:1].upper() + word[1:] for word in words)


def iter_formats(node: Any) -> Iterator[Dict[str, Any]]:
    """Yield every schema node anywhere in *node* that declares a ``format``."""
    if isinstance(node, dict):
        if isinstance(node.get("format"), str):
            yield node
        for value in node.values():
            yield from iter_formats(value)
    elif isinstance(node, list):
        for value in node:
            yield from iter_formats(value)


def widen_floats(node: Any) -> int:
    """Rewrite every ``float`` format to ``double``, in place, and count them.

    Guard 4 lives here, and it has two halves.

    A ``float`` on anything but a number means the keyword is being used for
    something this stage does not understand, so it stops rather than widening
    a type it has not reasoned about.

    Finding none at all is not decided here: see :func:`widened_numbers`, which
    the driver calls against the published schema.
    """
    widened = 0
    for owner in iter_formats(node):
        if owner["format"] != NARROW_FORMAT:
            continue
        kind = owner.get("type")
        admits_a_number = kind == "number" or (isinstance(kind, list) and "number" in kind)
        if not admits_a_number:
            raise DriftError(
                f"a schema declares {NARROW_FORMAT!r} on a {kind!r} rather than "
                f"on a number, so this stage will not widen it.\n"
                f"Widening is a claim about numeric precision and nothing else. "
                f"Read SCOPE.md section 3.2 before relaxing this."
            )
        owner["format"] = WIDE_FORMAT
        widened += 1

    return widened


def widened_numbers(document: Dict[str, Any]) -> int:
    """Count the widened numbers in *document*, refusing one that has none.

    The other half of guard 4, and it asks about the **result** rather than
    about the action: no ``float`` may survive anywhere, and at least one
    ``double`` has to exist. A rewrite that matched nothing looks exactly like
    a rewrite that worked, and that is how this defect returns unnoticed.

    It is separate from :func:`widen_floats` because only the published schema
    is known to carry fractional numbers. A test fixture that carries none is
    not drift.
    """
    formats = [node["format"] for node in iter_formats(document)]
    if NARROW_FORMAT in formats:
        raise DriftError(
            f"{formats.count(NARROW_FORMAT)} properties still declare "
            f"{NARROW_FORMAT!r} after the widening, so the rewrite did not "
            f"reach them all."
        )

    widened = formats.count(WIDE_FORMAT)
    if widened == 0:
        raise DriftError(
            f"the published schema declares no fractional number at all, so "
            f"this stage is doing nothing.\n"
            f"This is good news if Herdr now declares {WIDE_FORMAT!r} itself: "
            f"the server sends numbers an f32 cannot hold, and the widening "
            f"exists only because the published schema understated them. Check "
            f"what the schema declares now. If it is {WIDE_FORMAT!r} at the "
            f"source, delete the widening and this guard together.\n"
            f"If it declares neither, the ratios have changed shape and the "
            f"generated types need reading before anything ships. See SCOPE.md "
            f"section 3.2."
        )
    return widened


def iter_refs(node: Any) -> Iterator[str]:
    """Yield every ``$ref`` value anywhere in *node*."""
    if isinstance(node, dict):
        ref = node.get("$ref")
        if isinstance(ref, str):
            yield ref
        for value in node.values():
            yield from iter_refs(value)
    elif isinstance(node, list):
        for value in node:
            yield from iter_refs(value)


def rewrite_refs(node: Any, mapping: Dict[str, str]) -> Any:
    """Return a copy of *node* with each ``$ref`` replaced through *mapping*."""
    if isinstance(node, dict):
        return {
            key: mapping.get(value, value) if key == "$ref" and isinstance(value, str)
            else rewrite_refs(value, mapping)
            for key, value in node.items()
        }
    if isinstance(node, list):
        return [rewrite_refs(value, mapping) for value in node]
    return node


def _localise(schema_name: str, sub_schema: Any) -> Any:
    """Rewrite one sub-schema's own references to document-local ones.

    Guard 1 lives here. Anything that is not ``#/schemas/<schema_name>/$defs/X``
    fails closed, which covers a cross-schema reference, an external URL, and a
    pointer into some part of the document this pipeline does not understand.
    """
    mapping = {}
    for ref in iter_refs(sub_schema):
        match = OWN_REF.match(ref)
        if match is None or match.group("schema") != schema_name:
            raise DriftError(
                f"sub-schema {schema_name!r} contains the reference {ref!r}.\n"
                f"Every reference must point inside its own sub-schema, as "
                f"'#/schemas/{schema_name}/$defs/<name>'.\n"
                f"A reference to another sub-schema means Herdr now shares "
                f"definitions across sub-schemas. Re-read SCOPE.md section 3.2 "
                f"before relaxing this guard: merging assumes the five pools "
                f"are independent."
            )
        mapping[ref] = "#/$defs/" + match.group("name")
    return rewrite_refs(sub_schema, mapping)


def _body_diff(left_body: Any, right_body: Any, left: str, right: str) -> str:
    """A short unified diff of two definition bodies, for the collision report."""
    lines = difflib.unified_diff(
        json.dumps(left_body, indent=2, sort_keys=True).splitlines(),
        json.dumps(right_body, indent=2, sort_keys=True).splitlines(),
        fromfile=left,
        tofile=right,
        lineterm="",
    )
    return "\n".join(lines)


def _claim(pool: Dict[str, Any], origins: Dict[str, str], name: str, body: Any, origin: str) -> None:
    """Add *name* to the shared pool, or prove the existing entry agrees.

    Guard 2 lives here. Two sub-schemas may define the same name only when they
    define it identically, which is what makes one flat namespace correct
    rather than merely convenient.
    """
    if name not in pool:
        pool[name] = body
        origins[name] = origin
        return
    if pool[name] == body:
        return
    raise DriftError(
        f"the type {name!r} is defined twice with different bodies.\n"
        f"  first seen in: {origins[name]}\n"
        f"  also seen in:  {origin}\n\n"
        f"Two sub-schemas may share a name only when they share a definition. "
        f"Read the diff below to tell the two causes apart. A small change to "
        f"one side means Herdr changed a shared type and the other side has "
        f"not caught up. Two unrelated bodies mean a genuinely new type took a "
        f"name that was already taken, and it needs renaming before the merge "
        f"can be correct.\n\n"
        f"{_body_diff(pool[name], body, origins[name], origin)}"
    )


def extract(envelope: Dict[str, Any]) -> Tuple[Dict[str, Any], List[str]]:
    """Flatten *envelope* into one self-contained schema plus its root names.

    The returned document carries a root ``$ref`` rather than a root ``title``.
    A titled root makes ``cargo-typify`` emit a placeholder newtype over
    ``serde_json::Value`` for the container itself, which no caller wants. A
    root ``$ref`` suppresses it and leaves the generated type list otherwise
    byte-identical.
    """
    for key in ("protocol", "schema_version", "schemas"):
        if key not in envelope:
            raise DriftError(
                f"the fetched document has no {key!r} key, so it is not a Herdr "
                f"API envelope. Check the tag and the path in codegen/sync_api.py."
            )

    pool: Dict[str, Any] = {}
    origins: Dict[str, str] = {}
    roots: List[str] = []

    for schema_name in sorted(envelope["schemas"]):
        sub_schema = _localise(schema_name, envelope["schemas"][schema_name])
        for name, body in sub_schema.pop("$defs", {}).items():
            _claim(pool, origins, name, body, f"{schema_name}/$defs/{name}")

        title = sub_schema.pop("title", None)
        if not title:
            raise DriftError(
                f"sub-schema {schema_name!r} has no title, so its root type "
                f"cannot be named."
            )
        for key in ROOT_ONLY_KEYS:
            sub_schema.pop(key, None)
        _claim(pool, origins, title, sub_schema, f"{schema_name} root")
        roots.append(title)

    # Guard 3: every rewritten reference must resolve inside the merged pool.
    for ref in iter_refs(pool):
        if not ref.startswith("#/$defs/") or ref[len("#/$defs/"):] not in pool:
            raise DriftError(
                f"the reference {ref!r} does not resolve in the merged schema. "
                f"The envelope points at a definition it no longer carries."
            )

    # Guard 4: every fractional number is widened, or the pipeline stops. It
    # runs over the assembled pool so that one pass covers all five
    # sub-schemas, and it mutates the pool rather than the envelope, which
    # ``_localise`` has already rebuilt.
    widen_floats(pool)

    # Which root the ``$ref`` names does not matter, because typify emits no
    # type for it and the generated type list is identical whichever is picked.
    # Its only job is to stop typify inventing a root. Taking the root of the
    # first sub-schema in sorted order keeps the choice deterministic. The lift
    # stage repoints it if it happens to name the type that stage renames.
    document = {
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$ref": "#/$defs/" + roots[0],
        "$defs": dict(sorted(pool.items())),
    }
    return document, roots


def main(argv: List[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("envelope", help="Herdr's published herdr-api.schema.json")
    parser.add_argument("output", help="where to write the merged schema")
    args = parser.parse_args(argv)

    with open(args.envelope, encoding="utf-8") as handle:
        envelope = json.load(handle)

    try:
        document, roots = extract(envelope)
    except DriftError as error:
        print(f"extract: {error}", file=sys.stderr)
        return 1

    with open(args.output, "w", encoding="utf-8") as handle:
        json.dump(document, handle, indent=2, sort_keys=True)
        handle.write("\n")

    print(
        f"extract: {len(document['$defs'])} types from {len(roots)} sub-schemas "
        f"-> {args.output}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
