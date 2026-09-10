#!/usr/bin/env python3
"""Emit the method sweep: one row per method, derived from the schema.

The sweep is the acceptance test for the lift stage, and it is generated rather
than hand-written for two reasons.

First, a hand-maintained table of 102 method names drifts. Herdr adds methods
every release, and a table nobody regenerates quietly stops covering the ones
that matter most, which are the new ones.

Second, and this is the part worth understanding before changing anything here:
**the generated sweep is an independent check on typify, not a restatement of
it.** Both this module and typify read the same schema and derive a variant
name from each method's ``const``. Neither reads the other's output. So the
emitted ``match`` arms are a second opinion:

* if typify abandons the discriminator again and collapses the enum to
  ``Variant0``..``Variant101``, the arms name variants that do not exist and
  the test crate does not compile.
* if typify renames, drops, or adds a variant, the arms disagree and the test
  crate does not compile, because the ``match`` is exhaustive.
* if the tag is wired to the wrong variant, the arms compile and the assertion
  fails at run time.

A round-trip test catches none of those three. See SCOPE.md section 3.3.

Run directly::

    python3 codegen/emit_sweep.py lifted.json crates/herdr-plugin-kit/tests/method_sweep.rs

Python 3.9 is the floor. This machine has no other interpreter.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, List, Sequence, Tuple

from extract import DriftError
from lift_envelope import LIFTED_ROOT

#: The edition the crate is built with. rustfmt needs telling, because it is
#: invoked directly rather than through cargo.
EDITION = "2021"

#: Splits a wire discriminator into the words a Rust variant name is built from.
#: ``server.live_handoff`` becomes ``ServerLiveHandoff``. This reproduces the
#: transform typify applies, and is checked against typify's own output every
#: run by the fact that the emitted match has to compile.
WORD_BOUNDARY = re.compile(r"[^0-9A-Za-z]+")


class Unsatisfiable(Exception):
    """No minimal instance of this sub-schema exists down this branch."""


def variant_name(discriminator: str) -> str:
    """Derive the Rust variant name for one method discriminator."""
    words = [word for word in WORD_BOUNDARY.split(discriminator) if word]
    if not words:
        raise DriftError(f"the method {discriminator!r} has no name characters.")
    return "".join(word[:1].upper() + word[1:] for word in words)


def minimal_instance(schema: Any, defs: Dict[str, Any], stack: Tuple[str, ...] = ()) -> Any:
    """Build the smallest JSON value the generated Rust will accept for *schema*.

    Smallest means required properties only, empty arrays, empty strings, and
    ``null`` wherever the schema admits it. Optional fields are left out on
    purpose, because a fixture carrying them would still pass if the generated
    type made a required field optional.

    *stack* carries the chain of definitions currently being expanded, so a
    self-referential type such as ``LayoutNode`` takes its terminating branch
    instead of recursing forever.
    """
    reference = schema.get("$ref")
    if reference is not None:
        name = reference[len("#/$defs/"):]
        if name in stack:
            raise Unsatisfiable(f"{name} refers to itself down this branch")
        return minimal_instance(defs[name], defs, stack + (name,))

    if "const" in schema:
        return schema["const"]
    if "enum" in schema:
        return schema["enum"][0]

    for keyword in ("oneOf", "anyOf"):
        branches = schema.get(keyword)
        if branches is None:
            continue
        for branch in branches:
            try:
                return minimal_instance(branch, defs, stack)
            except Unsatisfiable:
                continue
        raise Unsatisfiable(f"every {keyword} branch is unsatisfiable")

    kind = schema.get("type")
    if isinstance(kind, list):
        # A nullable type is at its smallest as null.
        if "null" in kind:
            return None
        kind = kind[0]

    if kind == "object":
        properties = schema.get("properties", {})
        instance = {}
        for name in schema.get("required", []):
            if name not in properties:
                raise DriftError(
                    f"a schema requires the property {name!r} but does not "
                    f"declare it, so no valid instance can be built."
                )
            instance[name] = minimal_instance(properties[name], defs, stack)
        return instance
    if kind == "array":
        return []
    if kind == "string":
        return ""
    if kind in ("integer", "number"):
        return schema.get("minimum", 0)
    if kind == "boolean":
        return False
    if kind == "null":
        return None

    raise Unsatisfiable(f"no rule for the sub-schema {json.dumps(schema)[:160]}")


def collect_cases(document: Dict[str, Any]) -> List[Tuple[str, str, str]]:
    """Return one ``(discriminator, variant, params JSON)`` row per method."""
    defs = document["$defs"]
    branches = defs[LIFTED_ROOT]["oneOf"]

    cases = []
    for index, branch in enumerate(branches):
        properties = branch.get("properties", {})
        if sorted(branch.get("required", [])) != ["method", "params"]:
            raise DriftError(
                f"method branch {index} requires "
                f"{branch.get('required')!r}, not ['method', 'params'].\n"
                f"Every branch must be a tagged pair for the generated enum to "
                f"be a newtype variant per method, which the sweep's match "
                f"arms assume."
            )
        discriminator = properties["method"].get("const")
        if not isinstance(discriminator, str):
            raise DriftError(
                f"method branch {index} has no string 'const' on its 'method' "
                f"property, so it carries no discriminator to sweep."
            )
        params = minimal_instance(properties["params"], defs)
        cases.append((discriminator, variant_name(discriminator), json.dumps(params)))

    _reject_duplicates(cases)
    return cases


def _reject_duplicates(cases: Sequence[Tuple[str, str, str]]) -> None:
    for column, label in ((0, "discriminator"), (1, "variant name")):
        seen: Dict[str, str] = {}
        for case in cases:
            if case[column] in seen:
                raise DriftError(
                    f"two methods share the {label} {case[column]!r}: "
                    f"{seen[case[column]]!r} and {case[0]!r}.\n"
                    f"The sweep cannot tell them apart, and neither can serde."
                )
            seen[case[column]] = case[0]


def render(cases: Sequence[Tuple[str, str, str]], header: str) -> str:
    """Render the sweep as a Rust integration test."""
    rows = []
    for discriminator, variant, params in cases:
        if '"#' in params:
            raise DriftError(
                f"the minimal params for {discriminator!r} contain '\"#', which "
                f"cannot sit inside a Rust raw string literal: {params}"
            )
        rows.append(f'    ({json.dumps(discriminator)}, {json.dumps(variant)}, r#"{params}"#),')

    arms = [f"        RequestMethod::{variant}(_) => {json.dumps(variant)}," for _, variant, _ in cases]

    return TEMPLATE.format(
        header=header,
        count=len(cases),
        rows="\n".join(rows),
        arms="\n".join(arms),
    )


TEMPLATE = '''{header}
//! Every method discriminator reaches its own named variant.
//!
//! This file is the acceptance test for the lift stage in `codegen/`. Read
//! `codegen/lift_envelope.py` before changing anything here.
//!
//! The rows and the match arms below were derived from Herdr's schema, not
//! from the generated Rust. That is what makes this a test rather than a
//! restatement: `cargo-typify` derives the same variant names from the same
//! schema by its own route, so the two only agree when both are right.
//!
//! Note what is deliberately *not* asserted: a JSON round-trip. The exact
//! defect this file exists to catch round-trips perfectly, because the method
//! name survives as an opaque string while every variant collapses into one.

use herdr_plugin_kit::api::generated::RequestMethod;
use herdr_plugin_kit::api::Request;

/// One row per method: the wire discriminator, the variant it must reach, and
/// the smallest params object the schema accepts for it.
///
/// Smallest is on purpose. A fixture carrying optional fields would still pass
/// if a required field had been generated as optional.
const CASES: &[(&str, &str, &str)] = &[
{rows}
];

/// Names the variant a value actually holds.
///
/// The match is exhaustive, so a variant that Herdr adds, renames, or drops
/// stops this file compiling until the sweep is regenerated.
fn variant_name(method: &RequestMethod) -> &'static str {{
    match method {{
{arms}
    }}
}}

fn envelope(method: &str, params: &str) -> String {{
    format!(r#"{{{{"id":"req-1","method":{{}},"params":{{}}}}}}"#, serde_json::to_string(method).unwrap(), params)
}}

#[test]
fn every_method_reaches_its_own_named_variant() {{
    assert_eq!(CASES.len(), {count}, "the sweep must cover every method Herdr declares");

    for (method, expected, params) in CASES {{
        let json = envelope(method, params);
        let request: Request = serde_json::from_str(&json)
            .unwrap_or_else(|error| panic!("{{method}} did not deserialize: {{error}}\\n{{json}}"));

        assert_eq!(
            variant_name(&request.method),
            *expected,
            "{{method}} reached the wrong variant",
        );
        assert_eq!(request.id, "req-1", "{{method}} lost the envelope id");
    }}
}}

#[test]
fn an_unknown_method_is_rejected() {{
    let json = envelope("no.such.method", "{{}}");
    let result: Result<Request, _> = serde_json::from_str(&json);
    assert!(
        result.is_err(),
        "an invented method was accepted, so the enum is untagged: {{result:?}}",
    );
}}

#[test]
fn a_method_with_no_params_is_rejected() {{
    let (method, _, _) = CASES[0];
    let json = format!(r#"{{{{"id":"req-1","method":"{{method}}"}}}}"#);
    let result: Result<Request, _> = serde_json::from_str(&json);
    assert!(result.is_err(), "{{method}} was accepted with no params at all");
}}

#[test]
fn a_method_with_empty_params_is_rejected_when_it_requires_some() {{
    let demanding: Vec<_> = CASES.iter().filter(|(_, _, params)| *params != "{{}}").collect();
    assert!(
        !demanding.is_empty(),
        "no method declares a required param, so this test cannot discriminate",
    );

    for (method, _, _) in demanding {{
        let json = envelope(method, "{{}}");
        let result: Result<Request, _> = serde_json::from_str(&json);
        assert!(result.is_err(), "{{method}} was accepted with empty params");
    }}
}}
'''


def write_rust(path: Path, source: str) -> None:
    """Write generated Rust, then hand it to rustfmt.

    Everything this pipeline writes is rustfmt-clean, so ``cargo fmt --check``
    stays green in CI without anyone having to remember. typify already formats
    its own output, which makes this a no-op there today. The rule is applied to
    both files anyway, so that a typify release which stops formatting shows up
    as nothing at all rather than as a red build nobody expected.
    """
    path.write_text(source, encoding="utf-8")
    result = subprocess.run(
        ["rustfmt", "--edition", EDITION, str(path)], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise DriftError(
            f"rustfmt rejected the generated {path.name}:\n{result.stderr.strip()}\n"
            f"The file is left in place so the offending Rust can be read."
        )


def main(argv: List[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("lifted", help="the lifted schema from lift_envelope.py")
    parser.add_argument("output", help="where to write the Rust sweep")
    parser.add_argument("--header", default="// @generated by codegen/emit_sweep.py", help="banner comment")
    args = parser.parse_args(argv)

    with open(args.lifted, encoding="utf-8") as handle:
        document = json.load(handle)

    try:
        cases = collect_cases(document)
        rendered = render(cases, args.header)
        write_rust(Path(args.output), rendered)
    except (DriftError, Unsatisfiable) as error:
        print(f"emit-sweep: {error}", file=sys.stderr)
        return 1

    print(f"emit-sweep: {len(cases)} methods -> {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
