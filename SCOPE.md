# herdr-plugin-kit — Specification

**Status:** partly built. `api`, `env`, and `version` have landed, which is the whole of
what §13 ships. The transport, the shell templates, and CI have not.
**Date:** 2026-09-10, corrected and extended 2026-09-11 (§15.1).
**Repo:** `mike-bronner/herdr-plugin-kit`, public, under the `mike-bronner` GitHub organization.

**Verified against:** Herdr 0.9.0, API protocol 22, schema_version 1, `cargo-typify` 0.8.0, rustc 1.97.0, cargo 1.97.0.

Facts marked ✅ were measured. Everything else is design intent.

---

## 1. Purpose

Three published Herdr plugins each hand-maintain the same socket client, the same
environment loader, and the same build shims. One has no `--version` at all. Two send
toasts and throw away the answer that says whether the toast arrived. This kit ends the
duplication and turns a Herdr release into an ingestion step rather than a manual patch
across three repositories.

**In scope:** wire types, transport, environment, version reporting, error popups, update
checking, distribution shims, CI.

**Out of scope:** anything specific to one plugin's behaviour.

⚠️ **Do not apply the `herdr-plugin` GitHub topic to this repo.** That topic feeds the
marketplace index, and the kit is not an installable plugin.

### 1.1 Measured duplication this replaces

✅ Re-measured 2026-09-10, after two of the three cut a release. Five figures moved since
this table was first written, which is the point of the table rather than a caveat on it.

| Concern | project-finder | recent-spaces | agentic-panes-layout |
|---|---|---|---|
| `api.rs` | 223 lines | 158 | 302 |
| `config.rs` | 402 lines | 275 | 493 |
| `--version` | absent | `version.rs`, 151 lines | ~90 inline in `main.rs` |
| `build.rs` stamp | absent | 95 lines | 197 lines |
| Error popup | `notification.show` | absent | `issues.rs`, 204 lines |

✅ `find_cargo()` is byte-identical in **two** of the three `bin/build` scripts, and
`needs_build()` is byte-identical in **two** of the three launchers. recent-spaces holds
the odd copy of each, because it moved `find_cargo` out into its own `bin/find-cargo`.
Two identical copies plus one that has drifted is the duplication problem at its next
stage, not the absence of one.

#### The claim this table used to make, withdrawn

🚨 This section said that project-finder's error path runs on `notification.show`,
"which agentic-panes-layout's own measurements proved does not fire". **That is
retracted. The claim was false.** §7.1 carries the correction, the measurement that
overturned it, and how it came to be written.

✅ The real defect is smaller, and it is duplicated rather than unique. Both plugins that
send a toast discard the whole response: `let _ = client.call("notification.show", ...)`
at `project-finder/src/api.rs:218` and `agentic-panes-layout/src/api.rs:297`, literally
that. recent-spaces sends none, so this is two of three rather than all three.

So a dropped toast and a delivered one are indistinguishable to the caller, from
information the caller already received. §7.2 is what the kit does about it.

---

## 2. Repository layout

The target layout. Built so far: `codegen/`, the `api` module, `env.rs`, `version.rs`,
and the whole of `crates/herdr-plugin-kit-build/`. Still to come: `client.rs`,
`templates/`, and `.github/workflows/`. `report.rs` and `update.rs` are held by §13
rather than merely pending.

```
herdr-plugin-kit/
├── Cargo.toml                        # workspace root
├── crates/
│   ├── herdr-plugin-kit/             # runtime crate
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── api/
│   │   │   │   ├── mod.rs
│   │   │   │   ├── generated.rs      # committed, never hand-edited
│   │   │   │   ├── envelope.rs       # the hand-written Request wrapper
│   │   │   │   └── client.rs         # transport
│   │   │   ├── env.rs
│   │   │   ├── version.rs
│   │   │   ├── report.rs             # feature: "report"
│   │   │   └── update.rs             # feature: "update"
│   │   └── tests/
│   │       └── method_sweep.rs       # generated, the 102-discriminator sweep
│   └── herdr-plugin-kit-build/       # build-dependency crate only
├── codegen/
│   ├── sync_api.py                   # the driver: fetch, extract, lift, generate
│   ├── extract.py                    # stage 2, and the ref and collision guards
│   ├── lift_envelope.py              # stage 3
│   ├── emit_sweep.py                 # the sweep, derived from the schema
│   └── test_codegen.py               # the guards' own tests, no network
├── templates/
│   └── bin/
│       ├── build            build.ps1
│       └── launcher         launcher.ps1
├── .github/workflows/
│   ├── plugin-ci.yml                 # reusable, workflow_call
│   ├── plugin-release.yml            # reusable, workflow_call
│   └── kit-ci.yml                    # the kit's own
└── justfile
```

Two crates, not one. The build-stamp helper must never become a runtime dependency of
every plugin.

---

## 3. Code generation

### 3.1 Source

✅ `herdrdev/herdr` → `docs/next/api/herdr-api.schema.json`, 275,129 bytes, resolves at
release tags (confirmed at `v0.9.0`). Generated by `schemars` from `src/api/schema.rs`,
the same serde types the server uses on the wire, so it cannot drift from the
implementation.

✅ Envelope keys: `protocol` (22), `schema_version` (1), `title`, `schemas`.

| Sub-schema | Defs | Variants | Generation |
|---|---|---|---|
| `request` | 121 | 102 | 🔧 needs the lift |
| `success_response` | 71 | 64 | ✅ clean |
| `event` | 16 | 26 | ✅ clean |
| `subscription_event` | 10 | 3 | ⚠️ untagged, named, functional |
| `error_response` | 1 | 0 | ✅ clean, and validates nothing (§3.5) |

✅ All 384 `$ref` values are self-referential. No sub-schema references another.

### 3.2 Pipeline

Four stages, run by `just sync-api <herdr-tag>` or by `python3 codegen/sync_api.py
<herdr-tag>` where `just` is absent. Both entry points run the same module, so the
documented one and the tested one cannot drift apart.

1. **Fetch** the schema at the given tag.
2. **Extract** each sub-schema, rewriting `#/schemas/<name>/$defs/X` to `#/$defs/X`, and
   merge the five into one document. ✅ 219 definitions collapse to 183 distinct ones
   with zero conflicting bodies, which is what makes `SplitDirection` one Rust type
   instead of three.
3. **Lift** the top-level `id` out of `request`.
4. **Generate** with `cargo-typify`, then commit the output, along with the
   102-discriminator sweep derived from the same schema.

✅ Runs in 0.79 seconds, deterministic and byte-identical across runs.

**Three guards fail closed**, because each catches a change that would otherwise produce
Rust that compiles and is quietly wrong:

| Guard | Catches |
|---|---|
| Every `$ref` points inside its own sub-schema | Herdr starting to share definitions across sub-schemas, which changes what "merge" means |
| A name defined twice is defined identically | A shared type changed on one side only, or a new type that took a taken name |
| The request envelope's sibling property is exactly `id: string` | `envelope.rs` hand-writing a field the schema no longer declares |

