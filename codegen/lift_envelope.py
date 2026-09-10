#!/usr/bin/env python3
"""Lift stage: take the request envelope's ``id`` out of the way of the discriminator.

This stage exists to defuse one measured ``cargo-typify`` 0.8.0 defect, and it
is the reason the whole pipeline is written down rather than run by hand.

Herdr's request schema puts ``properties: {id}`` at the top level beside its
``oneOf``. typify has to merge that sibling property into all 102 branches, and
in that merge path it abandons the serde discriminator. The output collapses to
an untagged ``Variant0``..``Variant101``, ``method`` becomes an unconstrained
``String``, and every one of the 102 methods deserializes to ``Variant0``.

That failure is silent, and the silence is the danger:

* the broken types still round-trip JSON perfectly, because the method name
  rides along as an opaque string. A round-trip test passes cleanly against
  types that cannot dispatch. **Never read a green round-trip as evidence that
  these types are correct.**
* the broken types fail open. They accept an invented method name, and they
  accept a call whose required params are missing.

Removing the single ``id`` property before generation yields 102 correctly
named, correctly tagged variants. It is put back by hand, once, in
``crates/herdr-plugin-kit/src/api/envelope.rs``, wrapped around the generated
enum. That wrapper names no method, no variant, and no params shape, so it
survives every regeneration untouched.

The guards below all protect that hand-written wrapper. It hard-codes
``pub id: String``, so this stage refuses to run the moment the schema stops
saying exactly that. See SCOPE.md section 3.3.

Run directly::

    python3 codegen/lift_envelope.py merged.json lifted.json

Python 3.9 is the floor. This machine has no other interpreter.
"""

import argparse
import json
import sys
from typing import Any, Dict, List

from extract import DriftError, rewrite_refs

#: The generated name of the request root before the lift.
REQUEST_ROOT = "Request"

#: The generated name of the tagged enum after it, wrapped by ``Request`` in
#: ``envelope.rs``. The two names cannot be the same.
LIFTED_ROOT = "RequestMethod"

#: The sibling property lifted out, and the type ``envelope.rs`` declares for it.
LIFTED_PROPERTY = "id"
LIFTED_PROPERTY_SCHEMA = {"type": "string"}


def lift(document: Dict[str, Any]) -> Dict[str, Any]:
    """Return *document* with the request root's sibling property removed."""
    defs = document["$defs"]

    if REQUEST_ROOT not in defs:
        raise DriftError(
            f"the merged schema has no {REQUEST_ROOT!r} type, so there is "
            f"nothing to lift. Herdr renamed the request root."
        )
    if LIFTED_ROOT in defs:
        raise DriftError(
            f"the merged schema already defines {LIFTED_ROOT!r}, which is the "
            f"name this stage renames the request root to. Rename the new type "
            f"upstream, or pick another name here and in envelope.rs."
        )

    root = defs[REQUEST_ROOT]
    if "oneOf" not in root:
        raise DriftError(
            f"the {REQUEST_ROOT!r} type has no 'oneOf', so it no longer models "
            f"a choice of methods. The whole generation strategy assumes it does."
        )

    properties = root.get("properties")
    if properties is None:
        raise DriftError(
            f"the {REQUEST_ROOT!r} type has no sibling property beside its "
            f"'oneOf', so there is nothing to lift.\n"
            f"This is good news: typify keeps the discriminator when the 'oneOf' "
            f"stands alone. Delete this stage, and delete the hand-written "
            f"'{LIFTED_PROPERTY}' field from envelope.rs, which would now be a "
            f"field the schema does not declare."
        )
    if properties != {LIFTED_PROPERTY: LIFTED_PROPERTY_SCHEMA}:
        raise DriftError(
            f"the {REQUEST_ROOT!r} type's sibling properties are "
            f"{json.dumps(properties, sort_keys=True)}, not "
            f"{json.dumps({LIFTED_PROPERTY: LIFTED_PROPERTY_SCHEMA}, sort_keys=True)}.\n"
            f"envelope.rs hand-writes exactly one field to match this, so it is "
            f"now wrong. Update envelope.rs and this stage together, in the same "
            f"change, or the wrapper will silently drop or mistype a field Herdr "
            f"puts on the wire."
        )

    lifted = dict(root)
    lifted.pop("properties")
    remaining = [name for name in lifted.get("required", []) if name != LIFTED_PROPERTY]
    if remaining:
        lifted["required"] = remaining
    else:
        lifted.pop("required", None)

    defs = {LIFTED_ROOT if name == REQUEST_ROOT else name: body for name, body in defs.items()}
    defs[LIFTED_ROOT] = lifted

    renamed = {"#/$defs/" + REQUEST_ROOT: "#/$defs/" + LIFTED_ROOT}
    result = rewrite_refs({**document, "$defs": dict(sorted(defs.items()))}, renamed)
    return result


def main(argv: List[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("merged", help="the merged schema from extract.py")
    parser.add_argument("output", help="where to write the lifted schema")
    args = parser.parse_args(argv)

    with open(args.merged, encoding="utf-8") as handle:
        document = json.load(handle)

    try:
        lifted = lift(document)
    except DriftError as error:
        print(f"lift: {error}", file=sys.stderr)
        return 1

    with open(args.output, "w", encoding="utf-8") as handle:
        json.dump(lifted, handle, indent=2, sort_keys=True)
        handle.write("\n")

    count = len(lifted["$defs"][LIFTED_ROOT]["oneOf"])
    print(f"lift: {REQUEST_ROOT} -> {LIFTED_ROOT}, {count} methods -> {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
