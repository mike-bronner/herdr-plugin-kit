#!/usr/bin/env python3
"""Tests for the codegen guards.

The guards are the whole point of writing this pipeline down. Each one catches
a schema change that would otherwise produce Rust that still compiles and is
quietly wrong, so an untested guard is worth nothing: it is a claim, not a
check.

No network, and no dependency beyond the standard library. Run with::

    python3 codegen/test_codegen.py

Python 3.9 is the floor. This machine has no other interpreter.
"""

import copy
import json
import tempfile
import unittest
from pathlib import Path

from emit_sweep import (
    Unsatisfiable,
    collect_cases,
    collect_result_cases,
    minimal_instance,
    render,
    render_results,
    write_rust,
)
from extract import (  # noqa: E402
    NARROW_FORMAT,
    WIDE_FORMAT,
    DriftError,
    extract,
    variant_name,
    widen_floats,
    widened_numbers,
)
from lift_envelope import LIFTED_ROOT, REQUEST_ROOT, lift
from split_results import RESULT_ROOT, impls, split


def envelope(**schemas):
    """A Herdr-shaped envelope around the given sub-schemas."""
    return {"protocol": 22, "schema_version": 1, "title": "Test API", "schemas": schemas}


def sub_schema(title, defs=None, **rest):
    document = {"$schema": "https://json-schema.org/draft/2020-12/schema", "title": title}
    document.update(rest)
    document.setdefault("type", "object")
    document["$defs"] = defs or {}
    return document


class ExtractRefGuard(unittest.TestCase):
    """Guard 1: every reference must point inside its own sub-schema."""

    def test_a_cross_schema_reference_stops_the_pipeline(self):
        document = envelope(
            alpha=sub_schema("Alpha", properties={"x": {"$ref": "#/schemas/beta/$defs/Shared"}}),
            beta=sub_schema("Beta", defs={"Shared": {"type": "string"}}),
        )
        with self.assertRaises(DriftError) as caught:
            extract(document)
        message = str(caught.exception)
        self.assertIn("#/schemas/beta/$defs/Shared", message)
        self.assertIn("'alpha'", message)

    def test_an_external_reference_stops_the_pipeline(self):
        document = envelope(
            alpha=sub_schema("Alpha", properties={"x": {"$ref": "https://example.invalid/x.json"}}),
        )
        with self.assertRaises(DriftError):
            extract(document)

    def test_a_pointer_outside_defs_stops_the_pipeline(self):
        document = envelope(
            alpha=sub_schema("Alpha", properties={"x": {"$ref": "#/schemas/alpha/properties/y"}}),
        )
        with self.assertRaises(DriftError):
            extract(document)

    def test_an_own_reference_is_rewritten_to_a_local_one(self):
        document = envelope(
            alpha=sub_schema(
                "Alpha",
                defs={"Shared": {"type": "string"}},
                properties={"x": {"$ref": "#/schemas/alpha/$defs/Shared"}},
            ),
        )
        merged, _ = extract(document)
        self.assertEqual(merged["$defs"]["Alpha"]["properties"]["x"]["$ref"], "#/$defs/Shared")