The collision guard is the ref guard applied to type names instead of references, and it
is deliberate rather than a side effect of merging. Its message names the type, both
sub-schemas, and the diff between the two bodies, so the reader can tell a changed shared
type from a new one within seconds.

### 3.3 The lift, and why it is mandatory

✅ `typify` abandons the serde discriminator when a `oneOf` has a sibling top-level
property. Output collapses to an untagged `Variant0`..`Variant101`, and **all 102 methods
deserialize to `Variant0`**.

🚨 **This failure is silent.** The JSON still round-trips, because the method name
survives as an opaque string. It also fails open, accepting invented method names and
accepting `workspace.create` with no params.

**Never accept a JSON round-trip as evidence that these types are correct.** Sweep every
discriminator value and assert the variant.

Removing the single `id` field yields 102 correctly named, correctly tagged variants.
Put it back by hand in `envelope.rs`:

```rust
pub struct Request {
    pub id: String,
    #[serde(flatten)]
    pub method: RequestMethod,
}
```

This is the **only** hand-written type in the generated layer. It names no method, no
variant, and no params shape, so it survives every regeneration untouched.

### 3.4 Rules for generated code

- ✅ **Commit `generated.rs`.** Never use `typify::import_types!`, which would make typify
  a dependency of every plugin.
- ✅ No `build.rs`, no `[build-dependencies]`, no proc macro in the runtime crate.
  Verified building `--offline` with `cargo-typify` renamed off the PATH.
- Generate all 102 methods, not a subset. Selective generation is what produces drift.
- ✅ Generated code's runtime dependencies: `serde`, `serde_json`, `regress`.
- ✅ 29 type names appear in more than one sub-schema with **zero conflicting
  definitions**, so dedup into one shared module is mechanical.
- The file header records the Herdr tag and protocol it came from.

### 3.5 Known generation limits

- ✅ 11 methods have unvalidated params, because Herdr declares `PingParams` and
  `EmptyParams` as bare objects. Upstream gap, not ours.
- `#[serde(flatten)]` buffers through a map, so `deny_unknown_fields` does not work
  through the envelope.
- `subscription_event` does not cross-check event kind against payload.
- 🚨 ✅ **No error code is enumerated anywhere in the schema.** `ErrorBody` declares
  `code` and `message` as bare strings, both required, with no enum, so `generated.rs`
  emits `pub code: ::std::string::String`. **So no error code is generatable, and none
  is checkable.** A claim that some call answers a particular code cannot be verified
  against the published contract, even when the observation behind it is real. §4.2
  carries what follows from this for the transport.

---

## 4. Module: `api`

### 4.1 Constants

```
GENERATED_FOR_HERDR_TAG   // "v0.9.0"
GENERATED_PROTOCOL        // 22
GENERATED_SCHEMA_VERSION  // 1
```

### 4.2 Transport — must be portable

🚨 ✅ **`HERDR_SOCKET_PATH` is a Unix socket on Unix and a named pipe on Windows.**
`UnixStream` is therefore not portable, and the existing plugin code is Unix-only.

**Use `interprocess` 2.4.4**, which abstracts both behind one API. It is the same crate
Herdr itself depends on (`interprocess = "2.4.2"`), with explicit Windows, Linux, and
macOS support under active CI.

Hand-written, and kept in a separate module from the generated types so a regeneration
never threatens it.

- Socket resolution from `HERDR_SOCKET_PATH`, falling back to
  `~/.config/herdr/herdr.sock` on Unix.
- Request ids from an atomic counter, prefixed with the plugin id.
- Read and write timeouts, defaulting to 5 seconds, matching recent-spaces today.
- A typed `CallError` distinguishing connect, timeout, protocol, and server-error cases.

#### 4.2.1 Error codes are hand-maintained, and never generated

§3.5 measured the reason: the schema enumerates no error code at all, so nothing here
can be generated, and nothing here can be checked against the published contract.

✅ **Prefer reading an enumerated field on a success result over matching an error-code
string.** An enumerated field generates as a real Rust enum, so an unknown value fails
to deserialize rather than falling through a match. It also survives regeneration two
ways a string match does not: a changed value lands in the diff a human has to read
(§12), and an exhaustive `match` on the enum stops compiling. A string match absorbs the
same change in silence.

⚠️ The three codegen guards (§3.2) do **not** cover this. None of them watches enum
values, and adding one is not proposed here. The compiler and the diff review are the
whole of the protection.

`NotificationShowReason` (§7.2) is the worked example. It answers "did this message
land?" from a success result, without matching one string.

Where an error code genuinely has to be matched, that list is **hand-maintained beside
the transport, never in the generated layer**, and every entry carries the measurement
that put it there: the call, the Herdr version, and what came back. ⚠️ An entry with no
measurement beside it is a claim rather than a check, and nothing in the pipeline can
tell the two apart.

### 4.3 The protocol handshake

✅ `ping` returns a required `version` string and a required `protocol` integer, plus
optional `capabilities`.

The kit compares the live server's protocol against `GENERATED_PROTOCOL` and surfaces a
mismatch as a diagnosis. **No plugin can detect this today.** A wire-format change
currently appears as a confusing parse failure.

A mismatch is a warning, never a hard failure. A plugin that still works must keep
working.

---

## 5. Module: `env`

Promoted from project-finder's `config.rs`, the two-thirds identical across all three.

- `Environment` with `from_process`, `from_pairs`, `get`, `home`.
- `parse_env_file` and `read_env_file`, preserving the existing malformed-line skip.
- Named constants for the eight injected variables a plugin actually reads:
  `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH`, `HERDR_CONFIG_PATH`, `HERDR_PLUGIN_ROOT`,
  `HERDR_PLUGIN_CONFIG_DIR`, `HERDR_PLUGIN_STATE_DIR`, `HERDR_PLUGIN_EVENT`,
  `HERDR_PLUGIN_EVENT_JSON`.

Plugin-specific config parsing stays in the plugin. Only the shared mechanism moves.

### 5.1 Amended 2026-09-10: why `env` exists, and what it holds

**Decided by Mike, after measuring every variable against the API rather than assuming.**

The challenge was fair: why load environment variables at all, when Herdr has an API?

✅ **Measured answer: the API can replace 2 of 8, and both replacements are circular.**
Every property name in the 275 KB schema matching `dir`, `path`, or `root` was searched.

| Variable | In the API? |
|---|---|
| `HERDR_SOCKET_PATH` | 🔌 impossible by construction — you need the socket to call the API |
| `HERDR_PLUGIN_ROOT` | ⚠️ `plugin.list` → `plugin_root`, but see below |
| `HERDR_PLUGIN_CONFIG_DIR` | ❌ no such field anywhere |
| `HERDR_PLUGIN_STATE_DIR` | ❌ no such field anywhere |
| `HERDR_CONFIG_PATH` | ❌ no such field anywhere |
| `HERDR_BIN_PATH` | ❌ no such field anywhere |
| `HERDR_PLUGIN_EVENT_JSON` | ❌ per-invocation payload, not queryable |

