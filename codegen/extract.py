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


class DriftError(Exception):
    """The published schema no longer has the shape this pipeline was built for."""


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
