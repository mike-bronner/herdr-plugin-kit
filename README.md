# herdr-plugin-kit

Shared building blocks for [Herdr](https://github.com/herdrdev/herdr) plugins.

Three published Herdr plugins each hand-maintain the same socket client, the same
environment loader, and the same build shims. One has no `--version` at all. Two send
toasts and throw away the answer that says whether the toast arrived. This kit ends the
duplication, and turns a Herdr release into an ingestion step rather than a manual patch
across three repositories.

`SCOPE.md` is the full specification. Read it before changing anything here.

> **Not a plugin.** Do not apply the `herdr-plugin` GitHub topic to this repository.
> That topic feeds the marketplace index, and the kit is not installable.

## Status

Early. The kit currently carries the **wire types only**, generated from Herdr's own
published API schema. The transport, environment, version, report, and update modules,
the shell templates, and the CI workflows land in later stages, in the order `SCOPE.md`
section 13 sets out.

| Piece | State |
|---|---|
| `api::generated` — 102 request methods, 187 schema types | ✅ generated and committed |
| `api::Request` — the hand-written envelope | ✅ |
| `api::client` — transport | ⏳ later stage |
| `env`, `version` | ⏳ later stage |
| `report`, `update` | ⏳ held until one real consumer proves the boundaries |
| Shell templates, CI workflows | ⏳ later stage |

Generated against **Herdr `v0.9.0`**, protocol 22, schema version 1.

## Requirements

Rust **1.80** or newer. That is a raise from the 1.75 the three plugins declare today,
and it is not a preference: `cargo-typify` emits `std::sync::LazyLock` for every
pattern-constrained string in Herdr's schema, and that landed in 1.80. The generated
file is never hand-edited, so the floor moves with it.

Nothing else. The crate depends on `serde`, `serde_json`, and `regress`, and it carries
no build script, no build dependencies, and no proc macro of its own.

## Windows is compile-verified only

Windows ships in the first release: the PowerShell shims and all six target triples,
including `aarch64-pc-windows-msvc`. Both decisions are recorded in `SCOPE.md` sections
10.1 and 14.

**Nobody on this project has Windows hardware.** CI proves the code compiles on Windows.
It never proves a plugin runs there. Every Windows path in this repository — the
PowerShell shims when they land, and the transport that has to speak named pipes rather
than Unix sockets — is verified by the compiler and by nothing else.

That is stated here rather than filed as a deferral, because a caveat in a deferral
disappears the moment the deferral is closed. Treat a Windows bug report as new
information, not as a regression.

## Using it

Pin to a tag:

```toml
[dependencies]
herdr-plugin-kit = { git = "https://github.com/mike-bronner/herdr-plugin-kit", tag = "0.1.0" }
```

Build a request:

```rust
use herdr_plugin_kit::api::{generated::{PaneListParams, RequestMethod}, Request};

let request = Request {
    id: "pick-project-1".to_string(),
    method: RequestMethod::PaneList(PaneListParams { workspace_id: None }),
};

// {"id":"pick-project-1","method":"pane.list","params":{}}
let wire = serde_json::to_string(&request).unwrap();
```

That example is a doctest in `src/api/mod.rs`, so the wire format above is checked by
`cargo test` rather than asserted here.

The kit also exports what the types were generated against, so a plugin can compare its
expectations with a live server:

```rust
use herdr_plugin_kit::api::{GENERATED_FOR_HERDR_TAG, GENERATED_PROTOCOL};
```

A protocol mismatch is a diagnosis, never a hard failure. A plugin that still works must
keep working.

## The generated layer

`crates/herdr-plugin-kit/src/api/generated.rs` is machine-written and committed. **Never
edit it.** Any hand edit is destroyed by the next regeneration, silently and without a
conflict.

It is committed rather than generated at build time on purpose. `typify::import_types!`
would make `typify` a dependency of every plugin that consumes the kit, which is the
cost this arrangement exists to avoid.

`api/envelope.rs` holds the one hand-written type in that layer, and the reason it has to
exist is worth knowing before you touch the pipeline:

> `cargo-typify` 0.8.0 abandons the serde discriminator when a `oneOf` has a sibling
> top-level property, and Herdr's request schema puts `id` beside its `oneOf`. The enum
> comes out untagged as `Variant0`..`Variant101`, and all 102 methods deserialize to
> `Variant0`. **The broken output still round-trips JSON perfectly**, because the method
> name survives as an opaque string, and it fails open by accepting invented methods and
> missing required params.

So the pipeline lifts `id` out before generation and `envelope.rs` puts it back around
the outside. It names no method, no variant, and no params shape, which is what lets it
survive every regeneration untouched.

**Never read a green round-trip test as evidence that these types are correct.**
`tests/method_sweep.rs` is the test that actually discriminates: it sweeps all 102
discriminators and asserts each reaches its own named variant.

## Regenerating

```sh
just sync-api v0.9.0
```

Or, where `just` is not installed:

```sh
python3 codegen/sync_api.py v0.9.0
```

Both run the same module. Four stages: fetch the schema at that tag, extract the five
sub-schemas into one document, lift the request envelope's `id`, and generate. Output is
byte-identical across runs for a given tag and a given `cargo-typify`.

Regenerating needs `cargo-typify` 0.8.0 (`cargo install cargo-typify`), Python 3.9 or
newer, and network access. **Building the crate needs none of the three.**

A human reviews the diff before it lands. That is the whole review gate for a Herdr
upgrade, so it is not a formality.

### When the pipeline stops

It is built to stop rather than guess. Three schema changes halt it on purpose, each
with a message naming what changed and what to do:

- a `$ref` that points outside its own sub-schema, which would mean Herdr had started
  sharing definitions across sub-schemas
- a type name defined twice with two different bodies, which is either a shared type
  changed on one side only or a new type that took a taken name
- a change to the request envelope's `id` property, which `envelope.rs` hand-writes and
  therefore cannot track by itself

Every one of those would otherwise produce Rust that compiles and is quietly wrong.

## Layout

```
crates/herdr-plugin-kit/    the runtime crate
codegen/                    the four-stage pipeline and its tests
SCOPE.md                    the specification
justfile                    task wrappers, all one line each
```

## Testing

```sh
just test          # or: python3 codegen/test_codegen.py && cargo test
just check         # formatting, lints, and both suites
```

The codegen guards have their own suite because an untested guard is a claim rather than
a check. It needs no network and nothing beyond the standard library.

## Licence

MIT. See `LICENSE`.