✅ `PluginListParams` is `{"plugin_id": ["string","null"]}`. There is no "self" concept,
and omitting the filter returns every installed plugin. **To find its own entry a plugin
must already know its `plugin_id` or `plugin_root`, both of which come from the
environment.**

Availability seals it. §6.2 requires that nothing in `version` may fail, and `--version`
must print when the socket is down, which is exactly when it gets run. Routing it through
the API inverts that guarantee.

So `env` is not a settings loader. It is the reader for Herdr's **launch contract**. The
API serves shared server state; the environment carries per-invocation facts only the
launching process knows. Different data, not duplicate data.

**§5 was wrong in both directions, and is corrected above:**

- ❌ Five constants read by **no** plugin anywhere, cut: `HERDR_ENV`, `HERDR_PLUGIN_ID`,
  `HERDR_PLUGIN_CONTEXT_JSON`, `HERDR_PLUGIN_ACTION_ID`, `HERDR_PLUGIN_ENTRYPOINT_ID`.
- ➕ One real variable was missing, added: `HERDR_CONFIG_PATH`, read by project-finder
  (`config.rs:276`) and agentic-panes-layout (`config.rs:123`).
- 📉 Twelve constants become eight.

✅ Re-measured across all three plugin repositories, counting files that name each
variable as a string literal in Rust, or anywhere under `bin/`:

| Variable | Rust files | Shell files |
|---|---|---|
| `HERDR_SOCKET_PATH` | 12 | 0 |
| `HERDR_PLUGIN_ROOT` | 10 | 7 |
| `HERDR_PLUGIN_CONFIG_DIR` | 8 | 0 |
| `HERDR_CONFIG_PATH` | 5 | 0 |
| `HERDR_PLUGIN_EVENT_JSON` | 5 | 0 |
| `HERDR_PLUGIN_STATE_DIR` | 3 | 0 |
| `HERDR_BIN_PATH` | 2 | 2 |
| `HERDR_PLUGIN_EVENT` | 1 (a test, setting it) | 1 |
| *the five cut* | 0 | 0 |

⚠️ `HERDR_PLUGIN_CONTEXT_JSON` was **not** on the original list of four to cut. It has to
be, or the count does not reach eight, and the measurement above puts it with the other
zero-reader variables rather than with the readers.

**Four `Environment` methods have exactly one consumer each and stay in project-finder:**
`expanduser`, `overridden`, `overlaid`, `without_plugin_vars`. It uses them to build a
child-process environment (`app.rs:62`, `app.rs:80`, `layout.rs:205`). This is the same
one-consumer bar §13 already applies to hold back `report` and `update`. Promoting them
anyway would have contradicted the document's own rule.

**Kept**, each with two or more consumers: `from_process`, `from_pairs`, `get`, `home`,
`parse_env_file`, `read_env_file`. `from_pairs` earns its place as the test seam that
avoids mutating process globals, which matters more once these crates leave edition 2021
and `set_var` becomes unsafe.

Full reasoning: `decisions/2026-09-10-herdr-plugin-kit-shared-crate.md`.

---

## 6. Module: `version`

Promoted from recent-spaces, which has the best of the three implementations.

### 6.1 The macro requirement

⚠️ `env!` resolves in whichever crate it is written in. A kit function calling
`env!("CARGO_PKG_VERSION")` captures **the kit's** version, not the plugin's. Get this
wrong and every plugin reports the kit's commit as its own.

| Piece | Where | Does |
|---|---|---|
| `herdr-plugin-kit-build::stamp()` | plugin's `build.rs` | emits `HERDR_PLUGIN_COMMIT`, `HERDR_PLUGIN_BUILT` |
| `version_report!()` macro | expands in plugin crate | captures the plugin's own `env!` values |
| `version::report()` etc. | kit, plain functions | formats, reads manifest, computes staleness |

Fixed variable names for every plugin. No per-plugin prefix parameter.

⚠️ **The claim cannot be tested from inside the kit.** Expanded there, the kit *is* the
calling crate, so `env!("CARGO_PKG_VERSION")` answers the kit's version whether the
design is right or wrong. ✅ The test therefore compiles a real consumer crate declaring
`9.9.9`, runs it, and asserts the kit's own version never appears in its report.

✅ **A plugin whose `build.rs` never calls `stamp()` still compiles and still reports.**
The macro reads the two stamp variables with `option_env!` rather than `env!`, so an
absent stamp becomes the word `unknown` instead of a compile error. That is §6.2's rule
applied one level earlier than it looks like it should be.

#### The stamp's pathspec

`stamp()` asks `git status --porcelain` about this list and nothing else:

```
src build.rs Cargo.toml Cargo.lock .cargo rust-toolchain rust-toolchain.toml
```

🔑 **The same list is `bin/build`'s releasable check (§9.4.1)**, and §9.4.1 carries why
both toolchain names have to appear. The two answer the same question about the same
tree, so they cannot be allowed to disagree.

⚠️ Watched for a rebuild **only where the path exists**. ✅ Measured: a
`cargo:rerun-if-changed` pointing at an absent path rebuilds on every invocation, which
would move the build instant under a binary that never changed. ✅ Also measured: `git
status` exits 0 for a pathspec matching nothing, so naming a file the plugin does not
have costs nothing on the status side.

### 6.2 Behaviour

Keeps the existing format: crate version, commit with `-dirty` or `-unverified` marker,
build timestamp, manifest version, and the `STALE:` line when the two disagree. §6.3's
provenance line joins them.

```text
watch 0.5.0 (a1b2c3d, built 2026-09-11T04:39:22Z)
manifest 0.5.0 at /p/herdr-plugin.toml
built from source on this machine
STALE: this binary is 0.5.0 but the manifest is 9.9.9. Rebuild it with `cargo build --release`.
```

🔑 **The first three lines are unconditional, and the fourth is the only verdict.** A
report with a fact line missing would be ambiguous between "fine" and "could not tell",
which is the failure this whole module exists to remove.

**Nothing here may fail.** Every lookup degrades to a word. Concretely:

| Lookup | When it cannot answer |
|---|---|
| The commit and build instant | `unknown`, and an empty value counts as absent |
| The manifest | names the path and what went wrong: unreadable, unparsed, or no version key |
| No plugin root at all | says so, and says which variable would fix it |
| Staleness against a manifest it could not read | ⚠️ **no verdict**, because unknown is not agreement |
| Provenance | see §6.3 |

### 6.3 Provenance

The report also states **how this binary arrived**: fetched from a named release asset,
or built locally at a given time. This is what answers "did the download actually work?"
without inferring it from timing.

Provenance is a runtime read of a file the fetch path writes. It is deliberately **not**
part of the compile-time `version_report!()` macro. A binary can be built once and
shipped, so how it arrived is not something its own compilation can know.

**The note sits beside the binary**, at the binary's own path plus `.download`. Beside it
rather than in the state directory, because the two have to travel together: a note that
outlived the binary it describes would describe the wrong one.