class ExtractCollisionGuard(unittest.TestCase):
    """Guard 2: a name defined twice must be defined identically."""

    def test_a_conflicting_duplicate_stops_the_pipeline(self):
        document = envelope(
            alpha=sub_schema("Alpha", defs={"Shared": {"type": "string"}}),
            beta=sub_schema("Beta", defs={"Shared": {"type": "integer"}}),
        )
        with self.assertRaises(DriftError) as caught:
            extract(document)
        message = str(caught.exception)
        self.assertIn("'Shared'", message)
        self.assertIn("alpha/$defs/Shared", message)
        self.assertIn("beta/$defs/Shared", message)
        # The reader has to be able to tell a changed shared type from a new
        # type that took a taken name, so both bodies are in the report.
        self.assertIn('"type": "string"', message)
        self.assertIn('"type": "integer"', message)

    def test_an_identical_duplicate_merges_into_one_type(self):
        document = envelope(
            alpha=sub_schema("Alpha", defs={"Shared": {"type": "string"}}),
            beta=sub_schema("Beta", defs={"Shared": {"type": "string"}}),
        )
        merged, roots = extract(document)
        self.assertEqual(merged["$defs"]["Shared"], {"type": "string"})
        self.assertEqual(sorted(roots), ["Alpha", "Beta"])

    def test_a_root_title_may_repeat_a_definition_name_when_the_bodies_agree(self):
        # Herdr does exactly this today: EventEnvelope is the root of the event
        # sub-schema and a definition inside success_response.
        body = {"type": "object", "properties": {"x": {"type": "string"}}}
        document = envelope(
            alpha=sub_schema("Shared", **body),
            beta=sub_schema("Beta", defs={"Shared": dict(body)}),
        )
        merged, _ = extract(document)
        self.assertEqual(merged["$defs"]["Shared"], body)

    def test_a_root_title_that_contradicts_a_definition_stops_the_pipeline(self):
        document = envelope(
            alpha=sub_schema("Shared", properties={"x": {"type": "string"}}),
            beta=sub_schema("Beta", defs={"Shared": {"type": "integer"}}),
        )
        with self.assertRaises(DriftError) as caught:
            extract(document)
        self.assertIn("'Shared'", str(caught.exception))


class ExtractStructureGuards(unittest.TestCase):
    """Guard 3, and the envelope-shape checks around it."""

    def test_a_dangling_reference_stops_the_pipeline(self):
        document = envelope(
            alpha=sub_schema("Alpha", properties={"x": {"$ref": "#/schemas/alpha/$defs/Gone"}}),
        )
        with self.assertRaises(DriftError) as caught:
            extract(document)
        self.assertIn("does not resolve", str(caught.exception))

    def test_a_document_that_is_not_an_envelope_stops_the_pipeline(self):
        for missing in ("protocol", "schema_version", "schemas"):
            document = envelope(alpha=sub_schema("Alpha"))
            del document[missing]
            with self.assertRaises(DriftError, msg=missing):
                extract(document)

    def test_an_untitled_sub_schema_stops_the_pipeline(self):
        document = envelope(alpha=sub_schema("Alpha"))
        del document["schemas"]["alpha"]["title"]
        with self.assertRaises(DriftError) as caught:
            extract(document)
        self.assertIn("no title", str(caught.exception))

    def test_the_root_reference_resolves_and_emits_no_extra_type(self):
        document = envelope(alpha=sub_schema("Alpha"), beta=sub_schema("Beta"))
        merged, _ = extract(document)
        target = merged["$ref"][len("#/$defs/"):]
        self.assertIn(target, merged["$defs"])


