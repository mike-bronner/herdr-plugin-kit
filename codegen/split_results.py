#!/usr/bin/env python3
"""Split stage: give every response branch a name a caller can say out loud.

``ResponseResult`` is one enum carrying all 64 shapes Herdr can answer with. A
caller that deserializes it pays for all 64, because serde generates parsing
code for every variant and dead-code elimination cannot drop any: each one is
reachable through the single type.

✅ **Measured 2026-09-11 by the recent-spaces migration.** Its release binary
went from 979,376 to 3,257,616 bytes on macOS arm64 at ``opt-level = "s"`` with
``strip = true``. Deserializing the full union accounts for 1,951,648 of that,
against 102,368 for serializing all 102 request methods. The plugin names one
variant of the 64.

So this stage adds one definition per branch, and typify emits a struct for
each beside the union it already emits. A caller names the one it expects, and
the other 63 are never instantiated.

**The union is not touched.** Its branches keep their own inline bodies, and
the generated enum is byte-identical with this stage and without it. That is
asserted in ``test_codegen.py`` rather than believed.

Two things make this a copy rather than a move, and both are measured:

1. 🚨 **typify abandons the discriminator when a ``oneOf`` branch is a
   ``$ref``.** The output is ``#[serde(untagged)]``, which is SCOPE.md section
   3.3's failure class in a different hat: it compiles, it round-trips, and it
   dispatches on shape instead of on the tag. Three encodings were tried and
   all three failed the same way. Section 3.5 records them.
2. The union's own variants have to stay as they are, because a caller that
   genuinely wants any response must still be able to ask for one.

The tag is rewritten from ``const`` to a single-valued ``enum`` on the injected
copy alone. The two say the same thing in JSON Schema, and typify reads them
differently: a ``const`` becomes an unvalidated ``String`` field, and a
single-valued ``enum`` becomes a one-variant Rust enum. Only the second refuses
an answer of the wrong shape, which matters because ``OkAnswer`` declares
nothing but its tag and would otherwise accept every object Herdr can send.

Run directly::

    python3 codegen/split_results.py lifted.json split.json

Python 3.9 is the floor. This machine has no other interpreter.
"""

import argparse
import copy
import json
import sys
from typing import Any, Dict, List, Sequence, Tuple

from extract import DriftError, variant_name

#: The generated name of the response union. This stage reads it and leaves it
#: exactly as it found it.
RESULT_ROOT = "ResponseResult"

#: What one branch's own type is called: its variant name, plus this.
#:
#: ⚠️ Not ``Result``. Herdr already names ten payload types that way —
#: ``PaneSwapResult``, ``PaneReadResult``, ``IntegrationInstallResult`` and
#: seven more — so that suffix collides with definitions this pipeline does not
#: own. ``Answer`` collides with nothing, and it is the word the transport
#: already uses for the thing that comes back.
ANSWER_SUFFIX = "Answer"

#: The property carrying the discriminator, in both halves of the schema.
TAG_PROPERTY = "type"

#: The trait the generated types implement, so ``Client::call`` accepts them
#: and nothing else. Hand-written in ``api/response.rs``, because it says
#: nothing the schema decides.
VARIANT_TRAIT = "crate::api::ResponseVariant"


def answer_name(discriminator: str) -> str:
    """The Rust type name for one response discriminator."""
    return variant_name(discriminator) + ANSWER_SUFFIX


def result_branches(document: Dict[str, Any]) -> List[Tuple[str, Dict[str, Any]]]:
    """Return one ``(discriminator, branch)`` pair per response variant.

    Every guard that reads the union's shape lives here, so the stage and the
    sweep cannot disagree about what a branch is.
    """
    defs = document["$defs"]
    if RESULT_ROOT not in defs:
        raise DriftError(
            f"the schema has no {RESULT_ROOT!r} type, so there are no response "
            f"variants to name. Herdr renamed the response root."
        )

    root = defs[RESULT_ROOT]
    if "oneOf" not in root:
        raise DriftError(
            f"the {RESULT_ROOT!r} type has no 'oneOf', so it no longer models a "
            f"choice of results. Both the per-variant types and the response "
            f"sweep assume it does."
        )

    pairs = []
    for index, branch in enumerate(root["oneOf"]):
        properties = branch.get("properties", {})
        tag = properties.get(TAG_PROPERTY, {})
        discriminator = tag.get("const")
        if not isinstance(discriminator, str):
            raise DriftError(
                f"response branch {index} has no string 'const' on its "
                f"{TAG_PROPERTY!r} property, so it carries no discriminator.\n"
                f"Every branch must be tagged for the union to be an enum serde "
                f"can dispatch, and for a per-variant type to be able to refuse "
                f"an answer meant for another variant."
            )
        if TAG_PROPERTY not in branch.get("required", []):
            raise DriftError(
                f"response branch {index} does not require its {TAG_PROPERTY!r} "
                f"property, so an answer could arrive without a discriminator "
                f"at all. Fail closed rather than name a type that accepts one."
            )
        pairs.append((discriminator, branch))

    _reject_duplicates(pairs)
    return pairs