✅ Its format is recent-spaces' existing one, a `KEY=value` file the shell shim writes:

```
version=0.5.0
asset=watch-macos-arm64-6c55e13a5445
sha256=<64 hex characters>
url=https://github.com/.../releases/download/0.5.0/watch-macos-arm64-6c55e13a5445
```

So §5's `parse_env_file` reads it. The shim already writes this shape, and a second
format would be a second thing to keep in step.

🔑 **The note's existence is the fact that decides the remedy**, not its contents. A
thin note, or one that cannot be read at all, still means the binary was fetched, and
whoever installed a published binary has no toolchain. Telling them to run `cargo build`
would be useless. So "could not read the note" never collapses into "compiled here":

| On disk | Reported |
|---|---|
| No note | `built from source on this machine` |
| A note naming the asset and url | `fetched <asset> from <url>` |
| A note that is thin | `fetched`, and says which field it lacks |
| A note that cannot be read | `fetched, and the note beside it could not be read` |
| ⚠️ A **directory** with the note's name | `built from source`, matching the `[ -f ]` the shim asks |

⚠️ **project-finder 0.8.0 fetches without writing a note** (§9.4). Until the template
closes that, its report will say "built from source" for a fetched binary.

---

## 7. Module: `report` (feature-gated)

Promoted from agentic-panes-layout's `issues.rs`.

### 7.1 RETRACTED 2026-09-10: system toasts are not dropped

🚨 **This section said that `notification.show` returns `shown: false` with
`no_foreground_client` under Mike's own `ui.toast.delivery = "system"`, so a plugin toast
never rendered. That claim is withdrawn.** It is withdrawn from §1.1 as well, and from
four files in agentic-panes-layout by commit `46a0023`.

**Measured** 2026-09-10 against Mike's live 0.9.0 server, with
`ui.toast.delivery = "system"` unchanged on disk:

```
$ herdr notification show "agent layout" --body "probe: checking notification delivery"
{"id":"cli:notification:show","result":{"reason":"shown","shown":true,"type":"notification_show"}}
```

✅ `shown: true`, three times: twice independently, and once with the setting confirmed
still in place on disk.

⚠️ **The serving path is unidentified, and no mechanism is asserted here.** Herdr's own
`src/app/api.rs` at v0.9.0 matches the Terminal and System delivery kinds straight to
`NoForegroundClient` with no client check at all, which contradicts the live result. A
separate headless notifications module exists in the same tree, so `app/api.rs` may not
be the handler in force when a client is attached. **Nobody has established which path
serves this call.** Supplying one is what produced the retracted claim.

#### How the wrong claim arose, which matters more than the claim

| Hop | Claim | Evidence for the mechanism |
|---|---|---|
| 0 | `no_foreground_client` for every call on an isolated headless server. **It stated its own limit:** a headless server answers the same way for the other delivery setting, so the setting could not be separated from the absent client | ✅ the measurement itself |
| 1 | It fails because of `ui.toast.delivery = "system"` | ❌ none. The caveat was dropped in relay and the setting asserted as the cause |
| 2 | It fails because no foreground client is present | ❌ none. A different mechanism asserted in its place |
| — | Live measurement, setting unchanged on disk | Kills hop 1. Does **not** establish hop 2 |

Both substitutions were mechanism claims carrying no more evidence than the ones they
replaced. Each read as more careful than the original, because each was more specific.

⚠️ **This failure mode is one-way.** A caveat makes a claim less useful, so every relay is
under quiet pressure to drop one and under none to restore one. Full write-up:
`insights/2026-09-10-caveat-decay-is-one-way.md`.

**The rule this document now follows, in every section it touches: separate measured
behaviour from explanation, and mark the explanation as unverified where nobody has
established a mechanism.** Four documented Herdr claims in one day were right about the
conclusion and wrong about the cause. Each survived its first check, because the check
was aimed at the conclusion rather than at the cause.

### 7.2 What replaces it: return the reason, never assume the outcome

✅ **Measured in the schema.** `notification.show` answers with `shown: bool` and a
`reason` that is a genuine enum of five values. It generates as a real Rust enum,
`generated::NotificationShowReason`:

`shown`, `disabled`, `rate_limited`, `no_foreground_client`, `busy`.

🚨 ✅ Both plugins that send a toast throw that answer away (§1.1). **That discarded
response is the actual defect, and the false claim was hiding it.** It is a silent
failure detectable from information the caller already receives, which is the same shape
as the stale binary `--version` exists to catch.

**So the kit returns the reason rather than a bare success.** `report` hands the caller
the `NotificationShowReason`, and falls back to a pane on this policy.

| `reason` | Fall back to a pane | Why |
|---|---|---|
| `no_foreground_client` | ✅ yes | nothing was there to draw it, so nothing was delivered |
| `rate_limited` | ✅ yes | the message was dropped rather than shown |
| `busy` | ✅ yes | one toast is live at a time, and this was not it |
| `shown` | ❌ no | it was delivered |
| `disabled`, cosmetic message | ❌ no | respected, and see below |
| `disabled`, diagnostic that stops the plugin working | ✅ yes | overridden, and see below |

**Decided by Mike.** ⚠️ **Carry this reason with the rule wherever the rule lands, in code
comments included, because the rule on its own reads like a plugin ignoring a user
preference.** A user who turns off toasts has said something about toasts, not about
diagnostics. So `disabled` is respected for anything cosmetic, and overridden only for a
diagnostic that stops the plugin working.

### 7.3 What still justifies a pane for the detail

Two measurements survive the retraction untouched. Both were taken separately from the
retracted claim, and neither rests on it.

- ✅ **No severity.** Herdr hardcodes every API-originated notification to one kind, so a
  plugin cannot style an error differently from a success.
- ✅ **One at a time.** Only one toast is live, and the next answers `busy`.

⚠️ A rate limit is a third limit, and it has a weaker basis than those two here: what is
established is that the schema declares a `rate_limited` reason the server can return.
That is why §7.2 routes on the reason rather than predicting when it fires.

A pane has none of the three limits. It is the plugin's own terminal, carrying every
issue at once.

⚠️ **project-finder's current error path sends a toast and reads nothing back.** Moving it
onto §7.2 is a real behaviour change, which is why that plugin migrates last.

### 7.4 The pane itself

- Write diagnostics to a uniquely-named temp file, keyed on pid **and** an atomic
  counter. ✅ Pid alone caused a real race that failed the test suite about one run in four.
- Open a popup with `plugin.pane.open`.
- Never wait, and never gate the caller. A cosmetic warning must not block a workspace.
- Failure to open is deliberately silent, because stderr already carries the message.

The kit fixes the convention: entrypoint id `issues`, env vars
`HERDR_PLUGIN_ISSUES_FILE` and `HERDR_PLUGIN_ISSUES_HEADING`. Each plugin declares the
matching `[[panes]]` entry, which a crate cannot supply.

Feature-gated, because recent-spaces is a headless watcher and should not carry popup
machinery.

---

## 8. Module: `update` (feature-gated)

### 8.1 Constraints