class WideningFloats(unittest.TestCase):
    """Guard 4: the one place the pipeline disagrees with the published schema.

    🚨 typify maps `float` to `f32`, correctly, and an `f32` cannot hold what
    the server sends. Measured 2026-09-12 against a live 0.9.0: a ratio of
    `0.69` came back `0.6899999976158142`. Deserialization succeeds, so the
    loss is silent.
    """

    def test_a_float_becomes_a_double(self):
        schema = {"type": "number", "format": NARROW_FORMAT}
        self.assertEqual(widen_floats(schema), 1)
        self.assertEqual(schema["format"], WIDE_FORMAT)

    def test_a_nullable_float_is_widened_too(self):
        # PaneResizeParams.amount and PaneSplitParams.ratio are both nullable,
        # so a rule that only matched a bare "number" would miss three of the
        # eight sites Herdr declares.
        schema = {"type": ["number", "null"], "format": NARROW_FORMAT}
        self.assertEqual(widen_floats(schema), 1)
        self.assertEqual(schema["format"], WIDE_FORMAT)

    def test_it_reaches_every_depth(self):
        schema = {"$defs": {"A": {"properties": {"r": {"type": "number", "format": NARROW_FORMAT}}},
                            "B": {"oneOf": [{"type": "number", "format": NARROW_FORMAT}]}}}
        self.assertEqual(widen_floats(schema), 2)
        self.assertNotIn(NARROW_FORMAT, json.dumps(schema))

    def test_no_other_format_is_touched(self):
        # Herdr declares int32, uint, uint16, uint32 and uint64 as well, and
        # every one of them is a correct claim about an integer.
        schema = {"a": {"type": "integer", "format": "uint64"},
                  "b": {"type": "integer", "format": "int32"}}
        widen_floats({"c": {"type": "number", "format": NARROW_FORMAT}, **schema})
        self.assertEqual(schema["a"]["format"], "uint64")
        self.assertEqual(schema["b"]["format"], "int32")

    def test_a_float_on_something_that_is_not_a_number_stops_the_pipeline(self):
        # Widening is a claim about numeric precision and nothing else.
        with self.assertRaises(DriftError) as caught:
            widen_floats({"type": "string", "format": NARROW_FORMAT})
        self.assertIn("rather than", str(caught.exception))

    def test_the_envelope_the_caller_passed_is_not_widened(self):
        # extract rebuilds the pool, so the rewrite lands on the copy. A caller
        # holding the fetched document still sees what Herdr published.
        document = envelope(
            alpha=sub_schema("Alpha", properties={"r": {"type": "number", "format": NARROW_FORMAT}}),
        )
        merged, _ = extract(document)

        self.assertEqual(
            document["schemas"]["alpha"]["properties"]["r"]["format"], NARROW_FORMAT
        )
        self.assertEqual(merged["$defs"]["Alpha"]["properties"]["r"]["format"], WIDE_FORMAT)

    def test_a_schema_with_no_fractional_number_at_all_stops_the_driver(self):
        # A rewrite that matched nothing looks exactly like one that worked.
        with self.assertRaises(DriftError) as caught:
            widened_numbers({"type": "object"})
        message = str(caught.exception)
        self.assertIn("doing nothing", message)
        # Whoever hits this needs to be told the good-news reading too.
        self.assertIn("good news", message)
        self.assertIn("delete the widening", message)

    def test_a_surviving_float_stops_the_driver(self):
        with self.assertRaises(DriftError) as caught:
            widened_numbers({"a": {"type": "number", "format": NARROW_FORMAT},
                             "b": {"type": "number", "format": WIDE_FORMAT}})
        self.assertIn("did not reach them all", str(caught.exception))

    def test_it_counts_what_it_found(self):
        self.assertEqual(
            widened_numbers({"a": {"format": WIDE_FORMAT}, "b": {"format": WIDE_FORMAT}}), 2
        )