def _reject_duplicates(pairs: List[Tuple[str, Dict[str, Any]]]) -> None:
    """Two branches may not share a discriminator or a derived name."""
    for derive, label in ((lambda value: value, "discriminator"),
                          (answer_name, "type name")):
        seen: Dict[str, str] = {}
        for discriminator, _ in pairs:
            key = derive(discriminator)
            if key in seen:
                raise DriftError(
                    f"two response variants share the {label} {key!r}: "
                    f"{seen[key]!r} and {discriminator!r}.\n"
                    f"Neither the sweep nor serde can tell them apart."
                )
            seen[key] = discriminator


def split(
    document: Dict[str, Any],
) -> Tuple[Dict[str, Any], List[Tuple[str, str, Dict[str, Any]]]]:
    """Return *document* with one definition added per response variant.

    The second element is one ``(discriminator, type name, branch)`` triple per
    variant, in the order the schema declares them. The branch rides along so
    the response sweep can build a fixture from the same object this stage
    named, rather than walking the union a second time. It is the original
    branch, tag and all, which is what the union renders.
    """
    defs = dict(document["$defs"])
    named = []

    for discriminator, branch in result_branches(document):
        name = answer_name(discriminator)
        if name in defs:
            raise DriftError(
                f"the response variant {discriminator!r} needs the type name "
                f"{name!r}, and the schema already defines it.\n"
                f"Herdr has taken the name this stage derives. Rename the new "
                f"type upstream, or change ANSWER_SUFFIX here and regenerate: "
                f"leaving it would silently replace a type the rest of the "
                f"generated file refers to."
            )
        defs[name] = _own_tag(branch, discriminator)
        named.append((discriminator, name, branch))

    if not named:
        raise DriftError(
            f"the {RESULT_ROOT!r} type declares no branches at all, so no "
            f"per-variant type can be named."
        )

    return {**document, "$defs": dict(sorted(defs.items()))}, named


def _own_tag(branch: Dict[str, Any], discriminator: str) -> Dict[str, Any]:
    """Copy *branch*, rewriting its ``const`` tag to a single-valued ``enum``.

    A deep copy, because the union reads the same object and must come out of
    typify unchanged. Every other key on the tag property is kept, so a
    description Herdr adds survives into the generated documentation.
    """
    own = copy.deepcopy(branch)
    tag = own["properties"][TAG_PROPERTY]
    tag.pop("const")
    tag["enum"] = [discriminator]
    return own


def impls(named: Sequence[Tuple[str, str, Any]], trait: str = VARIANT_TRAIT) -> str:
    """Render the trait implementations that make these types callable.

    One line each, and one for the union, which stays callable because a
    caller that wants any response has to be able to say so.
    """
    lines = [
        "",
        "// ---------------------------------------------------------------------------",
        "// Response variants. One `impl` per branch of `success_response`, from the",
        "// same pass that emitted the types above.",
        "//",
        "// The union is here too: naming it asks for any response Herdr can send, and",
        "// pays for parsing code for all of them. Naming one type pays for one.",
        "// ---------------------------------------------------------------------------",
        "",
        f"impl {trait} for {RESULT_ROOT} {{}}",
    ]
    lines.extend(f"impl {trait} for {name} {{}}" for _, name, _ in named)
    lines.append("")
    return "\n".join(lines)


def main(argv: List[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("lifted", help="the lifted schema from lift_envelope.py")
    parser.add_argument("output", help="where to write the split schema")
    args = parser.parse_args(argv)

    with open(args.lifted, encoding="utf-8") as handle:
        document = json.load(handle)

    try:
        split_document, named = split(document)
    except DriftError as error:
        print(f"split: {error}", file=sys.stderr)
        return 1

    with open(args.output, "w", encoding="utf-8") as handle:
        json.dump(split_document, handle, indent=2, sort_keys=True)
        handle.write("\n")

    print(f"split: {len(named)} response variants -> {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