✅ No `herdr plugin update` exists.
✅ `[[build]]` runs only on install.
✅ `[[startup]]` fires once per server start and is not a timer.

### 8.2 Design

- **Timer:** a stamp file in `HERDR_PLUGIN_STATE_DIR`, checked at launch against a
  minimum interval. Default 24 hours. No new process for the two on-demand plugins.
  recent-spaces folds the check into its existing poll loop.
- **Check:** `git ls-remote --tags` against the plugin's origin. No auth, no rate limit.
  ⚠️ **Safe here only because of the bullets around it, and wrong on the bootstrap path.**
  §9.6 rejects it for `bin/build` on three grounds. Two of the three apply here as well.

  | Ground for rejecting it in `bin/build` | Applies to this check? |
  |---|---|
  | A network round trip on a hot path, and `bin/build` runs at every server start | ❌ no. This one is on a timer and detached |
  | ⚠️ It can **hang** on an ssh remote, waiting for a passphrase | ✅ yes. A detached spawn that hangs leaks a process, so give it a timeout |
  | ⚠️ It is **wrong for a fork**, whose tag points at different code | ✅ yes. A fork's tag says nothing about the upstream release |

- **Never block a launch.** Spawn detached, write the result to the stamp file, prompt on
  the *next* launch.
- **Prompt:** a popup through `report`, never a toast.
- **Apply:** managed installs re-run `$HERDR_BIN_PATH plugin install owner/repo --yes`,
  the documented refresh path.

### 8.3 Local installs: ask Herdr, and only where the question is live

**Redesigned 2026-09-10.** This section said only that the kit "asks Herdr what kind of
install it is, and does nothing when the answer is local". That is still the intent. What
changed is **where the question gets asked**, and it changed on a measurement.

⛔ ✅ All three of Mike's installs are `local:` links to working repositories. An updater
or a downloader that runs against a working tree is hostile: a developer who asked to
compile silently gets a binary somebody else built.

✅ **Herdr answers the question authoritatively.** `InstalledPluginInfo` carries a
`source`, whose `kind` is the enum `local` or `github`, and it **defaults to `local`**.
Confirmed in `generated.rs`: `PluginSourceKind` holds exactly those two values, and
`defaults::plugin_source_info_kind()` returns `Local`. A default of `local` is the safe
direction, because it sends an unknown install to a compile.

**Decided by Mike: both the check and the developer override belong in the kit**, rather
than being patched into project-finder. He asked project-finder for a development-first
override, then moved the whole thing here. See
`decisions/2026-09-10-local-install-check-belongs-in-plugin-kit.md`.

#### The manifest names the calling context, so the socket does not have to

✅ **`[[build]]` fires only on `herdr plugin install owner/repo`, and never on
`herdr plugin link`.** Documented by Herdr, and measured 2026-09-09 by the
agentic-panes-layout session: a throwaway plugin whose build command wrote a marker file
was linked, and the marker was never created. The plugin registered and its `[[build]]`
entry parsed, so the entry is understood and simply not triggered.

**So during `[[build]]` the install is `github` by construction, and the question already
has a fixed answer there.** `link` is the only route to a `local:` install, and `link`
runs no build command. The question is live only on the `[[startup]]` path, and on a
direct shim invocation.

⚠️ **Do not resolve this by testing whether the socket is reachable. Nobody has measured
that.** What is measured:

| Context | `HERDR_SOCKET_PATH` present? |
|---|---|
| `[[keys.command]]` | ✅ measured present, 2026-09-05 |
| plugin event hook | ⚠️ measured **absent** from the injected set |
| `[[startup]]` | ❓ never measured. Only the absence of `$TERM` is (§10) |
| `[[build]]` | ❓ never measured, and now irrelevant |

**So let the manifest say which context is calling.** The two entries are already
declared separately in every plugin manifest, so the install entry passes a flag the
startup entry does not:

```toml
[[build]]
command = ["sh", "bin/build", "--install"]
platforms = ["linux", "macos"]

[[startup]]
command = ["sh", "bin/build"]
platforms = ["linux", "macos"]
```

The flag names the **context**, not the action, which is the point of the redesign. The
manifest reports where the call came from, and the kit decides what to do about it.

| Situation | What the kit does |
|---|---|
| Install context, the flag is present | ✅ Fetch. The install is `github` by construction |
| Runtime context, `source.kind` is `local` | ✅ Build from source. It is somebody's working tree |
| Runtime context, `source.kind` is `github` | ✅ Fetch |
| Runtime context, the socket cannot be reached | ✅ **Build from source** |

🔑 **An unreachable socket means build from source, which is the safe direction.** A
developer gets the compile they wanted, and a GitHub user compiles once instead of
fetching. So being wrong about socket availability costs a slow compile, never a wrong
binary, and **correctness needs no measurement of the `[[startup]]` environment at all**.
That is what makes this safe to build before anyone measures it.

⚠️ Socket availability at `[[startup]]` remains worth measuring as an **optimisation**
question. It is no longer a correctness one.

Full measurements: `insights/2026-09-10-herdr-build-never-runs-for-local-installs.md`.

---

## 9. Distribution: fetch-or-build

### 9.1 The bootstrap split

The crate cannot hold the whole downloader. The first fetch is what *produces* the
binary, so it cannot live inside it.

| Fetch | Runs | Lives |
|---|---|---|
| Bootstrap | install, and every server start with no current binary | 🐚 shell, `bin/build` |
| Update | plugin already running | 🦀 Rust, `update` module |

⚠️ `bin/build` serves both `[[build]]` and `[[startup]]`, which is why §8.3 has to tell
the two contexts apart, and why §9.6 refuses a network call to resolve a tag.

The kit defines the URL scheme, asset naming, and checksum format as constants. Both
callers read the same convention.

### 9.2 Why prebuilt matters

✅ Herdr runs as a launchd agent with `PATH=/usr/bin:/bin:/usr/sbin:/sbin`. That is the
entire reason `find_cargo()` exists.

✅ `curl`, `tar`, `shasum`, `unzip`, and `git` are all in `/usr/bin`. The fetch path needs
no PATH workaround, and it drops the Rust toolchain requirement for every user.

✅ Under §9.6's raw-binary convention the fetch path uses only `curl`, `shasum`, and
`git`. `tar` and `unzip` are no longer on it, which is two fewer tools that have to be
present for an install to work.

### 9.3 Download is on by default from 0.1.0

**Decided by Mike, overriding a recommendation to ship build-only first.**

The objection was that a silent fallback to a source build makes a fetch that never works
indistinguishable from one that does. That objection is answered by design below, not by
delaying the feature.

### 9.4 Order in `bin/build`

**Rewritten 2026-09-10**, against the shim project-finder actually shipped and against
the §8.3 redesign.

1. **Is a build needed at all?** Stop if the binary is newer than every compiler-read
   path. This is `needs_build()`, and it runs first so that the common case costs no
   network call and no `git`.
