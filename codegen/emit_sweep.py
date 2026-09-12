#!/usr/bin/env python3
"""Emit the sweeps: one row per method, and one row per response variant.

A sweep is the acceptance test for a generation stage, and both are generated
rather than hand-written for two reasons.

First, a hand-maintained table of 102 method names drifts. Herdr adds methods
every release, and a table nobody regenerates quietly stops covering the ones
that matter most, which are the new ones.

Second, and this is the part worth understanding before changing anything here:
**a generated sweep is an independent check on typify, not a restatement of
it.** Both this module and typify read the same schema and derive a variant
name from each ``const``. Neither reads the other's output. So the emitted
``match`` arms are a second opinion:

* if typify abandons the discriminator again and collapses the enum to
  ``Variant0``..``Variant101``, the arms name variants that do not exist and
  the test crate does not compile.
* if typify renames, drops, or adds a variant, the arms disagree and the test
  crate does not compile, because the ``match`` is exhaustive.
* if the tag is wired to the wrong variant, the arms compile and the assertion
  fails at run time.

A round-trip test catches none of those three. See SCOPE.md section 3.3.

The response sweep carries the same three checks over ``ResponseResult``, and
two more that only apply to it. Each response variant now also has a type of
its own (``codegen/split_results.py``), so the sweep holds the two renderings
of one schema branch against each other: the same fixture must deserialize
through both, and both must serialize back to the same JSON. And an answer
tagged for a *different* variant must be refused, which is the whole reason a
narrow type is safe to name.

⚠️ **The response side had no sweep at all until 2026-09-11**, while the
request side has had one since the pipeline was written. That was a gap in the
first release rather than something the per-variant types introduced:
``ResponseResult`` has always been able to collapse the way ``RequestMethod``
demonstrably did.

Run directly for the request sweep::

    python3 codegen/emit_sweep.py lifted.json crates/herdr-plugin-kit/tests/method_sweep.rs

The response sweep has no entry point of its own, because it needs the names
``codegen/split_results.py`` gave the branches. ``codegen/sync_api.py`` emits
both, and is always runnable.

Python 3.9 is the floor. This machine has no other interpreter.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict, List, Sequence, Tuple

from extract import DriftError, variant_name
from lift_envelope import LIFTED_ROOT

#: The edition the crate is built with. rustfmt needs telling, because it is
#: invoked directly rather than through cargo.
EDITION = "2021"


class Unsatisfiable(Exception):
    """No minimal instance of this sub-schema exists down this branch."""


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


def collect_result_cases(
    document: Dict[str, Any],
    named_branches: Sequence[Tuple[str, str, Dict[str, Any]]],
) -> List[Tuple[str, str, str, str, str, bool]]:
    """Return one row per response variant.

    A row is ``(discriminator, union variant, own type, fixture, a fixture
    tagged for the next variant, is a unit variant)``.

    *named_branches* arrives as ``(discriminator, type name, branch)``, already
    validated and named by ``split_results``. Taking it as an argument rather
    than deriving it keeps one validator for the union's shape instead of two,
    and keeps this module out of an import cycle with that stage.

    The mistagged fixture is this variant's own minimal instance wearing the
    *next* variant's tag. Every other field is present and correct, so the only
    reason to refuse it is the tag itself. That is what makes the refusal a
    test of the discriminator rather than of a missing field.
    """
    defs = document["$defs"]
    if len(named_branches) < 2:
        raise DriftError(
            f"there are {len(named_branches)} response variants, and the sweep "
            f"needs at least two: it tags each fixture with the next variant's "
            f"discriminator to prove a type refuses an answer meant for "
            f"another."
        )

    rows = []
    for index, (discriminator, answer, branch) in enumerate(named_branches):
        instance = minimal_instance(branch, defs)
        next_discriminator = named_branches[(index + 1) % len(named_branches)][0]
        mistagged = {**instance, "type": next_discriminator}
        rows.append((
            discriminator,
            variant_name(discriminator),
            answer,
            json.dumps(instance),
            json.dumps(mistagged),
            # typify emits a unit variant where the tag is the only property,
            # which is derived here from the schema rather than read off its
            # output. A disagreement stops the sweep compiling.
            list(instance) == ["type"],
        ))
    return rows


def render_results(cases: Sequence[Tuple[str, str, str, str, str, bool]], header: str) -> str:
    """Render the response sweep as a Rust integration test."""
    rows = []
    checks = []
    imports = ["ResponseResult"]
    for discriminator, variant, answer, fixture, mistagged, _ in cases:
        for literal in (fixture, mistagged):
            if '"#' in literal:
                raise DriftError(
                    f"the minimal result for {discriminator!r} contains '\"#', "
                    f"which cannot sit inside a Rust raw string literal: {literal}"
                )
        rows.append(f'    ({json.dumps(discriminator)}, {json.dumps(variant)}, r#"{fixture}"#),')
        checks.append(
            f'    mirrors!(checked, {answer}, {json.dumps(discriminator)}, '
            f'r#"{fixture}"#, r#"{mistagged}"#);'
        )
        imports.append(answer)

    arms = []
    for _, variant, _, _, _, unit in cases:
        pattern = f"ResponseResult::{variant}" + ("" if unit else " { .. }")
        arms.append(f"        {pattern} => {json.dumps(variant)},")

    return RESULT_TEMPLATE.format(
        header=header,
        count=len(cases),
        imports="\n".join(f"    {name}," for name in imports),
        rows="\n".join(rows),
        arms="\n".join(arms),
        checks="\n".join(checks),
    )


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


RESULT_TEMPLATE = '''{header}
//! Every result discriminator reaches its own named variant, and its own type.
//!
//! This file is the acceptance test for `codegen/split_results.py`, and the
//! response side's answer to `tests/method_sweep.rs`. Read both before
//! changing anything here.
//!
//! The rows, the match arms, and the type names below were derived from
//! Herdr's schema, not from the generated Rust. `cargo-typify` derives the
//! same names from the same schema by its own route, so the two only agree
//! when both are right.
//!
//! Three things are held up, and the last two exist because one schema branch
//! is now rendered twice: once as a variant of the union, and once as a type a
//! caller can name on its own.
//!
//! 1. Each discriminator reaches its own union variant. The `match` is
//!    exhaustive, so a variant Herdr adds, renames, or drops stops this file
//!    compiling until the sweep is regenerated.
//! 2. The two renderings agree. The same fixture deserializes through both,
//!    and both serialize back to the same JSON.
//! 3. A type refuses an answer tagged for another variant. That refusal is
//!    what makes naming one type safe: `workspace.move` answers
//!    `workspace_list`, so a caller who guesses the tag from the method name
//!    has to be told, rather than handed a wrong parse.
//!
//! Note what is deliberately *not* asserted: a JSON round-trip on its own. An
//! untagged enum round-trips perfectly while dispatching on shape instead of
//! on the tag, which is the defect SCOPE.md section 3.3 records on the request
//! side and section 3.5 records for two more encodings tried here.

use herdr_plugin_kit::api::generated::{{
{imports}
}};

/// One row per result: the wire discriminator, the union variant it must
/// reach, and the smallest result object the schema accepts for it.
///
/// Smallest is on purpose. A fixture carrying optional fields would still pass
/// if a required field had been generated as optional.
const CASES: &[(&str, &str, &str)] = &[
{rows}
];

/// Names the union variant a value actually holds.
fn variant_name(result: &ResponseResult) -> &'static str {{
    match result {{
{arms}
    }}
}}

#[test]
fn every_result_discriminator_reaches_its_own_named_variant() {{
    assert_eq!(CASES.len(), {count}, "the sweep must cover every result Herdr declares");

    for (tag, expected, fixture) in CASES {{
        let result: ResponseResult = serde_json::from_str(fixture)
            .unwrap_or_else(|error| panic!("{{tag}} did not deserialize: {{error}}\\n{{fixture}}"));

        assert_eq!(variant_name(&result), *expected, "{{tag}} reached the wrong variant");
    }}
}}

#[test]
fn a_result_type_this_build_does_not_know_is_rejected() {{
    let result: Result<ResponseResult, _> =
        serde_json::from_str(r#"{{"type":"invented_by_a_later_herdr"}}"#);

    assert!(
        result.is_err(),
        "an invented result type was accepted, so the enum is untagged: {{result:?}}",
    );
}}

#[test]
fn a_result_carrying_no_type_at_all_is_rejected() {{
    // An untagged enum accepts this by matching whichever variant needs
    // nothing. A tagged one has nothing to dispatch on and says so.
    let result: Result<ResponseResult, _> = serde_json::from_str("{{}}");

    assert!(
        result.is_err(),
        "an untagged object was accepted as a result: {{result:?}}",
    );
}}

/// Holds one variant's own type against the union arm it was split from.
macro_rules! mirrors {{
    ($checked:ident, $answer:ty, $tag:literal, $fixture:literal, $mistagged:literal) => {{{{
        let union: ResponseResult = serde_json::from_str($fixture)
            .unwrap_or_else(|error| panic!("{{}} is not a union value: {{}}", $tag, error));
        let narrow: $answer = serde_json::from_str($fixture).unwrap_or_else(|error| {{
            panic!("{{}} is not a {{}}: {{}}", $tag, stringify!($answer), error)
        }});

        assert_eq!(
            serde_json::to_value(&union).unwrap(),
            serde_json::to_value(&narrow).unwrap(),
            "{{}} serialises differently through {{}} than through the union",
            $tag,
            stringify!($answer),
        );

        let refused: Result<$answer, _> = serde_json::from_str($mistagged);
        assert!(
            refused.is_err(),
            "{{}} accepted {{}}, which is tagged for another variant",
            stringify!($answer),
            $mistagged,
        );

        $checked += 1;
    }}}};
}}

#[test]
fn every_result_type_mirrors_the_union_variant_it_was_split_from() {{
    let mut checked = 0usize;

{checks}

    assert_eq!(
        checked,
        CASES.len(),
        "every result in the sweep must have a type of its own",
    );
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