def request_document(properties=None, required=None, branches=None, extra_defs=None):
    """A merged document shaped like the one the lift stage receives."""
    root = {
        "type": "object",
        "oneOf": branches if branches is not None else [
            {
                "type": "object",
                "properties": {"method": {"const": "ping", "type": "string"},
                               "params": {"$ref": "#/$defs/PingParams"}},
                "required": ["method", "params"],
            },
        ],
    }
    if properties is not None:
        root["properties"] = properties
    if required is not None:
        root["required"] = required
    defs = {"PingParams": {"type": "object"}, REQUEST_ROOT: root}
    defs.update(extra_defs or {})
    return {"$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "#/$defs/PingParams", "$defs": defs}


class LiftGuards(unittest.TestCase):
    """The lift protects the one hand-written type in the generated layer."""

    def test_the_sibling_property_is_removed_and_the_root_renamed(self):
        document = request_document(properties={"id": {"type": "string"}}, required=["id"])
        lifted = lift(document)["$defs"]
        self.assertNotIn(REQUEST_ROOT, lifted)
        self.assertNotIn("properties", lifted[LIFTED_ROOT])
        self.assertNotIn("required", lifted[LIFTED_ROOT])
        self.assertEqual(len(lifted[LIFTED_ROOT]["oneOf"]), 1)

    def test_other_required_names_survive_the_lift(self):
        document = request_document(
            properties={"id": {"type": "string"}, "trace": {"type": "string"}},
            required=["id", "trace"],
        )
        # Two siblings is itself drift, so prove the required list is filtered
        # rather than dropped by lifting a document the shape guard accepts.
        document["$defs"][REQUEST_ROOT]["properties"] = {"id": {"type": "string"}}
        lifted = lift(document)["$defs"][LIFTED_ROOT]
        self.assertEqual(lifted["required"], ["trace"])

    def test_a_reference_to_the_renamed_root_is_repointed(self):
        document = request_document(
            properties={"id": {"type": "string"}},
            required=["id"],
            extra_defs={"Wrapper": {"$ref": "#/$defs/" + REQUEST_ROOT}},
        )
        lifted = lift(document)
        self.assertEqual(lifted["$defs"]["Wrapper"]["$ref"], "#/$defs/" + LIFTED_ROOT)

    def test_a_missing_request_root_stops_the_pipeline(self):
        document = request_document(properties={"id": {"type": "string"}}, required=["id"])
        del document["$defs"][REQUEST_ROOT]
        with self.assertRaises(DriftError) as caught:
            lift(document)
        self.assertIn(REQUEST_ROOT, str(caught.exception))

    def test_a_taken_target_name_stops_the_pipeline(self):
        document = request_document(
            properties={"id": {"type": "string"}}, required=["id"],
            extra_defs={LIFTED_ROOT: {"type": "string"}},
        )
        with self.assertRaises(DriftError) as caught:
            lift(document)
        self.assertIn("already defines", str(caught.exception))

    def test_a_request_root_without_a_choice_of_methods_stops_the_pipeline(self):
        document = request_document(properties={"id": {"type": "string"}}, required=["id"])
        del document["$defs"][REQUEST_ROOT]["oneOf"]
        with self.assertRaises(DriftError) as caught:
            lift(document)
        self.assertIn("'oneOf'", str(caught.exception))

    def test_no_sibling_property_stops_the_pipeline_with_the_good_news(self):
        document = request_document()
        with self.assertRaises(DriftError) as caught:
            lift(document)
        message = str(caught.exception)
        self.assertIn("nothing to lift", message)
        # Whoever hits this needs to be told to delete the hand-written field
        # too, or envelope.rs starts declaring one the schema does not.
        self.assertIn("envelope.rs", message)

    def test_a_renamed_sibling_property_stops_the_pipeline(self):
        document = request_document(properties={"request_id": {"type": "string"}},
                                    required=["request_id"])
        with self.assertRaises(DriftError) as caught:
            lift(document)
        self.assertIn("envelope.rs", str(caught.exception))

    def test_a_retyped_sibling_property_stops_the_pipeline(self):
        document = request_document(properties={"id": {"type": "integer"}}, required=["id"])
        with self.assertRaises(DriftError) as caught:
            lift(document)
        self.assertIn("envelope.rs", str(caught.exception))

    def test_a_second_sibling_property_stops_the_pipeline(self):
        document = request_document(
            properties={"id": {"type": "string"}, "trace": {"type": "string"}},
            required=["id"],
        )
        with self.assertRaises(DriftError) as caught:
            lift(document)
        self.assertIn("envelope.rs", str(caught.exception))


class VariantNames(unittest.TestCase):
    def test_a_dotted_and_underscored_name_becomes_one_camel_case_word(self):
        self.assertEqual(variant_name("server.live_handoff"), "ServerLiveHandoff")
        self.assertEqual(variant_name("ping"), "Ping")
        self.assertEqual(variant_name("pane.graphics.frame_ack"), "PaneGraphicsFrameAck")

    def test_a_name_with_no_letters_stops_the_pipeline(self):
        with self.assertRaises(DriftError):
            variant_name("...")


class MinimalInstances(unittest.TestCase):
    """The fixture builder has to produce the *smallest* accepted value."""

    def test_a_nullable_type_is_smallest_as_null(self):
        self.assertIsNone(minimal_instance({"type": ["string", "null"]}, {}))

    def test_scalars_take_their_smallest_value(self):
        self.assertEqual(minimal_instance({"type": "string"}, {}), "")
        self.assertEqual(minimal_instance({"type": "integer"}, {}), 0)
        self.assertEqual(minimal_instance({"type": "integer", "minimum": 3}, {}), 3)
        self.assertEqual(minimal_instance({"type": "boolean"}, {}), False)
        self.assertEqual(minimal_instance({"type": "array"}, {}), [])

    def test_a_bare_true_schema_is_smallest_as_null(self):
        # ✅ Herdr writes one, for `agent_explain`'s `explain`, which typify
        # generates as `serde_json::Value`.
        self.assertIsNone(minimal_instance(True, {}))
        self.assertEqual(
            minimal_instance({"type": "object", "properties": {"x": True}, "required": ["x"]}, {}),
            {"x": None},
        )

    def test_a_bare_false_schema_has_no_instance_at_all(self):
        with self.assertRaises(Unsatisfiable):
            minimal_instance(False, {})

    def test_a_const_and_an_enum_take_the_declared_value(self):
        self.assertEqual(minimal_instance({"const": "pane"}, {}), "pane")
        self.assertEqual(minimal_instance({"enum": ["a", "b"]}, {}), "a")

    def test_optional_properties_are_left_out(self):
        schema = {
            "type": "object",
            "properties": {"needed": {"type": "string"}, "spare": {"type": "string"}},
            "required": ["needed"],
        }
        self.assertEqual(minimal_instance(schema, {}), {"needed": ""})

    def test_a_required_property_that_is_not_declared_stops_the_pipeline(self):
        schema = {"type": "object", "properties": {}, "required": ["ghost"]}
        with self.assertRaises(DriftError):
            minimal_instance(schema, {})

    def test_a_self_referential_type_takes_its_terminating_branch(self):
        defs = {
            "Node": {
                "oneOf": [
                    {"type": "object", "properties": {"kind": {"const": "split"},
                                                      "child": {"$ref": "#/$defs/Node"}},
                     "required": ["kind", "child"]},
                    {"type": "object", "properties": {"kind": {"const": "leaf"}},
                     "required": ["kind"]},
                ]
            }
        }
        self.assertEqual(minimal_instance({"$ref": "#/$defs/Node"}, defs), {"kind": "leaf"})

    def test_a_type_with_no_terminating_branch_stops_the_pipeline(self):
        defs = {"Loop": {"$ref": "#/$defs/Loop"}}
        with self.assertRaises(Unsatisfiable):
            minimal_instance({"$ref": "#/$defs/Loop"}, defs)

    def test_an_unrecognised_construct_stops_the_pipeline(self):
        with self.assertRaises(Unsatisfiable):
            minimal_instance({"not": {"type": "string"}}, {})


def method_branch(discriminator, params="#/$defs/PingParams", required=("method", "params")):
    return {
        "type": "object",
        "properties": {"method": {"const": discriminator, "type": "string"},
                       "params": {"$ref": params}},
        "required": list(required),
    }


class SweepCases(unittest.TestCase):
    def test_each_branch_becomes_one_row(self):
        document = request_document(branches=[method_branch("ping"), method_branch("server.stop")])
        document["$defs"][LIFTED_ROOT] = document["$defs"].pop(REQUEST_ROOT)
        cases = collect_cases(document)
        self.assertEqual(cases, [("ping", "Ping", "{}"), ("server.stop", "ServerStop", "{}")])

    def test_a_branch_that_is_not_a_tagged_pair_stops_the_pipeline(self):
        document = request_document(branches=[method_branch("ping", required=("method",))])
        document["$defs"][LIFTED_ROOT] = document["$defs"].pop(REQUEST_ROOT)
        with self.assertRaises(DriftError) as caught:
            collect_cases(document)
        self.assertIn("tagged pair", str(caught.exception))

    def test_a_branch_with_no_discriminator_stops_the_pipeline(self):
        branch = method_branch("ping")
        del branch["properties"]["method"]["const"]
        document = request_document(branches=[branch])
        document["$defs"][LIFTED_ROOT] = document["$defs"].pop(REQUEST_ROOT)
        with self.assertRaises(DriftError) as caught:
            collect_cases(document)
        self.assertIn("discriminator", str(caught.exception))

    def test_two_methods_that_share_a_variant_name_stop_the_pipeline(self):
        # 'a.b' and 'a_b' both camel-case to 'AB'. serde could not tell the
        # generated variants apart either.
        document = request_document(branches=[method_branch("a.b"), method_branch("a_b")])
        document["$defs"][LIFTED_ROOT] = document["$defs"].pop(REQUEST_ROOT)
        with self.assertRaises(DriftError) as caught:
            collect_cases(document)
        self.assertIn("variant name", str(caught.exception))

    def test_two_identical_discriminators_stop_the_pipeline(self):
        document = request_document(branches=[method_branch("ping"), method_branch("ping")])
        document["$defs"][LIFTED_ROOT] = document["$defs"].pop(REQUEST_ROOT)
        with self.assertRaises(DriftError) as caught:
            collect_cases(document)
        self.assertIn("discriminator", str(caught.exception))


def result_branch(discriminator, properties=None, required=None):
    """One branch of the response union, shaped the way Herdr writes them."""
    declared = {"type": {"const": discriminator, "type": "string"}}
    declared.update(properties or {})
    return {
        "type": "object",
        "properties": declared,
        "required": list(required) if required is not None else list(declared),
    }


def result_document(branches=None, extra_defs=None):
    """A lifted document carrying a response union."""
    defs = {
        RESULT_ROOT: {
            "oneOf": branches if branches is not None else [
                result_branch("pong", {"version": {"type": "string"}}),
                result_branch("ok"),
            ]
        }
    }
    defs.update(extra_defs or {})
    return {"$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": "#/$defs/" + RESULT_ROOT, "$defs": defs}


class SplitGuards(unittest.TestCase):
    """One type per response variant, and the union left exactly as it was."""

    def test_each_branch_gets_a_definition_of_its_own(self):
        split_document, named = split(result_document())

        self.assertEqual(named[0][:2], ("pong", "PongAnswer"))
        self.assertEqual(named[1][:2], ("ok", "OkAnswer"))
        self.assertIn("PongAnswer", split_document["$defs"])
        self.assertIn("OkAnswer", split_document["$defs"])

    def test_the_union_is_left_exactly_as_it_was(self):
        # 🔑 The whole design rests on this. The union is copied, never moved,
        # so the generated enum comes out of typify byte-identical and nothing
        # that matches it today has to move.
        document = result_document()
        before = copy.deepcopy(document)

        split_document, _ = split(document)

        self.assertEqual(document, before, "split mutated the document it was given")
        self.assertEqual(split_document["$defs"][RESULT_ROOT], before["$defs"][RESULT_ROOT])

    def test_the_injected_copy_validates_its_tag(self):
        # A `const` generates an unvalidated String field, and a single-valued
        # `enum` generates a one-variant Rust enum. Only the second refuses an
        # answer meant for another variant.
        split_document, _ = split(result_document())
        tag = split_document["$defs"]["PongAnswer"]["properties"]["type"]

        self.assertEqual(tag, {"type": "string", "enum": ["pong"]})
        self.assertNotIn("const", tag)

    def test_other_keys_on_the_tag_survive_the_rewrite(self):
        branch = result_branch("pong")
        branch["properties"]["type"]["description"] = "what this answer is"
        split_document, _ = split(result_document(branches=[branch, result_branch("ok")]))

        self.assertEqual(
            split_document["$defs"]["PongAnswer"]["properties"]["type"]["description"],
            "what this answer is",
        )

    def test_a_missing_response_root_stops_the_pipeline(self):
        document = result_document()
        del document["$defs"][RESULT_ROOT]
        with self.assertRaises(DriftError) as caught:
            split(document)
        self.assertIn(RESULT_ROOT, str(caught.exception))

    def test_a_response_root_without_a_choice_of_results_stops_the_pipeline(self):
        document = result_document()
        document["$defs"][RESULT_ROOT] = {"type": "object"}
        with self.assertRaises(DriftError) as caught:
            split(document)
        self.assertIn("'oneOf'", str(caught.exception))

    def test_a_union_with_no_branches_stops_the_pipeline(self):
        with self.assertRaises(DriftError) as caught:
            split(result_document(branches=[]))
        self.assertIn("no branches", str(caught.exception))

    def test_a_branch_with_no_discriminator_stops_the_pipeline(self):
        branch = result_branch("pong")
        del branch["properties"]["type"]["const"]
        with self.assertRaises(DriftError) as caught:
            split(result_document(branches=[branch]))
        self.assertIn("discriminator", str(caught.exception))

    def test_a_branch_that_does_not_require_its_tag_stops_the_pipeline(self):
        # An optional tag means an answer can arrive with no discriminator at
        # all, which no per-variant type could refuse.
        branch = result_branch("pong", required=[])
        with self.assertRaises(DriftError) as caught:
            split(result_document(branches=[branch]))
        self.assertIn("without a discriminator", str(caught.exception))

    def test_a_type_name_the_schema_already_uses_stops_the_pipeline(self):
        # Herdr names ten payload types `*Result` today. The day it names one
        # `PongAnswer`, this stage must not quietly replace it.
        document = result_document(extra_defs={"PongAnswer": {"type": "string"}})
        with self.assertRaises(DriftError) as caught:
            split(document)
        self.assertIn("PongAnswer", str(caught.exception))
        self.assertIn("already defines", str(caught.exception))

    def test_two_variants_sharing_a_derived_name_stop_the_pipeline(self):
        document = result_document(branches=[result_branch("a.b"), result_branch("a_b")])
        with self.assertRaises(DriftError) as caught:
            split(document)
        self.assertIn("type name", str(caught.exception))

    def test_two_variants_sharing_a_discriminator_stop_the_pipeline(self):
        document = result_document(branches=[result_branch("ok"), result_branch("ok")])
        with self.assertRaises(DriftError) as caught:
            split(document)
        self.assertIn("discriminator", str(caught.exception))

    def test_the_union_stays_callable_alongside_its_variants(self):
        _, named = split(result_document())
        rendered = impls(named, trait="Trait")

        self.assertIn("impl Trait for ResponseResult {}", rendered)
        self.assertIn("impl Trait for PongAnswer {}", rendered)
        self.assertIn("impl Trait for OkAnswer {}", rendered)


class ResultSweepCases(unittest.TestCase):
    def test_each_branch_becomes_one_row(self):
        document = result_document()
        _, named = split(document)

        cases = collect_result_cases(document, named)

        self.assertEqual(cases[0][:3], ("pong", "Pong", "PongAnswer"))
        self.assertEqual(json.loads(cases[0][3]), {"type": "pong", "version": ""})
        self.assertFalse(cases[0][5], "a branch with fields is not a unit variant")

    def test_a_branch_carrying_nothing_but_its_tag_is_a_unit_variant(self):
        # typify emits a unit variant for these, and the sweep's match arm has
        # to be written without a field pattern or it will not compile.
        document = result_document()
        _, named = split(document)

        cases = collect_result_cases(document, named)

        self.assertTrue(cases[1][5], "a tag-only branch is a unit variant")

    def test_the_mistagged_fixture_wears_the_next_variants_tag(self):
        # Every other field stays correct, so the only reason to refuse it is
        # the tag. That is what makes the refusal a test of the discriminator.
        document = result_document()
        _, named = split(document)

        cases = collect_result_cases(document, named)

        self.assertEqual(json.loads(cases[0][4]), {"type": "ok", "version": ""})
        # The last row wraps round to the first, so every row has one.
        self.assertEqual(json.loads(cases[1][4]), {"type": "pong"})

    def test_a_union_of_one_stops_the_pipeline(self):
        document = result_document(branches=[result_branch("ok")])
        _, named = split(document)

        with self.assertRaises(DriftError) as caught:
            collect_result_cases(document, named)
        self.assertIn("at least two", str(caught.exception))


class ResultRendering(unittest.TestCase):
    def rendered(self):
        document = result_document()
        _, named = split(document)
        return render_results(collect_result_cases(document, named), "// header")

    def test_the_sweep_carries_a_row_an_arm_and_a_check_per_variant(self):
        rendered = self.rendered()

        self.assertIn('("pong", "Pong", r#"{"type": "pong", "version": ""}"#),', rendered)
        self.assertIn('ResponseResult::Pong { .. } => "Pong",', rendered)
        self.assertIn('mirrors!(checked, PongAnswer, "pong",', rendered)
        self.assertIn("CASES.len(), 2", rendered)

    def test_a_unit_variant_gets_a_pattern_with_no_fields(self):
        # `ResponseResult::Ok { .. }` does not compile against a unit variant.
        rendered = self.rendered()

        self.assertIn('ResponseResult::Ok => "Ok",', rendered)
        self.assertNotIn("ResponseResult::Ok { .. }", rendered)

    def test_every_named_type_is_imported(self):
        rendered = self.rendered()

        self.assertIn("    ResponseResult,", rendered)
        self.assertIn("    PongAnswer,", rendered)
        self.assertIn("    OkAnswer,", rendered)

    def test_a_fixture_that_would_break_a_raw_string_stops_the_pipeline(self):
        case = ("pong", "Pong", "PongAnswer", '{"x":"\\"#"}', "{}", False)
        with self.assertRaises(DriftError) as caught:
            render_results([case], "// header")
        self.assertIn("raw string", str(caught.exception))

    def test_a_mistagged_fixture_that_would_break_a_raw_string_stops_the_pipeline(self):
        case = ("pong", "Pong", "PongAnswer", "{}", '{"x":"\\"#"}', False)
        with self.assertRaises(DriftError) as caught:
            render_results([case], "// header")
        self.assertIn("raw string", str(caught.exception))


class Rendering(unittest.TestCase):
    def test_the_rendered_sweep_carries_a_row_and_an_arm_per_method(self):
        rendered = render([("ping", "Ping", "{}")], "// header")
        self.assertIn('("ping", "Ping", r#"{}"#),', rendered)
        self.assertIn('RequestMethod::Ping(_) => "Ping",', rendered)
        self.assertIn("CASES.len(), 1", rendered)

    def test_params_that_would_break_a_raw_string_stop_the_pipeline(self):
        with self.assertRaises(DriftError) as caught:
            render([("ping", "Ping", '{"x":"\\"#"}')], "// header")
        self.assertIn("raw string", str(caught.exception))


class WritingRust(unittest.TestCase):
    """Everything the pipeline writes has to survive `cargo fmt --check`."""

    def test_written_rust_comes_back_formatted(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sample.rs"
            write_rust(path, "fn   main ( )   {\nlet    x=1;\n}\n")
            self.assertEqual(path.read_text(), "fn main() {\n    let x = 1;\n}\n")

    def test_rust_that_will_not_parse_stops_the_pipeline(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "broken.rs"
            with self.assertRaises(DriftError) as caught:
                write_rust(path, "fn main( {\n")
            self.assertIn("rustfmt rejected", str(caught.exception))
            # The bad source stays on disk so it can actually be read.
            self.assertTrue(path.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