2. **Which context is this?** The manifest flag means install, so fetch. No flag means
   runtime, so ask Herdr and build from source when the answer is `local`, or when the
   socket cannot be reached (§8.3).
3. **Is this checkout releasable?** Read `HEAD`, and ask `git status` about the
   compiler-read paths only (§9.4.1). A dirty compiler-read path means build from source:
   no published asset can match this tree.
4. Read `version` from `herdr-plugin.toml` for the release tag, and take the first twelve
   characters of `HEAD` for the asset name (§9.6).
5. Map the host to a platform name (§9.6).
6. Fetch the asset and its `.sha256` over HTTPS.
7. Verify the checksum **before anything executes**, and before the file ever reaches the
   path the launcher execs. ✅ project-finder does this by verifying in a temporary
   directory and moving only on a match. The move is the gate, so no flag or failure mode
   can run an unverified file.
8. Move into place, and record provenance (§6.3).
9. On any failure, fall back to building, and record why (§9.5).

⚠️ **project-finder 0.8.0 does not yet write provenance at step 8**, though recent-spaces
does. The kit's template closes that gap, because §6.3's report has nothing to read
without it.

### 9.4.1 The dirtiness check is narrowed to what the compiler reads

✅ Taken from project-finder's `bin/build`, which asks `git status --porcelain` about one
pathspec and no more:

```
src Cargo.toml Cargo.lock build.rs .cargo rust-toolchain rust-toolchain.toml
```

**An edited README cannot change the binary, and must not force a compile.** The question
being asked is "what was compiled", not "what does `git status` say about the tree".

⚠️ **Listing both `rust-toolchain` and `rust-toolchain.toml` is not redundant.** Git
pathspecs match whole path components, so `rust-toolchain` does not match
`rust-toolchain.toml`. Drop either one and a file the compiler reads goes unwatched.

🔑 **This is the same list the build stamp uses (§6.1).** Both answer "was this binary
built from committed source?", so letting them differ makes the stamp and the fetch
disagree about the same tree.

### 9.5 The fallback is loud, and failure classes differ

| Cause | Falls back | Loudness |
|---|---|---|
| No asset for this platform | yes | quiet, expected |
| Asset 404 | yes | ⚠️ warn — this is a release-process bug |
| Network unreachable | yes | quiet, transient |
| Checksum mismatch | yes | 🚨 loud, delete the file, never cache it |

A checksum mismatch still falls back, because building from the cloned source is safe. It
must never be quiet, and the bad artifact must never be reused.

The state directory records the last attempt: timestamp, URL, and outcome. Repeated 404s
become visible rather than something you have to notice.

### 9.6 Asset convention: keyed on the commit, not on the version

**Corrected 2026-09-10.** This section specified `<bin>-<version>-<triple>.tar.gz` plus a
matching `.sha256`. ✅ **project-finder shipped something different, and it works.** Its
0.8.0 release carries eight assets. Verified by reading the release and all 320 lines of
its `bin/build`:

```
https://github.com/mike-bronner/herdr-plugin-project-finder/releases/download/0.8.0/
  pick-project-macos-arm64-6c55e13a5445        (and .sha256)
  pick-project-macos-x64-6c55e13a5445          (and .sha256)
  pick-project-linux-arm64-6c55e13a5445        (and .sha256)
  pick-project-linux-x64-6c55e13a5445          (and .sha256)
```

**Take that convention.** Three things change from the original text.

| Was | Is | Why |
|---|---|---|
| Keyed on the version | ✅ Keyed on the **first twelve characters of the checked-out commit** | below |
| Target triple | ✅ **Platform nickname**, `macos-arm64` | shorter, and it is what shipped |
| `.tar.gz` archive | ✅ **Raw binary** plus a matching `.sha256` | one file, no extract step, no `tar` |

#### Why the commit, and not the version

🔑 ✅ **The URL itself asserts that the binary was built from the source in this folder.**
A checkout one commit past the tag asks for a file that does not exist, gets a 404, and
compiles. That is the correct outcome, and it needs no comparison logic to reach. A
version-keyed name cannot make the same claim, because two different trees can both call
themselves `0.8.0`.

🚨 It also **removes any network call to resolve a tag**, which matters because
`bin/build` runs at every server start (§8.1, §9.1). `git ls-remote` was rejected on that
path for three reasons, all three of which stand:

- A network round trip on a hot path.
- ⚠️ It can **hang** on an ssh remote, waiting for a passphrase, with nothing to time it
  out.
- ⚠️ It is **wrong for a fork**, whose tag points at different code than the upstream tag
  of the same name.

§8.2 records which of the three still apply to the update check, where the answer differs.

**The release tag stays version-keyed.** ✅ The shipped URL is
`releases/download/<version>/<bin>-<platform>-<commit12>`. So the tag names the release a
human recognises, and the asset name names the tree that produced the binary. §12 governs
the form of that tag, and a stale `v` there is a silent 404.

#### Six platform names, because six triples ship

⚠️ Four shipped. §11.6 builds six, so the naming has to extend past what has been
exercised.

| Target triple | Platform name | Shipped in project-finder 0.8.0 |
|---|---|---|
| `aarch64-apple-darwin` | `macos-arm64` | ✅ |
| `x86_64-apple-darwin` | `macos-x64` | ✅ |
| `aarch64-unknown-linux-gnu` | `linux-arm64` | ✅ |
| `x86_64-unknown-linux-gnu` | `linux-x64` | ✅ |
| `aarch64-pc-windows-msvc` | `windows-arm64` | ❌ never built, never fetched |
| `x86_64-pc-windows-msvc` | `windows-x64` | ❌ never built, never fetched |

⚠️ **The two Windows names are this document's extension of a four-name convention.**
Nothing has produced or consumed them. The `.exe` suffix they raise is open (§14.2), and
it belongs with the PowerShell shims (§10.1), where Windows is compile-verified and
nothing more.

### 9.7 Security limit, stated plainly

🔓 A checksum published in the same release proves the download was not corrupted. It does
not prove the release was not tampered with, because whoever can replace the asset can
replace the checksum. Real authenticity needs GitHub artifact attestations, and verifying
those needs `gh`, which is not on launchd's PATH.

Checksums are the right bar here. **Do not document them as more.**

### 9.8 Rust-side downloads shell out to curl

The plugins have zero HTTP dependencies today, with `opt-level = "s"` and `strip = true`.
Pulling in an HTTP stack to fetch one file per week is a bad trade.

---

## 10. Shell templates

✅ A crate cannot ship `bin/build` or the launchers.

The kit holds them as templates with the plugin name substituted, plus a sync task. That
turns shell drift into a reviewable diff instead of silent three-way divergence.

**Known bug to carry across:** ✅ a Herdr `[[startup]]` command gets no `$TERM` at all.
This was found once and fixed three times. The template fixes it once.

### 10.1 Windows doubles every template

✅ `["sh", "bin/build"]` cannot run on Windows. Herdr supports per-item `platforms` that
override the top-level list, so each shim is declared twice:

