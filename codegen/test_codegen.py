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

import json
import tempfile
import unittest
from pathlib import Path

from emit_sweep import (
    Unsatisfiable,
    collect_cases,
    minimal_instance,
    render,
    variant_name,
    write_rust,
)
from extract import DriftError, extract
from lift_envelope import LIFTED_ROOT, REQUEST_ROOT, lift


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