```toml
[[build]]
command = ["sh", "bin/build"]
platforms = ["linux", "macos"]

[[build]]
command = ["powershell", "-File", "bin/build.ps1"]
platforms = ["windows"]
```

⚠️ **The earlier sequencing recommendation is withdrawn.** It said to take Windows in the
Rust transport (section 4.2) and the CI matrix (section 11) now, and to defer the PowerShell
shims and the `platforms = [..., "windows"]` declaration in the plugins until someone could
actually run Windows. It was written when Windows was going to be a CI-only target, where the
code is proven to compile and nothing more.

Section 11.6 now ships Windows release assets, and that changes the picture. `bin/build` is
what fetches an asset (section 9), and it cannot run on Windows. A Windows user installs the
plugin, the `sh` shim never executes, and the asset built for that user is unreachable.
Building all six targets while holding the shims produces artifacts nobody can install. The
shims and the Windows assets are one decision, not two, and cannot be taken separately.

**Answered 2026-09-10: the shims land in 0.1.0.** Decided by Mike, alongside keeping all
six target triples. The two were one decision, exactly as the paragraph above argues, so
they were taken together. See §14.

The caveat that survives from the old text is unchanged, and it does not expire with the
decision. **CI proves the code compiles on Windows. It never proves a plugin runs
there**, and Mike works on macOS arm64. That caveat is stated in the README rather than
held as a deferral, because a caveat inside a deferral disappears the moment the deferral
is closed.

---

## 11. CI

### 11.1 Workflows cannot ship inside the crate

🚫 Cargo does not scaffold a consumer's repo, and Actions only runs workflows physically
present in `.github/workflows/`.

### 11.2 Reusable workflows instead

✅ `{owner}/{repo}/.github/workflows/{file}@{ref}` works cross-repo, pins to a tag or SHA,
and nests up to ten levels. Each plugin repo gets a caller of roughly ten lines. Bumping
the pinned ref propagates to all three, the same model as pinning the crate.

✅ `mike-bronner` is a GitHub **Organization**, so `secrets: inherit` is available.

### 11.3 `plugin-ci.yml`

Inputs: crate name, binary name, minimum Rust version.
Jobs: fmt, clippy with warnings denied, test, build matrix, **version agreement**.

### 11.4 The version-agreement gate is load-bearing

Assert that `herdr-plugin.toml`, `Cargo.toml`, and the git tag all state the same version.

It was originally justified by a stale-binary scare. Under download-by-default it also
guards the download path, because a manifest ahead of its latest tag means **every install
404s and silently compiles**.

✅ **The worked example has changed. The gate has not.** This section once named a live
instance: a plugin whose manifest had been bumped while its newest tag was a release
behind, which is this failure exactly. That instance is closed, and §13.1 records
agreement as a yes-or-no property rather than the pair of numbers that produced it.

**The gate stays, for two reasons.** A fixed instance is not a closed class, and this
class went wrong once already. ➕ **The gate also asserts the tag *form*, not only the
version.** §12 records why: a `v` on one side and none on the other 404s and then
compiles, in silence.

⚠️ One window the gate cannot close: between the version-bump commit landing on `main`
and the tag being pushed, `main` advertises an unreleased version. Push the bump and the
tag together, or trigger the release from the bump commit.

### 11.5 `plugin-release.yml`

Tag-triggered. Builds the matrix, strips, generates a `.sha256` beside each binary, and
uploads both. Produces exactly what §9.6 consumes: raw binaries, keyed on the commit, and
**no archive step**. It also asserts the tag form (§12.2), which is what stops the two
conventions crossing at the one place that publishes.

Requires the **caller** to grant `contents: write`. A called workflow runs on the
caller's permissions.

🚧 **Blocking for the kit's own 0.1.0.** Under download-by-default the first migrated
plugin release must produce assets, so the release workflow has to be correct before that
release, not after.

### 11.6 Build matrix — native runners, not cross-compilation

GitHub provides native runners for all three operating systems, so cross-compilation buys
nothing and macOS is the painful case.

| Target | Runner |
|---|---|
| `aarch64-apple-darwin` | `macos-latest` |
| `x86_64-apple-darwin` | `macos-latest` (Apple's own SDK handles it) |
| `aarch64-unknown-linux-gnu` | `ubuntu-24.04-arm` |
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` |
| `x86_64-pc-windows-msvc` | `windows-latest` |
| `aarch64-pc-windows-msvc` | `windows-11-arm` |

**All six targets ship. Decided by Mike, closing an earlier question about dropping
`aarch64-pc-windows-msvc`.** None of the six is optional. Shipping Windows release assets is
what couples the shim question in section 10.1 to this one.

✅ arm64 runners went GA and free for public repos in August 2025, and reached private
repos in January 2026. There is no `ubuntu-latest-arm` alias, only versioned labels.

### 11.7 Trigger on `push` as well as `pull_request`

🪤 ✅ When a PR cannot compute a merge ref against `main`, Actions skips `pull_request`
workflows entirely. No run, no error, and the checks simply never appear. A push trigger
keeps a conflicted branch covered.

---

## 12. Versioning and sync policy

### 12.1 Mike's own tags carry no `v`. Herdr's keep theirs.

**Decided by Mike 2026-09-10**, from project-finder 0.8.0 onward. Git tags and GitHub
release names are the bare semantic version. SemVer's own FAQ is the reason: a
`v`-prefixed string is a **tag name**, and the semantic version is the unprefixed part.
The versions in `herdr-plugin.toml` and `Cargo.toml` were already unprefixed, so the tag
matches them instead of leaning on a convention. Existing prefixed tags are **not**
rewritten. See `decisions/2026-09-10-unprefixed-release-tags.md`.

- ✅ **This kit tags `0.1.0` onward**, not `v0.1.0`.
- ⚠️ **Herdr's release tags keep their `v`, and the schema URL resolves at `v0.9.0`.** So
  `just sync-api v0.9.0` takes the prefix, because that is Herdr's own tag name. **Do not
  let the two conventions cross.**

### 12.2 A stale `v` in an asset URL fails silently

🚨 The fetch 404s, falls back to compiling, and the plugin still works. It just stops
using the prebuilt binary the whole mechanism exists to deliver, and nothing surfaces.
That is the same failure §9.3 was told to answer by design rather than by delay.

Two requirements follow, and both are requirements rather than advice:

- ➕ **Pin the expected asset URL in a test, whole, including the tag form.** ⚠️ A test
  that builds the URL the same way the shim builds it cannot catch a wrong tag form. It
  has to state the expected string.
- ➕ **The release workflow must reject one of the two forms** (§11.5). Tolerating both
  `0.8.0` and `v0.8.0` is exactly how two conventions drift apart and then disagree
  without saying so.

⚠️ **Live instance to carry.** recent-spaces' own `bin/build` builds
`releases/download/v$version/...`, and that repo still tags with a `v` (§13.1). The two
agree, so nothing is broken there yet. It breaks on the **first unprefixed tag that repo
cuts**, and it breaks by compiling instead of fetching. ✅ The shell template hardcodes
neither form: it reads the tag out of the manifest version and prefixes nothing, so a
migrated plugin inherits the unprefixed convention by construction. Fix the repo's own
tag form when recent-spaces migrates (§13).

### 12.3 The rest

- Consumers pin to a tag via a git dependency.
- The kit records the Herdr tag and protocol it was generated against.
- `just sync-api <tag>` refetches and regenerates. **A human reviews the diff before it
  lands.**
- A protocol change is a minor bump at minimum.
- Promotion to crates.io stays open and needs no design change.

---

## 13. Migration

| Order | Plugin | Why |
|---|---|---|
| 1️⃣ | recent-spaces | Smallest at 158 lines of `api.rs`, no TUI, donates the best `version.rs`, and carries the live `v`-prefix hazard (§12.2) |
| 2️⃣ | agentic-panes-layout | Donates `issues.rs` |
| 3️⃣ | project-finder | Only one with a real behaviour change (§7.3), and donates the shim (§9.4) |

**Ship the kit with `api`, `env`, and `version` only.** Hold `report` and `update` until
one real consumer has proven the boundaries. Designing abstractions with no consumer is
how they come out wrong.

### 13.1 Release state: the properties, never the numbers

🚨 **This section used to carry a version column, and it is gone on purpose.** It held
each plugin's manifest version and its latest release tag, freshly measured. One of those
numbers was wrong the following day, which is what a table of current values in a
document nobody re-measures on a schedule always becomes. **Being wrong in a
specification is worse than being silent**, because a reader trusts it.

So this records only what the design turns on, and every cell is a yes or a no. Somebody
re-measuring in three months changes an answer, never a number.

| Repo | Publishes assets | Tag form | Manifest agrees with its latest release |
|---|---|---|---|
| project-finder | ✅ yes | ✅ unprefixed | ✅ yes |
| recent-spaces | ✅ yes | ⚠️ `v`-prefixed | ✅ yes |
| agentic-panes-layout | ❌ no | ⚠️ `v`-prefixed | ✅ yes |

Last measured 2026-09-11. **Re-measure rather than trust it.**

- ✅ **Agreement is a property, not a number.** It is what §11.4's gate asserts, and the
  gate is unchanged by any release either side of it cuts.
- ⚠️ **A repo publishing no assets cannot be retrofitted.** Its first migrated version has
  to be a new release cut through the new workflow, because there is nothing to attach
  commit-keyed assets to retrospectively.
- ⚠️ **Two of the three tags still carry a `v`.** §12 governs which form is right, and
  §12.2 records why crossing the two fails silently rather than loudly.

⚠️ **This rule does not reach the Herdr generation tag.** `GENERATED_FOR_HERDR_TAG`, the
tag in every generated file's header, and the tag argument to `sync_api.py` are a
deliberate **pin on an external dependency**, and §12 requires a human to review the diff
every time it moves. That pin is load-bearing precisely because it does not track the
latest. A plugin's own version number is an incidental snapshot of one of Mike's
repositories, and the two are not the same kind of fact.

---

## 14. Questions, answered and open

### 14.1 Answered 2026-09-10 by Mike

| # | Question | Answer |
|---|---|---|
| 1 | PowerShell shims in 0.1.0, or Windows assets shipped unreachable until someone can test them? | ✅ **The shims ship in 0.1.0.** Overrides the recommendation in §10.1 to defer them. Building six targets while holding the shims produces artifacts no Windows user can install, so shipping the assets and shipping the shims are one decision |
| 2 | Keep `aarch64-pc-windows-msvc`, or drop to five targets? | ✅ **All six triples stay.** Overrides the recommendation to drop it. §11.6 records the same decision on the CI matrix |

⚠️ **Neither answer removes the Windows caveat.** Both are shipped compile-verified only,
and nobody on this project has Windows hardware. §10.1 and the README both say so plainly.

### 14.2 Still open

| # | Question | Note |
|---|---|---|
| 1 | Attestation signing later? | Needs `gh`, which is not on launchd's PATH. §9.7 states the limit checksums do and do not cover |
| 2 | Does a Windows asset name carry `.exe`? | §9.6 extends a four-name convention to six. Nothing has produced or consumed the two Windows names, and the launcher execs the path it builds |
| 3 | Is the socket reachable at `[[startup]]`? | ✅ **No longer a correctness question** (§8.3). Purely an optimisation now: measuring it could save a `plugin.list` call |

---

## 15. Provenance of this document

Written 2026-09-10 from a design session that measured, rather than assumed:

- The Herdr API schema shape, its refs, and its method count.
- `typify` behaviour, via a spike with a minimal root-cause reproduction, a 102-method
  sweep, 24 tests, and 16 mutation checks.
- The launchd PATH and which binaries survive it.
- `herdr plugin` subcommands on 0.9.0, and the absence of an update command.
- Reusable-workflow and arm64 runner availability.
- The three plugins' release and asset state.

### 15.1 Corrected 2026-09-10, later the same day

Three sessions working the three donor plugins measured a great deal that contradicted
this document within hours of it being written. **Every correction below was verified
against the schema, a repository on disk, or a live server. None was relayed.**

| § | What changed | Kind |
|---|---|---|
| 1.1, 7.1 | `notification.show` **does** fire. The claim that it does not is retracted, and no mechanism replaces it | 🚨 retraction |
| 1.1 | Five figures re-measured. `find_cargo` and `needs_build` are identical in two of three, not three | 📏 re-measurement |
| 7.2 | The discarded `reason` is the real defect. Mike's fall-back policy replaces the retracted design | 🔧 design |
| 3.5, 4.2.1 | No error code is enumerated in the schema, so none is generatable or checkable | ➕ new limit |
| 8.3, 9.4 | The local-install check moves to a manifest-declared context, because `[[build]]` never runs for a linked install | 🔧 design |
| 9.6 | Assets are keyed on the commit, named for a platform, and are raw binaries | 📏 corrected to what shipped |
| 12 | Mike's own tags drop the `v`. Herdr's keep it | ➕ new convention |
| 11.4, 13, 13.1 | The manifest-versus-release drift is closed, and a second repo now publishes assets | 📏 re-measurement |

⚠️ **Two of these changed real design, not just wording:** §7.2 and §8.3.

**The standing rule that came out of it, and applies to every future edit: separate
measured behaviour from explanation, and mark the explanation as unverified wherever
nobody has established a mechanism.** Four documented Herdr claims in one day were right
about the conclusion and wrong about the cause. Each survived its first check, because
the check was aimed at the conclusion rather than at the cause. §7.1 traces one of them
end to end.

Related vault notes: `decisions/2026-09-10-herdr-plugin-kit-shared-crate.md`,
`decisions/2026-09-10-local-install-check-belongs-in-plugin-kit.md`,
`decisions/2026-09-10-unprefixed-release-tags.md`,
`insights/2026-09-10-typify-drops-discriminator-beside-sibling-property.md`,
`insights/2026-09-10-herdr-build-never-runs-for-local-installs.md`,
`insights/2026-09-10-caveat-decay-is-one-way.md`.

