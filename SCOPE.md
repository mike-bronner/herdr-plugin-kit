# herdr-plugin-kit — Specification

**Status:** partly built. `api` including its transport (§4.2, §4.3), `env`, `version`
and the shell templates have landed, plus `dialog` (§7.5), which §13 released from its
one-consumer hold by a deliberate decision. CI has not.
**Date:** 2026-09-10, corrected and extended 2026-09-11 (§15.1, §15.2, §15.3).
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

The target layout. Built so far: `codegen/`, the `api` module including `client.rs`,
`env.rs`, `version.rs`, `dialog.rs`, the whole of `crates/herdr-plugin-kit-build/`,
`templates/`, and `.github/workflows/`. `report.rs` and `update.rs` are held by §13
rather than merely pending. ⚠️ **`dialog.rs` was held by the same bar and was released
from it by a deliberate decision** — §13 records which half of that was evidence and
which was a choice.

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
│   │   │   │   ├── response.rs       # the trait a caller names a result through
│   │   │   │   └── client.rs         # transport
│   │   │   ├── env.rs
│   │   │   ├── version.rs
│   │   │   ├── dialog.rs             # feature: "dialog"
│   │   │   ├── report.rs             # feature: "report"
│   │   │   └── update.rs             # feature: "update"
│   │   ├── examples/
│   │   │   └── preview.rs            # draws every dialog locally, in real colour
│   │   └── tests/
│   │       ├── client.rs             # the transport, against a scripted server
│   │       ├── dialog.rs             # the dialogs, against a fake opener
│   │       ├── method_sweep.rs       # generated, the 102-discriminator sweep
│   │       └── response_sweep.rs     # generated, the 64-result sweep
│   └── herdr-plugin-kit-build/       # build-dependency crate only
├── codegen/
│   ├── sync_api.py                   # the driver, and the five stages in order
│   ├── extract.py                    # stage 2, the ref and collision guards, naming
│   ├── lift_envelope.py              # stage 3
│   ├── split_results.py              # stage 4, one type per response variant
│   ├── emit_sweep.py                 # both sweeps, derived from the schema
│   └── test_codegen.py               # the guards' own tests, no network
├── templates/
│   ├── bin/                          # byte-identical in every plugin
│   │   ├── common           common.ps1   # facts, staleness, asset naming
│   │   ├── find-cargo                    # its own file so a test can blind it
│   │   ├── progress                      # the display seam, sh only
│   │   ├── build            build.ps1
│   │   └── launcher         launcher.ps1
│   ├── sync_bin.py                   # the sync task, and `--check`
│   └── test_templates.py             # the shims' own tests, no network
├── tools/
│   ├── mutate.py                     # the mutation harness, JSON-classified
│   ├── test_mutate.py                # its own tests, including two regressions
│   ├── plugin_gate.py                # the target table and the version rules
│   ├── test_plugin_gate.py           # both sides of every agreement, run
│   └── mutations/
│       ├── client.json               # the transport's 20 mutations
│       └── dialog.json               # the dialogs' 25 mutations
├── .github/workflows/
│   ├── kit-ci.yml                    # the kit's own, fast
│   └── kit-mutation.yml              # the kit's own, slow — §11.8
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
| `success_response` | 71 | 64 | ✅ clean, and split into one type per variant (§4.4) |
| `event` | 16 | 26 | ✅ clean |
| `subscription_event` | 10 | 3 | ⚠️ untagged, named, functional |
| `error_response` | 1 | 0 | ✅ clean, and validates nothing (§3.5) |

✅ All 384 `$ref` values are self-referential. No sub-schema references another.

### 3.2 Pipeline

Five stages, run by `just sync-api <herdr-tag>` or by `python3 codegen/sync_api.py
<herdr-tag>` where `just` is absent. Both entry points run the same module, so the
documented one and the tested one cannot drift apart.

1. **Fetch** the schema at the given tag.
2. **Extract** each sub-schema, rewriting `#/schemas/<name>/$defs/X` to `#/$defs/X`, and
   merge the five into one document. ✅ 219 definitions collapse to 183 distinct ones
   with zero conflicting bodies, which is what makes `SplitDirection` one Rust type
   instead of three.
3. **Lift** the top-level `id` out of `request`.
4. **Split** `ResponseResult`, adding one definition per branch so typify emits a type a
   caller can name (§4.4). ➕ **Added 2026-09-12.** The union is copied rather than
   moved, and comes out of typify byte-identical either way.
5. **Generate** with `cargo-typify`, then commit the output, along with the
   102-discriminator sweep and the 64-result sweep, both derived from the same schema.

✅ Runs in 0.93 seconds, deterministic and byte-identical across runs.

**Seven guards fail closed**, because each catches a change that would otherwise produce
Rust that compiles and is quietly wrong:

| Guard | Catches |
|---|---|
| Every `$ref` points inside its own sub-schema | Herdr starting to share definitions across sub-schemas, which changes what "merge" means |
| A name defined twice is defined identically | A shared type changed on one side only, or a new type that took a taken name |
| The request envelope's sibling property is exactly `id: string` | `envelope.rs` hand-writing a field the schema no longer declares |
| `ResponseResult` is still a `oneOf` | The response side ceasing to be a choice of tagged branches, which every per-variant type assumes |
| Every response branch requires a string `const` tag | A branch that no per-variant type could refuse an answer for, because nothing distinguishes it |
| No derived result-type name is already defined | Herdr taking a name the split stage derives, which would otherwise replace a type the rest of the file refers to |
| Every `format: float` is widened to `double`, and at least one exists | The one place this pipeline disagrees with the published schema silently reverting — see below |

The collision guard is the ref guard applied to type names instead of references, and it
is deliberate rather than a side effect of merging. Its message names the type, both
sub-schemas, and the diff between the two bodies, so the reader can tell a changed shared
type from a new one within seconds.

#### 🚨 3.2.1 The one deliberate divergence from what Herdr publishes

✅ **Measured 2026-09-12 against Mike's live 0.9.0 server, through this kit's own
client.** It is the first defect the transport found by talking to a real Herdr rather
than to a scripted peer.

Herdr's schema declares `"format": "float"` **eight times and `"double"` never**, walked
across the whole document. `cargo-typify` maps `float` to `f32`, correctly. **An `f32`
cannot hold what the server sends:**

| Sent by the server | Read back through the generated type |
|---|---|
| `0.69` | `0.6899999976158142` |
| `0.7` | `0.699999988079071` |

🚨 **Three things make this worse than a wrong number.** Deserialization *succeeds*, so
it is fidelity loss rather than a parse failure and nothing errors or warns. A plugin
that reads a layout and applies it back corrupts every ratio it touches. And it is
data-dependent: `pane.layout`, `pane.edges`, `pane.neighbor` and `layout.export` all
round-tripped clean in the same probe, because the target pane sat on an exact 50/50
split and **0.5 is exactly representable in an `f32`**. Only `session.snapshot`, spanning
twelve workspaces, reached a ratio that was not.

⚠️ **The same defect is already documented from the other side**, in
`agentic-panes-layout/docs/herdr-behaviour.md`: send `ratio` as a JSON double, because
serialising an `f32` widens it on the wire. Weeks old, write side. The kit inherited the
read side without anyone checking.

**So the extract stage rewrites `float` to `double` before generation.** The precedent is
`split_results.py` rewriting `const` to a single-valued `enum` for the same class of
reason: the fix belongs in the schema the generator reads, never in `generated.rs`, which
is machine-written and never hand-edited (§3.4).

✅ **Eight declarations become six widened numbers**, because merging collapses
`LayoutNode` and `PaneLayoutSplit`, each declared in two sub-schemas. Seven of the eight
are named `ratio`; the eighth is `PaneResizeParams.amount`, which is a fractional number
on the same wire and is widened for the same reason.

🔑 **This is the only place the pipeline says something the published schema does not, so
it is a guard rather than a quiet rewrite.** A reader comparing the generated types
against the schema will find `f64` where `float` is declared, and this section is the
answer. The guard has two halves, and the second is the one that matters: a rewrite
matching **nothing** looks exactly like a rewrite working, so the driver refuses a schema
it widened nothing in, and its message carries the good-news reading — if Herdr has
started declaring `double` itself, delete the widening.

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

#### 🚨 The same trap, met a third time — 2026-09-12

`#[serde(untagged)]` is how typify reports every shape it cannot tag, and this project
has now walked into it three times from three directions:

1. the request envelope's sibling `id`, which this section exists for.
2. a response `oneOf` whose branches are `$ref`s, tried while looking for a way to define
   the union in terms of the per-variant types (§3.5).
3. the same `oneOf` written as `allOf` against a `$ref`, and again with a `$ref` carrying
   a sibling tag property. Both produced untagged output too.

**Every one of them compiles, round-trips, and dispatches on shape instead of on the
tag.**

⚠️ **Do not build a guard on a grep for `#[serde(untagged)]`, and the measurement says
why.** ✅ There are **five** in the generated file today: `AgentViewField`,
`AgentViewSortField`, `AgentViewValue`, `PopupSize`, and `SubscriptionEventData`. Four
are unions of scalars the schema genuinely declares that way, and the fifth is the
`subscription_event` limit §3.1 already carries.

🚨 So the attribute is normal here, and **that grep answers green today and answers green
after a regeneration that has collapsed the request enum.** A collapsed `RequestMethod`
would be the sixth occurrence rather than the first, and nothing about the search would
say so.

The sweeps in `tests/method_sweep.rs` and `tests/response_sweep.rs` are what actually
catch it, on the two types where it would matter, without anyone remembering to look.

### 3.4 Rules for generated code

- ✅ **Commit `generated.rs`.** Never use `typify::import_types!`, which would make typify
  a dependency of every plugin.
- ✅ No `build.rs`, no `[build-dependencies]`, no proc macro in the runtime crate.
  Verified building `--offline` with `cargo-typify` renamed off the PATH.
- Generate all 102 methods, not a subset. Selective generation is what produces drift.
- ➕ **Everything new is emitted in the same whole-file pass**, from the schema, by the
  same generator. The per-variant result types (§4.4) are a schema injection rather than
  a transform over typify's output, so nothing hand-written enters the generated layer
  and the union is not rewritten by anything.
- ✅ Generated code's runtime dependencies: `serde`, `serde_json`, `regress`.
- ✅ 29 type names appear in more than one sub-schema with **zero conflicting
  definitions**, so dedup into one shared module is mechanical.
- The file header records the Herdr tag and protocol it came from.

### 3.5 Known generation limits

#### ✅ Measured 2026-09-12: typify cannot be driven to a tagged newtype enum

This is a negative result, and it is written down because it cost a day to establish and
answers a question anyone looking at §4.4 will ask: *why are there two renderings of one
schema branch, rather than a union defined in terms of the per-variant types?*

The shape that was wanted, which is byte-identical on the wire to what typify emits
today, and was verified as such by hand:

```rust
#[serde(tag = "type")]
enum ResponseResult { WorkspaceList(WorkspaceListAnswer), Ok }
```

Four encodings of the same schema were tried against `cargo-typify` 0.8.0:

| Each `oneOf` branch written as | typify emits |
|---|---|
| a `$ref` to a named definition | ❌ `#[serde(untagged)]`, discriminator lost |
| `allOf: [tag object, $ref]` | ❌ `#[serde(untagged)]` |
| a `$ref` with a sibling tag property | ❌ `#[serde(untagged)]` |
| an inline object with a `const` tag (today) | ✅ `#[serde(tag = "type")]`, inline struct variants |

🚨 The three failures are all §3.3's trap, so the alternative was not merely unavailable:
it was the defect this project has already met twice.

⚠️ **One near miss, and the reason it does not generalise.** When **every** branch carries
exactly one non-tag property **under the same key**, typify emits
`#[serde(tag = "type", content = "<key>")]` with newtype variants — the shape wanted,
reached by a different route. ✅ Herdr's branches fail that twice over: **25 of the 64
carry more than one** non-tag property, so no single key could hold them at all, and the
37 that carry exactly one spread across **29 distinct keys**. Nesting every payload under
one key would change the wire format, which is not ours to change.

**So the union stays as typify writes it, and the per-variant types are emitted beside
it.** Reaching the newtype form would mean post-processing the generated Rust, which is a
transform between the schema and the committed output that none of §3.2's guards watch,
over 26,000 lines nobody reads. §3.4's argument applies directly: it would deduplicate
generated code, which nobody maintains, and pay for it with a stage that can silently
drop, merge, or rename a variant.

#### The rest

- ✅ 11 methods have unvalidated params, because Herdr declares `PingParams` and
  `EmptyParams` as bare objects. Upstream gap, not ours.
- 🚨 ✅ **The schema understates its own numbers.** `format: float` eight times,
  `double` never, and the server sends values no `f32` can hold. §3.2.1 carries the
  measurement and the rewrite that answers it. ⚠️ **A schema-derived fixture cannot
  catch this**, which is why the 64-row response sweep did not: `minimal_instance`
  answers `0` for a number, and 0.0 is exactly representable. The regression test uses
  0.69 and lives in `tests/client.rs`, where a real call carries it.
- ✅ **A bare `true` is a legal schema, and Herdr writes one**: `agent_explain`'s
  `explain` property, which typify generates as `serde_json::Value`. The fixture builder
  now has a rule for it (`true` is smallest as `null`, `false` is unsatisfiable). ⚠️ It
  sat unnoticed through six stages because the request side never reaches it, and the
  first thing to walk the response side found it. **Adding a check to a previously
  unchecked half finds old things, not new ones.**
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

### 4.2 Transport — must be portable ✅ **built 2026-09-11**

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

#### What shipped, and the four decisions that were not in the list above

`api/client.rs`. One connection per call, newline-delimited JSON, which is the wire
protocol as measured and as all three donors use it. `interprocess` costs the graph
`libc` and nothing else at run time, and its own declared floor of 1.75 is well under
anything else in the tree (§11.8).

⚠️ **It fails the bar `toml` cleared in §5, and harder than `crossterm` did in §7.5.7:
no donor depends on it, so all three consumers gain a dependency.** Bought anyway, and
not for convenience — without it there is no portable transport to write at all.

**1. Off Unix there is no fallback, and that is deliberate.** 🚨 `interprocess` maps a
filesystem path to a named pipe only when it already starts `\\.\pipe\`, and **refuses
any other path outright**. So the Unix default cannot simply be reused: it would fail as
an opaque name-mapping error far from its cause, and resolution would stay infallible
only by lying. Inventing a pipe name is worse, because the failure then names a path
Herdr never used and reads as plausible. `Socket::resolve` answers `NoSocket` instead,
naming the one variable that fixes it. The signature is a `Result` that can never fire
on Unix, which is the price of not lying on the platform nobody can check.

**2. The answer's id is checked against the request's.** Wire data is untrusted input,
and an answer addressed to somebody else is worse than no answer because it looks like
one. 🔑 Without the check the atomic counter is decoration: one connection per call
already correlates a request with its reply, so nothing verifies the id until something
does.

**3. The timeout is measured, not assumed.** ✅ `set_recv_timeout` existing says nothing
about a timeout firing, and one set on the wrong handle only shows itself against a
wedged server. A server that accepts and then says nothing was stood up and the call
timed. On macOS the timeout arrives as `WouldBlock`; `TimedOut` is the Windows mapping
and is compile-verified only. A failure to *set* the timeout is propagated rather than
discarded — recent-spaces writes `let _ =` there, which leaves the call able to hang
forever in the one situation the timeout exists for.

**4. No second error-code list was built.** See §4.2.1 below.

#### Where the transport is still blind

Nothing here has run against a live Herdr server. The scripted server in
`tests/client.rs` speaks the real wire protocol over a real socket, so the bytes are
genuine, but the peer is not Herdr.

⚠️ **Everything Windows is compile-verified only, and that much was actually done**:
`cargo clippy --all-targets --features dialog --target x86_64-pc-windows-msvc -D
warnings` is clean, 2026-09-11. That type-checks the Windows arms of the default socket,
the timeout classification, and the test harness's own pipe naming. It proves nothing
about behaviour.

#### 4.2.1 Error codes are hand-maintained, and never generated

§3.5 measured the reason: the schema enumerates no error code at all, so nothing here
can be generated, and nothing here can be checked against the published contract.

✅ **Prefer reading an enumerated field on a success result over matching an error-code
string.** An enumerated field generates as a real Rust enum, so an unknown value fails
to deserialize rather than falling through a match. It also survives regeneration two
ways a string match does not: a changed value lands in the diff a human has to read
(§12), and an exhaustive `match` on the enum stops compiling. A string match absorbs the
same change in silence.

⚠️ The six codegen guards (§3.2) do **not** cover this. None of them watches enum
values, and adding one is not proposed here. The compiler and the diff review are the
whole of the protection.

`NotificationShowReason` (§7.2) is the worked example. It answers "did this message
land?" from a success result, without matching one string.

Where an error code genuinely has to be matched, that list is **hand-maintained beside
the transport, never in the generated layer**, and every entry carries the measurement
that put it there: the call, the Herdr version, and what came back. ⚠️ An entry with no
measurement beside it is a claim rather than a check, and nothing in the pipeline can
tell the two apart.

##### ✅ Settled 2026-09-11: the list already existed, so a second one was not built

**The kit matches exactly one error code, and it is `dialog::BUSY_CODE`.** That constant
already sits beside its measurement (§7.5.3), and `dialog::OpenError::from_error` is
already the function that reads it. §4.2's client routes through that rather than
matching `ui_busy` a second time.

🔑 **This clause governs where a list lives, not whether one must exist.** A second list
in `client.rs` with nothing in it would be a structure pretending to be a policy, and
the first entry somebody added to it would have no reason to be there rather than in the
one that already works. Everywhere else the code and message are handed back verbatim in
`CallError::Server`, for the caller to read or ignore.

### 4.3 The protocol handshake ✅ **built 2026-09-11**

✅ `ping` returns a required `version` string and a required `protocol` integer, plus
optional `capabilities`.

The kit compares the live server's protocol against `GENERATED_PROTOCOL` and surfaces a
mismatch as a diagnosis. **No plugin can detect this today.** A wire-format change
currently appears as a confusing parse failure.

A mismatch is a warning, never a hard failure. A plugin that still works must keep
working.

#### The diagnosis is a value, and the plugin decides what to do with it

`Client::ping` answers a `Handshake`. `Handshake::mismatch` answers
`Option<ProtocolMismatch>`, and `ProtocolMismatch`'s `Display` writes the warning line
naming both numbers, so a plugin writes one line and gets the wording for free.

🔑 **Returning the information rather than acting on it is this kit's established
pattern, not a new choice.** §7.2 hands back a notification's delivery reason instead of
assuming an outcome. §6.2 formats a version report and never prints it. §7.5 answers
`Shown` and `Unanswered` and lets the caller act. A client that wrote to stderr itself
would be the first place the kit decided something on a plugin's behalf.

⚠️ **Two concrete costs settled it, beyond the pattern.** Checking automatically inside
an unrelated call would hide a socket round trip there, which is precisely the class of
confusion this section exists to end. And recent-spaces is a headless watcher, so it
would take stderr output it never asked for.

**The accepted cost: a plugin can forget to ask.** Taken rather than engineered around.
Three plugins wording the warning three ways is not a defect — they are three programs
with three voices — and the `Display` implementation means none of them has to invent
the sentence.

⚠️ **A mismatch never fails a call.** `ping` succeeds and the diagnosis rides along
beside the result. A test holds that up specifically, because "warning, not failure" is
the kind of promise that erodes quietly.

### 4.4 One result type per variant ✅ **built 2026-09-12**

`Client::call` is generic over the result the caller names:

```rust
let panes = client.call::<PaneListAnswer>(RequestMethod::PaneList(params))?;
```

🚨 **The cost that started this was measured by the recent-spaces migration, not
inferred.** Its release binary tripled, 979,376 → 3,257,616 bytes, and the decomposition
was the surprising part: serializing all 102 request methods costs **102,368 bytes**, and
deserializing the 64-variant `ResponseResult` costs **1,951,648**. A factor of nineteen,
on the axis nobody was watching. serde generates parsing code per variant and dead-code
elimination cannot drop any, because every one is reachable through the single type.

⚠️ **The size is not the only thing the union was costing, and it is the half that gets
quoted.** ✅ Under the union, recent-spaces' `workspace.move` call discarded its answer:
`{"type":"ok"}` read as a silent success on a call that reorders somebody's sidebar.
Naming the result makes an unexpected answer a `CallError::Protocol` instead. That is
§7.2's discarded-response defect arriving on a second call site, and it is arguably the
better headline than 43.9%.

#### Per variant, never per method

**A method-to-variant mapping is not derivable, and building one by hand is §3.4's blast
radius by another door.** The `$ref` guard in `extract.py` enforces that `request` and
`success_response` are structurally independent, and the schema states nothing about what
a method answers. 🚨 The obvious name heuristic is false on the first plugin that looked:
`workspace.move` answers `workspace_list`, carrying the sidebar after the move, and
`workspace_moved` appears **zero** times in `ResponseResult` and once in `EventData`.

Per variant needs no mapping at all, because the caller names what it expects. That puts
the knowledge at the call site, which is the only place it was ever established, by
measurement, whatever this kit does. ⚠️ If a method-to-variant table is ever written
down, §4.2.1 is its home: documentation beside the transport, never an input to the
emitter, for the same reason error codes live there.

#### What makes naming one type safe

Each generated type carries its own tag as a one-variant enum, so an answer meant for
another variant fails to deserialize and becomes `CallError::Protocol` naming the method
and both tags. 🔑 **The check is in the type rather than in `call`, and the narrowest type
is why**: `OkAnswer` declares nothing but its tag, and without it would accept every
object Herdr can send.

`tests/response_sweep.rs` holds three things up per variant: the discriminator reaches its
own union variant, both renderings of the branch agree on the wire, and an answer tagged
for another variant is refused. ⚠️ **The response side had no sweep at all before this**,
while the request side has had one since the pipeline was written.

⚠️ **Two of the 64 variants are unit variants** — `subscription_started` and `ok` — and
they cost almost nothing in either shape. ✅ Measured: 62 branches carry fields, the
widest being `pane_graphics_info` at 11, `pane_copy_search` at 6, and `worktree_opened`
at 5. **A plugin whose every call answers a bare `ok` saves nothing here**, and many
methods do answer one. The saving is per distinct result shape read, never per method
called.

#### The trade, both halves

| | |
|---|---|
| Generated file | 18,475 → 26,589 lines, **+44%**, compiled by every consumer |
| A consumer's binary | 2,400,048 → 1,393,952 bytes, **−42%**, when it names one result |

✅ **Measured 2026-09-12** on a consumer that resolves a socket, pings, and makes one
call. macOS 27.0 arm64, rustc 1.97.0, release with `opt-level = "s"` and `strip = true`,
no LTO, default codegen-units, kit as a path dependency with default features.

| Rung | Bytes |
|---|---|
| socket resolved, no call | 368,464 |
| ping + `call::<PaneListAnswer>` | 1,393,952 |
| ping + `call::<ResponseResult>` | 2,400,048 |
| the union, plus `{:?}` on it in one arm | 2,470,032 |

**The saving is 1,006,096 bytes**, and it holds at 1,005,568 with the handshake removed,
so it is the call rather than `ping` that moves. It lands 89,792 short of the 1,095,888
a scratch crate bounded it at, which is the right direction: the transport, `regress`,
and the request enum sit in both rungs of a real client.

#### 🚨 A second consumer inverts the headline — measured 2026-09-12

**project-finder narrowed every call site, seven result types, and its binary grew.**
macOS arm64, the same profile settings.

| Build | Bytes |
|---|---|
| before, its own hand-written client | 1,738,592 |
| after, seven narrow types | **2,886,640** |
| after, if any one call had stayed on the union | 3,990,400 |

**Narrowing is worth 1,103,760 bytes, 27.7%**, and it is pinned by tests that answer each
call with a different valid Herdr result and assert refusal. 🚨 **The net against its old
client is +1,148,048, +66.0%.** ✅ The published 0.8.1 asset corroborates the starting
figure to within 17 KB, at 1,755,584, which is a different build rather than a
disagreement.

Attribution, from an unstripped build, by `__text` unless noted: `herdr_plugin_kit`
155 KB, `regress` 124 KB, `interprocess` 5.5 KB, `serde_json` **+185 KB as a jump rather
than a total**, and **426 KB of `__const`** for the generated types' static tables.

#### 🔑 Why the two consumers disagree, and what actually scales

**Seven narrow types do not amortise the way one does.** Each drags its own nested schema
types in, while the shared `__const` tables and `regress` arrive once regardless. So the
cost has two parts:

| Part | Who pays it | Scales with |
|---|---|---|
| A **fixed floor** — the static tables, the regex engine, the transport | every consumer | nothing |
| A **per-type cost** — each named result's nested types | the consumer naming them | how much of the API the plugin touches |

⚠️ **Read the absolute saving, not the percentage.** recent-spaces saved 1,006,096 bytes
and project-finder 1,103,760 — within 10% of each other — because narrowing removes the
same thing both times, the 63 variants nobody named. The percentages differ, 43.9%
against 27.7%, only because the binaries they are fractions *of* differ. 🪤 Two points
make that a weak pattern rather than a law, and nothing here has measured a third.

🚨 **So the two headline figures are each honest and neither generalises.**
recent-spaces names one type, which is the best case, and 43.9% holds for recent-spaces.
project-finder names seven, and 66% growth holds for project-finder. ⚠️ **Nothing
entitles a reader to interpolate between them.** project-finder also links `ratatui`,
`crossterm` and `nucleo` where recent-spaces links none — that stack is in both its
before and its after so it does not explain the delta, but it is one more reason these
are two measurements rather than two points on a line.

❓ **Unmeasured, and worth stating rather than leaving to be assumed.** A consumer naming
most of the 64 would presumably reach a crossover where the union is cheaper than the
types it replaced. **Nobody has measured where that sits**, and nothing suggests it is
near seven.

#### ⚠️ Two measurements, two intervals, and neither is safe alone

Quoting one of these without the other misleads in opposite directions.

| Interval | Effect | Measured on |
|---|---|---|
| kit 0.1.0 → 0.2.0, **union named on both sides** | **+92,944** | recent-spaces, one tree, pin bumped and nothing else |
| within 0.2.0, **union → narrow** | **−1,471,712**, 43.9% | recent-spaces, same tree |
| within this branch, **non-generic → generic call**, union named on both sides | **−331,552** | the kit's own probe below |

🚨 **The first row is what a consumer experiences for doing half the work**, and §13 leads
with it. The 128 new types land whether or not anything narrows.

⚠️ **Nothing here isolates the generic's own cost, and the second and third rows are not
subtractable.** They come from two different programs: one a real plugin, one a probe
built for this. The +92,944 contains the new types, the generic, and everything else
between two tags. recent-spaces refused to claim it had isolated the generic and the
refusal is right; isolating it would need a build of 0.2.0's crate with a non-generic
call, which does not exist and is not worth constructing now that the net figure is the
one a consumer lives with.

⚠️ **These figures carry about ±16 bytes of build-to-build noise**, which is the gap
between the 3,257,616 recorded at the top of this section and the 3,257,600 in §13 for
what is the same build of the same tree. Read the differences, not the last digits.

✅ **`regress` survives narrowing entirely, which the measurement now shows rather than
argues.** `WorkspaceInfo.tokens` keys through a pattern-constrained type, so the regex
engine is reachable from the narrowest result a consumer can name. It is a fixed floor
under every figure here, and the saving is the other 63 variants' visitor machinery
rather than anything shared.

#### ✅ The generic signature has no cost. It pays.

Nobody had measured this, and both sessions that argued about the design expected the
opposite. The same union call is **331,552 bytes smaller** than it was before the client
changed (2,361,008 against 2,692,560, handshake excluded from both). The reason is the
shape the generic required: `Answer` carries the result as an unread `serde_json::Value`
and parses it afterwards into the one type the call named. **Deserializing from a `Value`
generates less code than `from_str` straight into the union.**

⚠️ **So `ping` and the `dialog` transport name narrow types too, and that is
load-bearing rather than tidy.** A `ping` that kept matching the union would instantiate
all 64 variants inside the kit, for every plugin that opens with a handshake, and no
narrow call anywhere else could win it back.

`open_pane` is the one caller that reads nothing, because its promise is that Herdr
accepted the request rather than that a pane appeared. It names a **crate-private**
`AnySuccess`: §7.2 measured a discarded response as the actual defect in two of three
donors, so the kit publishes no sanctioned way to do it. That type accepts a result shape
this build has never heard of, which follows the promise rather than widening it, and
refuses an answer carrying no `type` at all.

---

## 5. Module: `env`

Promoted from project-finder's `config.rs`, the two-thirds identical across all three.

- `Environment` with `from_process`, `from_pairs`, `get`, `home`, ➕ `vars` and
  ➕ `expanduser`.
- `parse_env_file` and `read_env_file`, preserving the existing malformed-line skip.
- Named constants for the eight injected variables a plugin actually reads:
  `HERDR_SOCKET_PATH`, `HERDR_BIN_PATH`, `HERDR_CONFIG_PATH`, `HERDR_PLUGIN_ROOT`,
  `HERDR_PLUGIN_CONFIG_DIR`, `HERDR_PLUGIN_STATE_DIR`, `HERDR_PLUGIN_EVENT`,
  `HERDR_PLUGIN_EVENT_JSON`, ➕ and `PER_PLUGIN_VARS` naming the five of them that
  belong to one plugin.

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

### 5.2 ➕ Added 2026-09-12: what project-finder's migration needed

Three additions, and the first is the one that matters structurally.

#### `vars`, an accessor rather than three methods

project-finder needs three things `Environment` could not express: a filter producing a
child-process environment, a copy with one key overridden, and a copy with pairs merged
underneath. 🔑 **All three were blocked by the same absence — nothing enumerated.** You
cannot copy-with-changes a map you cannot walk.

**Decided by Mike: add the accessor, not the three methods.** Each plugin writes its own
policy as an extension trait over the kit's type. Two reasons, and the second is the
stronger one:

- §13's one-consumer bar stays honest. An accessor on a type the kit already owns is not
  an abstraction with no consumer.
- ⚠️ **It stops the kit adjudicating a shape two plugins genuinely disagree about.**
  recent-spaces resolves its `.env` pairs per key; project-finder wants a copy with pairs
  filled underneath. Neither should be imposed on the other, and a kit that picked one
  would be choosing for a consumer rather than serving it.

It yields `(&str, &str)` so that it composes both ways with no intermediate: straight
into `Command::envs`, which takes `AsRef<OsStr>` pairs, and back through `from_pairs`,
which takes borrowed pairs.

#### `expanduser`, which meets the two-consumer bar without argument

✅ Two independent implementations exist and agree exactly: project-finder's `expanduser`
and agentic-panes-layout's `expand_home`. `~` answers home, `~/rest` answers home joined
with *rest*, everything else comes back unchanged. §1.1's duplication, written twice.

⚠️ **It answers a `PathBuf` where one donor answered a `String`** through
`to_string_lossy`. ✅ Two of that donor's three call sites wrap the result in
`PathBuf::from` immediately, and the lossy step cannot round-trip a path that is not
UTF-8. A caller needing a string converts at its own edge, where the loss is visible.

⚠️ **`~other` is not expanded**, which is the promoted behaviour rather than an omission.
Resolving another user's home needs the password database, which this type deliberately
cannot reach. Unchanged is wrong in a way a caller can see; silently resolving to *this*
user's home is wrong in a way they cannot.

#### `PER_PLUGIN_VARS`, which carries a correctness argument rather than a reuse count

🚨 **project-finder builds a child environment for another plugin's binary** — it hands a
picked workspace to agentic-panes-layout's `bin/agent-layout` — and strips the variables
that would make the child read project-finder's checkout as its own. It strips
`HERDR_PLUGIN_ROOT` and `HERDR_PLUGIN_CONFIG_DIR` and **misses
`HERDR_PLUGIN_STATE_DIR`**, which recent-spaces reads to place a lock file.

🔑 **The argument is not that there is a live bug.** It is that which launch variables
belong to *this* plugin is a fact about Herdr's contract, Herdr writes it down nowhere,
and the first person to answer it got two of three. The second person writing this filter
gets it wrong too, for the same reason.

⚠️ **Unverified, and the list does not rest on it**: nobody has confirmed that Herdr sets
`HERDR_PLUGIN_STATE_DIR` for a pane command at all, so that particular leak may be
theoretical.

🔑 **Named for the condition, not for what a caller does with it.** `without_plugin_vars`
was the donor's name and the donor flagged it as wrong: it says what it removes rather
than why, and **a plugin spawning its own helper wants those variables kept**. They are
true for that child.

➕ **The event pair is included on a fail-closed reading.** A child was not triggered by
the event that triggered this process, so `HERDR_PLUGIN_EVENT` and
`HERDR_PLUGIN_EVENT_JSON` are false for it in the same way the directories are. Stripping
one a caller wanted costs them a line to put it back; leaving one they did not want is
silent.

Plugin-specific config parsing stays in the plugin. Only the shared mechanism moves.

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

✅ **The shell template writes it** (§10), which is what makes this section readable at
all. A fetch path that does not write the note makes its own binary lie: the report says
"built from source" for a binary nobody built here, and then offers a remedy that needs a
toolchain the user does not have. One of the donor shims had that defect, and it is the
clearest single argument for templating these files rather than hand-maintaining three.

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
`insights/2026-09-10-caveat-decay-is-one-way.md`,
`insights/2026-09-11-plugin-pane-open-placement-decides-the-handle.md`.

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

### 7.5 Module: `dialog` (feature-gated) — **built 2026-09-11**

Styled dialogs in four states, in two variants. Promoted from
agentic-panes-layout's `confirm.rs`. ⚠️ **Released from §13's hold by a deliberate
decision, and §13 records which half of that was evidence and which was a choice.**

- **Bare** (`notify`): informs, carries no buttons, dismisses on any key or click, and
  **returns at once**. §7.4's rule, applied: a cosmetic warning must not make somebody
  wait on a dialog to get their workspace.
- **Actioned** (`ask`): a labelable primary button and a labelable cancel, and it returns
  which one the user chose. This one necessarily waits, because the answer is the point.

#### 7.5.1 The placement decides everything, and it was measured

✅ 2026-09-11, isolated 0.9.0 server, rendering captured through a real client in a pty
and machine-counted.

| | `popup` | `overlay` |
|---|---|---|
| Looks like Herdr's own dialogs | ✅ floats, survives across a pane divider | ❌ covers the whole tab, zero rows survive |
| `width` / `height` | ✅ percentages | ❌ `invalid_params` |
| Returns a `pane_id` | ❌ | ✅ |
| In `pane.list` | ❌ | ✅ |
| More than one at once | ❌ `ui_busy` | ✅ stacks |

🔑 **The handle and the dialog look are mutually exclusive, and the look wins.** A popup
is not in the pane tree at all, so `{"type":"ok"}` carries no id because there is no id
to carry. That is structural rather than an omission.

🚨 **So the file channel and the pid marker are load-bearing, not defensive.** There is no
handle to poll and no `pane.list` entry to find, and Herdr has no plugin-to-plugin
channel. The answer travels through a file whose path rides in `plugin.pane.open`'s `env`
map, and the popup writes its own pid before drawing anything — the only evidence a pane
ever appeared, because `plugin.pane.open` answers `ok` either way.

#### 7.5.2 Mouse first, keyboard in full

**Decided by Mike: mouse first**, matching Herdr's own dialogs.

✅ Measured 2026-09-10: a click inside a plugin pane reaches that pane's pty, SGR-encoded
and **rebased to pane-local coordinates**; a click outside is not forwarded; keystrokes
keep arriving throughout. Rebasing is what makes hit-testing a rectangle comparison with
nothing to translate.

⚠️ **That was measured on a plugin pane, not a popup specifically.** Treat click
forwarding into a popup as very likely rather than settled, which is why every answer is
reachable from the keyboard alone.

Enter fires the primary button in **every** state, danger included, matching Herdr's own
delete-worktree dialog. Only the left button activates, and a click on the body resolves
nothing: a misclick must not fire a destructive primary.

#### 7.5.3 `ui_busy` is an outcome, and the two variants differ

🚨 ✅ **The single-popup limit is global, not per workspace.** So an unanswered dialog in
one workspace blocks dialogs in **every** workspace, with nothing on screen to explain it,
and the kit cannot say where the blocker is because a popup has no id and is absent from
`pane.list`. `ui_busy` is a state a user can sit in indefinitely, not a rare race.

- **Bare** falls back to `notification.show` with the same title and body. It needs
  nothing from the user, so another route carries the same message.
- **Actioned** does **not**. A notification cannot collect an answer, and silently turning
  a question into a statement would lose it. `ask` returns `Unanswered::Busy`, and a
  notification separately explains why nothing appeared.

**Both halves are reported.** The `reason` is read rather than discarded, which is §7.2's
rule and its reason: a fallback that fails silently removes the caller's last signal that
anything went wrong. Four of `NotificationShowReason`'s five values mean the user saw
nothing.

#### 7.5.4 The visual design, approved by Mike

Rounded frame in all four states. ⚠️ Varying the corner for danger was proposed and
**rejected**: the glyph is already the non-colour channel, so a second one is redundant.

Two blank rows inside the border at the top, three columns each side, a blank row
separating the body from the buttons, and 🔑 **one** blank row beneath the button row
because it is already visually heavy. The primary renders inverted in the state's colour;
the cancel is plain text; the hovered one gains an underline.

🔑 **The kit draws each button's key**, so `Buttons::new("close anyway", "keep")` renders
`↵ close anyway` and `esc keep`. Labels stay the caller's. Callers typing their own glyph
is how three plugins drift apart on the symbol, which is what this crate exists to end.

➕ **Amended 2026-09-11: the buttons stack when a single row cannot hold both labels
whole**, one per row, each centred, at the cost of one row of height. **Decided by Mike
after the preview showed what the alternative rendered**: at the 24-cell floor the labels
were cut to `↵ reb` and `esc kee`, so "rebuild anyway" and "keep them" both arrived as
fragments. Truncating rather than pushing the row through the right border was right; cut
to three characters was not the point of it.

Two alternatives were rejected. Drawing the keys alone loses the words entirely, and
raising the 24-cell floor means a narrow pane gets no dialog at all rather than a usable
one.

⚠️ **The threshold is measured from the drawn widths, never a constant.** The kit draws
the key affordances and the labels are the caller's, so the question is whether *these
two* buttons and the gap between them fit *this* frame. A width alone cannot answer it:
at 30 cells `Go`/`Stop` share a row and `rebuild anyway`/`keep them` do not.

⚠️ **Truncation survives as the last resort**, for a single label too wide for a row of
its own, which no layout can rescue. At the floor the primary still shortens — but to
`rebuild anyw` rather than `reb`, with the cancel whole.

**The glyph rule, which took four attempts:** one codepoint, no variation selector, East
Asian Width `Neutral`. `U+229D`, `U+2713`, `U+26A0`, `U+2716`.

| Rejected | Why |
|---|---|
| A selector (`U+26A0 U+FE0E`) | Two codepoints, and the exact trigger for Terminal.app drawing one cell and advancing another |
| Emoji (`U+1F535`, `U+2705`) | East Asian Width `Wide`, so two cells |
| ⚠️ `Ambiguous` (`U+24D8`, `U+2299`, `U+25B2`) | **Worse than either**: a terminal *setting* decides the width, and enabling it visibly breaks box drawing |
| Private Use Area | Needs a patched font. This kit is public |

The rule is a **test**, not four pinned literals, and `width() == width_cjk()` is an exact
test for `Ambiguous` because the two functions differ only there.

#### The palette — ✅ **reviewed and approved 2026-09-11**, and still unmatched

✅ **Mike approved all four states**: light blue info at **94**, green at 32, yellow at
33, red at 31. Info moved from 34 to 94 in the same session; 94 is the bright form of the
same basic colour, so it stays inside the eight-plus-eight ANSI set and keeps the property
the palette was chosen for — a themed terminal maps it to whatever blue the user already
picked, rather than to a fixed RGB value.

**What was established, stated narrowly.** Mike looked at rendered output from
`cargo run --features dialog --example preview`, in Terminal.app, under his own theme, and
judged that the four colours read correctly and tell the four states apart. 🔑 That is a
**design review by the person whose tools these are**, which is the right authority for a
palette and is exactly what the original caveat was missing.

⚠️ **Approved is not matched, and the approval does not swallow the open half.** Nobody
has put the kit's blue beside Herdr's own blue in one frame. The probe discarded SGR
attributes, so borders were confirmed present and never confirmed coloured, and **nobody
has established how Herdr colours its own dialogs**. That was true before the review and
is true after it. Inversion is separate again: a terminal attribute rather than a claim
about Herdr, so that part renders as intended regardless.

✅ **The asymmetry was reviewed with the rest, and stands.** Info is bright while success,
warning and danger are not. It was flagged rather than quietly widened, and bright info
against three normal siblings looked correct to the person who asked for it. The argument
for not brightening the other three — that bright red reads as more alarming rather than
better matched, and bright yellow is often the worst cell on a light theme — was never
tested and is now moot. Flagging it rather than acting on it was the right call anyway:
widening the change would have been a palette decision nobody had made.

💰 **Why this review was cheap enough to happen, which is the transferable part.** Before
`examples/preview.rs` existed, judging the palette meant linking a temporary plugin into a
live Herdr server and unlinking it afterwards. **Mike made three such trips and the answer
was still unsettled.** The fourth attempt was a local command and settled it in one pass.
Recorded beside the example itself, not only here.

#### 7.5.5 Workspace scoping — ✅ delivered by the default

**Mike's requirement:** a dialog must be visible only in the workspace that triggered it.

✅ Measured 2026-09-11: with `workspace_id` unset, a popup does not appear when the user
switches workspace, and is intact on return. 🚨 And **setting it is refused** —
`invalid_params`, with the message that overlay and popup plugin panes target the active
pane. A nonexistent workspace id gives the *identical* error, so the refusal is about the
parameter being present rather than a lookup failing. Sending it would have meant no
dialog at all.

#### 7.5.6 The `Transport` seam

⚠️ **`dialog` sends nothing itself.** It takes a `Transport`: `open_pane` and
`show_notification`, the two socket calls it needs.

Named for the requirement rather than the transport, because `Herdr` would overclaim two
of a hundred and two methods, and a requirement-shaped name still fits when dialogs need a
third call. **Not a workaround for §4.2 being unbuilt** — a sender taken as a trait is how
this would be designed anyway, and it is what makes every path testable without a live
server. When §4.2 lands it implements the trait and no caller changes.

The answer file and the pid marker stay outside it. They are filesystem work.

✅ **Closed 2026-09-11, and the claim held.** `api::client::Client` implements
`Transport`, so a consumer supplies nothing. **No caller changed**, and nothing in
`dialog.rs` was touched to make it fit — which is what the paragraph above predicted
when it said this shape was not a workaround. The implementation lives in `client.rs`
behind the `dialog` feature, so the dialog module still knows nothing about transports.

⚠️ **`open_pane` treats any success as acceptance, deliberately.** The trait promises
Herdr accepted the request, not that a pane appeared. ✅ A popup answers `{"type":"ok"}`
and an overlay answers `plugin_pane_opened`, both measured 2026-09-11, and reading the
shape here would refuse a placement the trait never restricted.

#### 7.5.7 Costs, recorded rather than discovered later

⚠️ **`crossterm` fails the bar `toml` cleared in §5.** `toml` was accepted because all
three donors already depended on it; `crossterm` is in agentic-panes-layout's graph only,
so two of three consumers gain a dependency. Bought anyway: raw mode has no alternative
under `#![forbid(unsafe_code)]`, and the feature gate keeps it out of recent-spaces. It
buys input and terminal state only — every character of the frame is hand-written ANSI.

`unicode-width` is a **dev-dependency**, so it is absent from any consumer's graph.
Verified with `cargo tree -e normal`.

Feature-gated for §7.4's reason: recent-spaces is a headless watcher and should carry no
popup machinery.

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
| `[[startup]]` | ✅ measured **present**, 2026-09-11, with the plugin id and a context JSON beside it |
| `[[build]]` | 🚨 measured **absent**, 2026-09-11, along with every other `HERDR_*` variable |

🚨 **Measured 2026-09-11: a `[[build]]` hook is handed no `HERDR_*` variables at all.**
No socket path, no plugin id, no root, no bin path. It is an **active strip** rather than
inheritance loss: the same install was run twice, once with `HERDR_SOCKET_PATH` set
explicitly on the invoking CLI, and produced the same empty set both times, while
unrelated variables passed through untouched.

🚨 **Registration happens after the build hook completes**, also measured rather than
read. With a socket recovered by guessing its path from the config root, `plugin.list`
returns an empty list during the hook and `plugin.pane.open` answers `plugin_not_found`.
The same call immediately after the install returns the plugin.

✅ **This confirms the design above rather than disturbing it.** The kit never needed the
socket during `[[build]]`, because the manifest flag already names the context. What the
measurement does settle is §10's progress display: **no Herdr dialog can be shown during
an install compile**, which is the longest wait these plugins impose.

✅ **`herdr plugin link` still does not run `[[build]]`**, now confirmed a second time and
independently: a plugin with a build hook declared was unlinked and relinked, and its log
stayed empty.

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

✅ **Step 8 writes the note, and the template is where that became true.** One donor shim
wrote it and the other did not, which is exactly the kind of divergence three
hand-maintained copies produce and a diff against one template does not. §6.3's report
has nothing to read without it.

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

### 9.4.2 The developer override, and the one file the sync task writes

➕ **Documented 2026-09-12, having shipped undocumented.** `BUILD_FROM_SOURCE` existed in
three template files and in **zero** prose: not in this document, not in the README.
Mike asked for a development-first override by name, it was built, and nothing told
anybody it was there. 🚨 **A feature nobody can find has not shipped.**

**Create an empty `BUILD_FROM_SOURCE` file in the plugin root, and that tree compiles.**
`bin/build` checks for it at step 2 of §9.4, before any network call.

```sh
touch BUILD_FROM_SOURCE     # this checkout builds from source
rm BUILD_FROM_SOURCE        # back to fetch-or-build
```

🔑 **A file rather than an environment variable, and that is not a style choice.** Herdr
runs as a launchd agent, so a developer's shell export never reaches a script Herdr
launches. A file works whoever started the process, and somebody who has never read the
shim can still find it in a directory listing.

#### ⚠️ Which is exactly why it can be committed by accident

The marker is untracked, it sits in the plugin root, and one careless `git add .` commits
it. Every install of that release then compiles from source. **The plugin still works**,
which is why nothing complains: the only signal is a `note` line in a server log during
an install nobody is watching. §12.2 and §11.4 exist for the same failure shape, and
this one had no guard at all.

So `templates/sync_bin.py` appends this to the plugin's **root** `.gitignore`, and
`--check` fails when it is missing:

```
# The kit's developer override: its presence forces a source build.
/BUILD_FROM_SOURCE
```

🚨 **This is the only file the sync task writes that the plugin owns.** Everything in
`bin/` is byte-identical and replaceable; a `.gitignore` is not. So the edit is
**append-only**: nothing already there is rewritten, reordered, or reformatted, a missing
trailing newline is completed rather than corrected, an entry the plugin wrote its own
way is left alone under either anchoring, and a second sync adds nothing.

✅ **Tested by asking git rather than by reading the file back**: the suite creates the
marker in a real repository, runs `git add -A`, and asserts it is not staged. A text
assertion would pass on an entry git does not honour.

✅ **No plugin's CI breaks on this, and the reason survives the removal of the reusable
workflows (§11).** A plugin runs `sync_bin.py --check` out of a kit **it** checked out at
**its own** pin, so a plugin pinned at `0.1.0` runs 0.1.0's sync task and never sees this
check until it bumps. **Opting in is an explicit act**, which is precisely what lets the
check be a hard failure rather than a warning: no plugin can meet it by accident, and
none is held to it without asking.

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
| `aarch64-unknown-linux-musl` | `linux-arm64` | ✅ |
| `x86_64-unknown-linux-musl` | `linux-x64` | ✅ |
| `aarch64-pc-windows-msvc` | `windows-arm64` | ❌ never built, never fetched |
| `x86_64-pc-windows-msvc` | `windows-x64` | ❌ never built, never fetched |

📏 **Corrected 2026-09-11: the Linux triples are musl.** This table named the gnu ones,
and project-finder already publishes musl. §11.6 carries the reason, and it is not a
preference: a gnu binary carries the glibc floor of the runner that built it, so a user
on an older distribution gets a download that passes its checksum and *then* refuses to
start. **The platform names are unchanged, because they never named a libc.**

⚠️ **The two Windows names are this document's extension of a four-name convention, and
nothing has produced or consumed them yet.** ➕ **They carry `.exe`, recommended
2026-09-11 and not confirmed** (§14.2): a file without that extension is not executable
on Windows, and somebody downloading from the releases page should get something that
runs. The sidecar is therefore `…-windows-x64-<commit12>.exe.sha256`.

🔑 **Two assignments decide it, and a test runs both sides against each other.**
`WINDOWS_ASSET_EXTENSION` in `tools/plugin_gate.py` is what the release publishes;
`$AssetNameExtension` in `templates/bin/common.ps1` is what the shim asks for. Reversing
the recommendation is one line in each. Windows stays compile-verified and nothing more
(§10.1).

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

The kit holds them as templates plus a sync task. That turns shell drift into a
reviewable diff instead of silent three-way divergence.

🔑 **Decided by Mike: nothing is substituted.** An earlier draft of this section said "with
the plugin name substituted", and that is exactly what was rejected. The files land
**byte-identical in every plugin**, so a `diff` between two plugins' `bin/` directories
shows drift and nothing else. Substitution would put rendering noise beside real drift in
the one diff this whole arrangement exists to make readable.

✅ **So each shim reads the plugin's own files at run time.** Exactly two facts, and both
are inferred rather than declared:

| Fact | Read from | Why not the obvious neighbour |
|---|---|---|
| The binary's name | `Cargo.toml`'s `[[bin]]` `name` | Not `[package]` `name`: the two agree in two of the three plugins and differ in the third |
| The plugin's own name | `herdr-plugin.toml`'s **top-level** `id`, after the last dot | It names the plugin, not its binary |

🚨 **The top-level qualifier is load-bearing.** `herdr-plugin.toml` carries further `id`
keys further down, in `[[panes]]` and `[[actions]]` entries — `id = "picker"` and
`id = "apply"`. A line-anchored read returns the wrong one in **two of the three
plugins**, silently, and a log prefix reading `picker` looks entirely plausible. The
reader stops at the first section header, and a fixture manifest with a decoy `id` in a
later section pins it.

✅ Reading `version` the same way also retires a hazard the donor shims carried as a
comment: `min_herdr_version` sits in the same table, and an unanchored match reads the
Herdr floor as the plugin version and then looks for a release nobody ever cut.

⚠️ **Exactly one `[[bin]]` is required, and zero or several fails closed.** Guessing
between two binaries would fetch the asset for one and execute the other.

**Known bug to carry across:** ✅ a Herdr `[[startup]]` command gets no `$TERM` at all.
This was found once and fixed three times, each fix a different shape. ✅ The template
fixes it once, as `can_draw()` in `bin/common`, and every caller that draws asks there.
It asks two questions because they fail differently: `[ -t 2 ]` is false on the startup
path, where stderr is a pipe, and `TERM` catches a real terminal with no terminfo behind
it, which accepts escape sequences and then draws them as themselves.

### 10.0 The sync task, and the `--check` that makes it stick

```sh
just sync-bin ../herdr-plugin-recent-spaces      # or: python3 templates/sync_bin.py …
just check-bin ../herdr-plugin-recent-spaces     # writes nothing, exits non-zero on drift
```

✅ Both forms run the same module, because `just` is not installed on every machine that
has to be able to do this, and a task nobody can run is a task nobody tests.

🔑 **`--check` is what a plugin's own CI runs.** Without it the kit is a suggestion:
drift becomes a discovery rather than a failing build.

➕ **Unchanged by §11's removal of the reusable workflows, and worth saying because the
opposite was assumed for a day.** The mechanism was never the kit running this; it was
this running. §11.3 has the two-command recipe a plugin copies, against a kit the plugin
checks out at its own pin. Only the invoker changed.

⚠️ **The sync verifies by running the synced `bin/common`, not by parsing the TOML
itself.** A second parser on the Python side would agree with itself while disagreeing
with the shell, which is the failure it exists to catch.

### 10.2 The progress display is a seam, with two implementations

**Decided by Mike: a Herdr dialog wherever a dialog can be reached** — shown only while a
build is running, removed when it finishes, no cancel button.

🚨 **A `[[build]]` hook can never show one**, measured 2026-09-11 and blocked twice over
independently (§8.3): the hook is handed zero `HERDR_*` variables, and registration
happens after it completes, so `plugin.pane.open` answers `plugin_not_found` even when a
socket is recovered by hand.

🔑 **So the drawn terminal spinner stays permanently for that path**, and it is the
longest wait these plugins impose — sixty to ninety seconds of compiling against about
one second for a fetch. One extra code path, bought by a measured constraint rather than
a preference.

`bin/progress` holds the seam. `progress_start` and `progress_stop` are the only two
calls `bin/build` makes, and `progress_backend()` picks the implementation. ⚠️ **The
terminal is one named implementation, not the default everything falls back to** — that
distinction is what makes the dialog one new arm rather than a rewrite.

✅ **The placement question is answered, and the dialog itself is built** (§7.5). A popup
floats like a dialog, an overlay covers the whole tab, and popup hands back no pane id.

⚠️ **The arm in `bin/progress` is still empty, deliberately.** Filling it needs a
newline-delimited JSON socket client **in POSIX shell**: `bin/common` has no socket
helper, and the shim cannot call the Rust module, because a build spinner runs *while the
binary is being compiled* and there is no binary to call. ✅ **§4.2 has since landed and
does not help here**, for exactly that reason: the Rust client cannot run before the
binary it lives in exists. The arm stays empty, and the duplication risk that argued for
waiting is now settled rather than pending.

🚨 **The longest wait can never use a dialog whatever gets built**, per the measurement
above. So a dialog arm would only ever serve the startup and pane-hosted paths, which
already have full environments and are the fast ones.

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

#### 🚨 `platforms` works on `[[build]]` and `[[startup]]`, and is rejected on `[[panes]]`

✅ **Measured by project-finder, 2026-09-13**, on its own 0.9.0 manifest. ⚠️ **The kit has
not reproduced it**, and states it as reported for the same reason §14.2 states the
unrun Windows path as reported: the measurement is real, the reproduction is not ours.

Doubling a `[[panes]]` entry the way the `[[build]]` block above is doubled makes the
whole manifest fail to load with `duplicate pane id 'picker'`. Herdr then falls back to a
cached older copy of the manifest, which is worse than a refusal: the plugin keeps
working, as an earlier version, with nothing saying so.

🔑 **The usable rule: a pane id must be unique across every `[[panes]]` entry, whatever
platform each one declares.** ⚠️ **It is refused even when the two platform lists are
disjoint**, which is the case a reader would most expect to work and the reason the rule
is stated as an absolute rather than as a conflict between overlapping declarations.

🪤 **A mechanism was stated here and is withdrawn.** This section said pane ids resolve
before platforms are filtered, so both halves exist when the check runs. That explains
the outcome, and **nobody measured it** — project-finder wrote it, cut it from their own
notes as an inference, and it reached this document anyway. The disjoint case is the
thing it does not predict. §11.2.1 carries the shape.

🔑 **Nothing about this is the kit's fault, and the kit documented the opposite by
omission.** The block above shows the doubling pattern without saying where it stops, and
a reader generalising it to their panes gets a plugin that silently runs stale. **Double
`[[build]]` and `[[startup]]`. Do not double `[[panes]]`.**

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

🚨 **The PowerShell shims do not reach even that bar, and the difference matters.**
"Compile-verified" is a claim about Rust, which CI builds for both Windows triples.
PowerShell has no compiler and no CI job, and it is not installed on the machine these
were written on, so `bin/*.ps1` has never been **run or parsed by anything**. Each file
says so in its own header, and a test asserts that every one of them still does. Treat a
bug there as new information, never as a regression.

⚠️ **They are deliberately plainer than their shell counterparts.** No progress display,
and no download-failure classification. Unverified code should be small, because every
line of it is one nobody can test.

#### The `.exe` question lives in one assignment per side

⚠️ The launcher executes the path it builds, so a wrong answer spread across several
string concatenations would break Windows silently.

✅ It is therefore decided in **one line on each side**: `$AssetNameExtension` in
`bin/common.ps1` is what the shim asks for, and `WINDOWS_ASSET_EXTENSION` in
`tools/plugin_gate.py` is what the release publishes. ➕ **Both were set to `.exe` on
2026-09-11, recommended and not confirmed** (§14.2). Each is marked unverified where it
is assigned, and reversing the recommendation is one line in each.

🔑 **A test runs both sides against each other**, which is what makes "one line per side"
safe rather than two places to forget. It extracts the release workflow's own naming line
and executes it, asks the real shim what it would download, and compares the two answers.
A producer and a consumer disagreeing here is a 404 and a silent compile on every Windows
install.

🔑 **No shell template ever names a Windows asset.** `bin/common`'s `platform()` answers
`macos` and `linux` and refuses everything else, because `sh` cannot run on Windows at
all. Three tests hold that up: one counts the assignments across the whole repository,
one greps every shell template for a Windows asset name, and one drives `platform()`
under a stubbed `uname`.

⚠️ Note what is **not** open. Cargo always writes an `.exe` on Windows, so the local
binary path is settled and always has been. The open question is only what a release
asset is *called on the server*, and the checksum gate does not care what the file was
named.

---

## 11. CI ✅ **built 2026-09-11**, 🔻 **plugin testing removed, releases kept**

🔑 **Decided by Mike 2026-09-12, and corrected the same day.** The kit does not run
**tests** for other repositories: `plugin-ci.yml` is gone and stays gone. It does
publish releases: `plugin-release.yml` came back once the resolution problem turned out
to be solvable (§11.2.1).

⚠️ **`plugin-ci.yml` could work now, and is still not coming back.** The correction below
applies to it identically. It stays deleted because Mike does not want the kit running
another repository's tests, which is a **scope decision rather than a technical one**.
Nobody should reinstate it believing they have solved something.

🔑 **The gates never failed; one lookup did.** A callee cannot discover which version of
itself a caller pinned **from the Actions context** — but it does not have to, because
the caller's repository is checked out in front of it and the pin is in that repository's
`Cargo.toml`. §11.3's recipe and §11.5's workflow now read it the same way, out of the
same file, with the same block of shell.

**Corrected 2026-09-11 against what shipped.** This section was written before any code
existed. Three things changed in the building and survive the removal: Linux is musl
rather than gnu (§11.6), the release strips nothing (§11.5), and the logic lives in a
tested Python module rather than in YAML (§11.2.1).

### 11.1 Workflows cannot ship inside the crate

🚫 Cargo does not scaffold a consumer's repo, and Actions only runs workflows physically
present in `.github/workflows/`.

### 11.2 🔻 Reusable workflows were the answer, and they do not work

✅ The mechanism itself is real: `{owner}/{repo}/.github/workflows/{file}@{ref}` works
cross-repo, pins to a tag or SHA, and nests up to ten levels. Two shipped, each plugin
was to get a caller of roughly ten lines, and bumping the pinned ref would propagate to
all three exactly as pinning the crate does.

🪤 **It appeared to fail on the one thing it needed**, because the Actions context tells
a called workflow nothing about which of its own versions it is. §11.2.1 carries that
measurement and the correction: it does not need the context, because the caller's
repository is checked out in front of it.

**Both arrangements now work, and the split between them is a scope decision.** A plugin
runs its own checks (§11.3); the kit publishes its releases (§11.5).

### 11.2.1 The logic is `tools/plugin_gate.py`, and that is what outlived the workflows

🔑 **YAML cannot be run, so nothing that can be wrong lives in it.** One Python module
holds the target table, the asset naming and the version rules, and
`tools/test_plugin_gate.py` drives it. ✅ The same arrangement the justfile has with
`codegen/` and `templates/`: **a recipe that grows logic of its own becomes a path nobody
can test**.

➕ **That decision is why removing the workflows cost almost nothing.** The workflows were
wrappers, so deleting them deleted wrappers. Every check they invoked still exists, still
has its tests, and is now invoked by the plugin instead.

#### 🚨 Answered 2026-09-12: a called workflow cannot discover which version of itself is running

✅ **Measured, twice, identically.** `kit-ref-probe.yml` called cross-repository at
`@main` and at an annotated tag:

```
github.job_workflow_sha = <empty>
github.job_workflow_ref = <empty>
github.workflow_ref     = mike-bronner/herdr-plugin-kit/.github/workflows/
                          kit-ref-probe-caller.yml@refs/heads/main
```

**Every explanation that had been offered is ruled out by those two runs.**

| Suspected | Ruled out because |
|---|---|
| Tags do not populate it | `@main` is a branch, and it is empty there too |
| Annotated tags specifically | Empty on both forms |
| The event — the consumer failed on `push` | The probe ran on `workflow_dispatch` and failed the same way |
| `job_workflow_ref` would answer instead | It does not exist in the context at all |

🚨 **`github.workflow_ref` names the *caller's* entry workflow**, exactly as the OIDC
claims imply and not as this pipeline needed. So the only populated value points at the
caller's own file at the caller's own ref. **Following it would check a plugin out
against whatever `refs/heads/main` means in this kit**, and pass. The repository check
added the same day is what refuses it.

🔑 **The measurement stands: a reusable workflow cannot learn which of its own versions a
caller pinned *from the context*.** Four stages of this design rested on a documented
value that is empty in practice, and **the documented behaviour of
`github.job_workflow_sha` and what it does are different things**, which is worth more
outside this repository than in it.

#### 🪤 Corrected an hour later: the conclusion drawn from it was too strong

🚨 **"So there is no route" is what this section said, and it was wrong.** There is a
route, and it was written one subsection later while §11.3's recipe was being drafted for
plugins to use.

`github.repository` is the **caller's** repository, so a called workflow that runs
`actions/checkout` with no `repository:` gets the **plugin**. The plugin's `Cargo.toml`
carries the kit pin. So the callee resolves the kit's ref exactly as §11.3 does, and
never consults `job_workflow_sha` at all.

🔑 **The asymmetry was right and the inference from it was not.** A callee cannot learn
what a caller pinned **from the context**. It can read it out of the caller's checkout,
because it has one. The difference between those two sentences is a working release
workflow.

⚠️ **The tell, for next time: the answer was already written down in a neighbouring
section, for a different audience.** §11.3 told plugins to read their own pin with `cargo
metadata` at the same moment §11.2.1 concluded that nothing could. Both were written the
same afternoon. **A conclusion of the form "there is no way to do X" deserves one pass
over what was just built for somebody else.**

🪤 **Cover the invocation as well as the logic, and mutate both.** ✅ Measured
2026-09-13 while building §11.4.1's gate: replacing `python3` with `true` in the workflow
step left **every other test green**, because seven tests covered what the gate decides
and none covered whether anything called it. **A gate nobody runs passes every test
written about it.**

🚨 **Three members of one family, all found on 2026-09-13.** Each reads as established
because it is specific, and each is unbacked:

| What it looked like | What it was |
|---|---|
| A comment saying a test held four copies together | No such test existed, and it miscounted the copies |
| Seven green tests over a gate | Nothing called the gate |
| "Pane ids resolve before platforms are filtered" (§10.1) | An inference explaining a real measurement, never measured |

🔑 **The third is the hardest to catch, because an inference that explains a measurement
wears the measurement's authority.** It arrived inside a section whose whole purpose is
separating what was measured from what was read, with a caveat beneath it saying the kit
had not reproduced *it* — while the strongest sentence in the paragraph was not a
measurement at all. **A claim is not a check, a tested function nobody invokes is not a
check, and an explanation of a measurement is not a measurement.**

⚠️ **Keep this subsection through any rewrite of §11**, both halves of it. The
measurement is the reason the sections around it changed, and the correction is the
reason one of them changed back.

### 11.3 What a plugin's own CI runs

**Copy this, do not paraphrase it.** 🚨 A recipe somebody restates is how three plugins
end up running three different checks, which is the same argument that makes
`templates/bin/` land byte-identical (§10).

```yaml
# .github/workflows/ci.yml in the plugin
jobs:
  kit-gates:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
        with:
          # 🚨 Load-bearing. The version gate reads tag history, and a shallow
          # clone lets it pass by seeing no releases at all.
          fetch-depth: 0

      - id: kit
        run: |
          # ---8<--- kit pin resolution. This exact text sits in SCOPE.md §11.3's recipe
          # and in .github/workflows/plugin-release.yml, and tools/test_kit_pin.py runs
          # it and fails if the two copies differ.
          #
          # 🔑 It reads the pin out of the repository rather than asking the Actions
          # context. ✅ Measured 2026-09-12 (§11.2.1): a called workflow is told nothing
          # about which of its own versions was pinned. It does not need to be told —
          # `github.repository` is the *caller's*, so the plugin is already checked out,
          # and the pin is in the plugin's own Cargo.toml.
          #
          # `--no-deps` needs no network and no lockfile, and it answers for a
          # workspace-inherited dependency, which a regex over Cargo.toml cannot see.
          source=$(cargo metadata --no-deps --format-version 1 | python3 -c '
          import json, sys
          for package in json.load(sys.stdin)["packages"]:
              for dependency in package["dependencies"]:
                  if dependency["name"] == "herdr-plugin-kit":
                      print(dependency.get("source") or "")
          ' | head -1)
          case "$source" in
            *\?tag=*) tag="${source##*\?tag=}" ;;
            *) tag='' ;;
          esac
          if [ -z "$tag" ]; then
            echo "this plugin does not pin herdr-plugin-kit to a tag." >&2
            echo "cargo reports its source as: ${source:-<none>}" >&2
            echo "A branch, a commit or a path pin has no version to check" >&2
            echo "against, so nothing is guessed here. Pin a tag." >&2
            exit 1
          fi
          echo "tag=$tag" >> "$GITHUB_OUTPUT"
          # --->8--- end kit pin resolution

      - uses: actions/checkout@v7
        with:
          repository: mike-bronner/herdr-plugin-kit
          ref: ${{ steps.kit.outputs.tag }}
          path: kit

      - run: python3 kit/templates/sync_bin.py . --check
      - run: python3 kit/tools/plugin_gate.py versions .
```

Three things in it are load-bearing and none is obvious:

- 🚨 **`fetch-depth: 0`.** The version gate reads tag history, and a shallow clone lets it
  pass by seeing **no releases at all** (§11.4). A gate that passes because it saw
  nothing is the silent-pass class this project keeps finding.
- 🔑 **The tag comes from the plugin's own dependency pin**, read through `cargo
  metadata` rather than out of the TOML. ✅ Measured 2026-09-12: `--no-deps` needs no
  network and no lockfile, and it answers `git+…?tag=0.3.0` for a **workspace-inherited**
  dependency, which a regex over the member's `Cargo.toml` cannot see at all. It is the
  same standard `plugin_gate.py` already holds itself to — ask the tool, do not parse the
  file.
- ⚠️ **A pin that is not a tag stops the job.** ✅ Measured on all four forms: `?branch=`,
  `?rev=`, a bare git source and a path dependency each answer no tag, and the recipe
  says so rather than checking out nothing. A plugin pinning a branch is a real case and
  it has no version to check against.

⚠️ **The plugin's own suite, formatting and lint are the plugin's business.** The old
`gates` job ran them and it was the one part that worked, but nothing about it needed the
kit: it is `cargo test` and `cargo clippy` on the runner's own toolchain.

#### 🔑 Sync `bin/` from the tag you pin, never from the kit's `main`

➕ **Reached independently by both migrating plugins, 2026-09-13**, which is as close to
confirmation as two consumers get.

**The check above reads a tag; `just sync-bin` reads a working tree.** The recipe
resolves the pin, checks the kit out at that ref, and runs `sync_bin.py --check` against
**that** tree. A tag is immutable, so whatever `main` says is invisible to it.

🪤 **So syncing from `main` is the one action that turns a passing check into a failing
one**, and it fails naming the file it has just "fixed". A template change on `main`
cannot reach a plugin until the plugin asks for it.

✅ **Which means a plugin picks a template fix up at the pin bump, and not before.** One
action, one diff, one review — rather than a sync now and a bump later that may not agree
with each other.

⚠️ **This corrects an inference, not a measurement.** A source change in `templates/`
was read as consumer drift without asking what the check compares against, and the answer
was thirty lines away in the recipe itself. **Ask what a check reads before predicting
what it will say.**

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

➕ **As built it makes four assertions, in this order.** Each one is silent in production.

| # | Assertion | What its absence costs |
|---|---|---|
| 1 | `bin/common` and cargo name the same binary | The release publishes one name and every install requests another |
| 2 | `herdr-plugin.toml` and `Cargo.toml` state the same version | The shim reads the first and cargo builds the second |
| 3 | No `v`-prefixed tag names the declared version | The fetch 404s and compiles, in silence (§12.2) |
| 4 | No release tag sorts above the declared version | An install at that release asks for a tag naming an older version |

🔑 **Nothing in the gate parses TOML.** The shim's answers come from running the plugin's
own synced `bin/common`, and cargo's come from `cargo metadata`. Those are the two
readers that actually run, at install time and at build time, so the gate compares two
live implementations rather than adding a third opinion about the files. ⚠️ This is
§10.0's rule — *a second parser would agree with itself while disagreeing with the
shell* — applied one directory over.

⚠️ One window the gate cannot close: between the version-bump commit landing on `main`
and the tag being pushed, `main` advertises an unreleased version. Assertion 4 permits it
deliberately, because it is the ordinary state of a bumped tree. Push the bump and the
tag together, or trigger the release from the bump commit.

#### 🚨 11.4.1 Every pin naming the kit names the same kit

➕ **Added 2026-09-13, and it closes a hole in the kit rather than in a plugin.**

A plugin names this kit in **three** places: the runtime crate, the build crate, and the
`uses:` that calls the release workflow. ✅ Measured on project-finder: all three, and
nothing compared them. **A release built by one kit version while the crate depends on
another publishes assets from a tree the plugin does not use**, and §11.4's gate does not
look at it because it compares the plugin's own versions rather than the kit's.

🔑 **The evidence that it is the kit's hole**: project-finder's caller carried *"Nothing
in the kit's own gates compares them, so tests/packaging.rs does."* A consumer patching a
hole in the thing it depends on is how three plugins end up with three different tests,
which is the argument §11.3's recipe rests on.

⚠️ **The workflow's own ref is not recoverable from the Actions context.** ✅ Measured
2026-09-12 (§11.2.1): `job_workflow_sha` empty, `job_workflow_ref` absent, and
`workflow_ref` naming the **caller's** entry workflow. So the check does not ask what
version it is. It reads every pin out of the plugin's own checkout and compares them to
the kit that was actually checked out — the same route as the pin resolution, and the one
the first successful release proved.

⚠️ **It runs after the kit checkout, so it is not YAML logic.** The pin resolution cannot
call the kit, because it is deciding which kit to fetch. This can, so it lives in
`tools/plugin_gate.py` with everything else (§11.2.1).

➕ **Wider than the hole reported.** The rule is *every* pin naming this repository, not
the two that were noticed. A fork whose name merely starts the same is excluded by an
anchored match, because a prefix test is not a name test.

### 11.5 `plugin-release.yml` ✅ **restored 2026-09-12**

Tag-triggered, cross-repository, and the one workflow the kit still runs for a plugin.
Builds the matrix, generates a `.sha256` beside each binary, and uploads both. Produces
exactly what §9.6 consumes: raw binaries, keyed on the commit, and **no archive step**.

➕ **Two things changed while it was away, and both are improvements it keeps.**

- 🔑 **It resolves the kit from the plugin's own pin** (§11.2.1), not from the Actions
  context. The same block of shell as §11.3's recipe, byte-identical, held together by
  `tools/test_kit_pin.py`.
- 🔑 **It no longer names assets itself.** `plugin_gate.py asset-name` is the producing
  half of §9.6's convention, and the shim is the consuming half. A name this workflow
  built for itself would be a second opinion about a convention already written down, and
  `tools/test_plugin_gate.py` runs both halves and compares them.

Requires the **caller** to grant `contents: write`. A called workflow runs on the
caller's permissions. ✅ The requirement is stated in the workflow's own header and in the
README's caller snippet, which are the two places a caller reads, and a caller who omits
it gets a run that fails before it starts rather than a 403 halfway through a release.

⚠️ **It strips nothing.** All three plugins set `strip = true` in `[profile.release]`, so
cargo strips at link time, **before** macOS ad-hoc signs the Mach-O. A strip step after
the fact would re-strip an already stripped binary and take a re-signing risk on arm64
for nothing.

➕ **All six, or none.** The publish job needs the whole matrix, and it counts the files
it collected against the size of the table before uploading any of them. Five platforms
published and a sixth missing is not a partial success: it is one platform compiling on
every install, forever, with nothing to say so.

➕ **Both caller triggers work.** `release: types: [created]` is what project-finder uses
today; a bare tag push works too, and the release is created if it does not exist.
Anything else — a branch, a manual run on `main` — reaches the tag-form gate and is
refused there.

#### ✅ Established 2026-09-12: a target that cannot compile is already loud

**Asked because it had to be established rather than assumed**, and the answer is that
nothing needs closing. Read off the file, there is no path to a green release with
nothing attached:

| Step | What the file does |
|---|---|
| A leg cannot compile | `cargo build` fails, so that leg fails |
| A leg compiles but produces no file | `upload-artifact` carries `if-no-files-found: error`, so that leg fails |
| Any leg fails | `build` fails. ⚠️ `fail-fast: false` only stops the **others being cancelled**, so every failure is visible; it does not change the job's conclusion |
| `build` fails | `publish` declares `needs: [guard, build]` and **no `if:`**, so it is skipped. The only `if:` in the whole file is the musl step's `runner.os == 'Linux'`, and there is no `always()` and no `continue-on-error` anywhere |
| `publish` is skipped | `gh release create` lives **only** inside it, so nothing is created and nothing is uploaded |
| `publish` somehow ran partial | The count compares files found against twice the matrix rows and exits **before** any upload |

🔑 **So the worst case is a red run**, which is a problem somebody fixes, rather than a
green release with no assets, which is one nobody notices until every install compiles.

⚠️ **One shape resembles it and is not it.** Under the `release: created` trigger a
*human* created the release before the workflow started, so a red run leaves a
human-created release with no assets attached. The workflow cannot prevent that — the
release existed before it ran — and its loudness is the red run beside it.

🚨 **The worked example was live and is now closed.** project-finder could not compile
for **either** Windows target: `src/layout.rs` reached for
`std::os::unix::fs::PermissionsExt` to test executability and
`std::os::unix::process::CommandExt` with `setsid` to detach a child. 🔑 **Mike's
decision: port `layout.rs` rather than narrow the matrix.** ✅ Both reaches are now
`#[cfg]`-split, the six targets stay, and all six compile and link.

⚠️ **The chain above is still read off the file, and that caveat narrows rather than
disappears.** ✅ What is now observed is the **success** path. **No leg has failed in a
real run**, so every row in that table except the first remains GitHub's documented
`needs` and matrix semantics rather than something anybody has watched happen.

#### ✅ It has run — measured 2026-09-13

Verified against GitHub rather than reported: project-finder's **0.9.0**, run
`34762541628`, conclusion **success**, event `release`, published `2026-09-13T14:23:49Z`
with **12 assets** — six platforms, a binary and a `.sha256` each, every one keyed on
commit `0837eb5571b6`.

| What that settles | How |
|---|---|
| The six build legs | All six native runners produced a binary |
| The publish job | The all-six count passed and the upload ran |
| §9.6's commit-keyed convention | Every published name carries the same 12 characters |
| `asset-name` as the producing half | The names GitHub holds are the ones it emits |
| The `.exe` recommendation, **naming half only** | Both Windows assets published as `.exe` (§14.2) |

🔑 **project-finder also fetched what the real `asset_url` named and matched its sha256
against the published one.** That is the round trip `tools/test_plugin_gate.py`
structurally cannot do: the suite runs both halves of the naming agreement and compares
them, and nothing in it can ask GitHub whether the file is actually there.

### 11.6 Build matrix — native runners, not cross-compilation

GitHub provides native runners for all three operating systems, so cross-compilation buys
nothing and macOS is the painful case.

| Target | Runner |
|---|---|
| `aarch64-apple-darwin` | `macos-latest` |
| `x86_64-apple-darwin` | `macos-latest` (Apple's own SDK handles it) |
| `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` |
| `x86_64-unknown-linux-musl` | `ubuntu-24.04` |
| `x86_64-pc-windows-msvc` | `windows-latest` |
| `aarch64-pc-windows-msvc` | `windows-11-arm` |

📏 **Corrected: Linux is musl, not gnu.** This table named the gnu triples. ✅
project-finder 0.8.0 already publishes musl, and it is the right answer for a downloaded
binary: **a gnu build carries the glibc floor of the runner that produced it.** A user on
an older distribution would then get a download that passes its checksum and *then
refuses to start* — which is worse than the 404 this whole design is built around,
because the fetch succeeded and the fallback never fires. musl links its libc statically
and has no floor. The runners install `musl-tools`; the platform names are unchanged,
because they never named a libc.

✅ **What project-finder needed zig and `cargo-zigbuild` for, native arm64 runners now
do.** Its release workflow cross-compiles arm64 from an x86 runner. This one does not
need to, and drops two third-party actions with it.

**All six targets ship. Decided by Mike, closing an earlier question about dropping
`aarch64-pc-windows-msvc`.** None of the six is optional. Shipping Windows release assets is
what couples the shim question in section 10.1 to this one.

✅ arm64 runners went GA and free for public repos in August 2025, and reached private
repos in January 2026. There is no `ubuntu-latest-arm` alias, only versioned labels. A
test pins that: every arm64 row must carry a versioned label, because a job naming a
label that does not exist never starts, and the release then publishes five assets.

### 11.7 Trigger on `push` as well as `pull_request`

🪤 ✅ When a PR cannot compute a merge ref against `main`, Actions skips `pull_request`
workflows entirely. No run, no error, and the checks simply never appear. A push trigger
keeps a conflicted branch covered.

➕ **It applies to a plugin's own workflows now**, and the kit's own CI (§11.8) already
carries both triggers. It is written down here because a plugin author copying §11.3's
recipe is choosing triggers at the same moment.

### 11.8 ➕ The kit's own CI, and where the slow check lives

`kit-ci.yml` runs on push and pull request: five suites on **both** Ubuntu and macOS,
formatting, clippy with warnings denied, and a clippy pass over all six target triples.

✅ **Cross-linting is the whole Windows guarantee, and it is enough for it.** clippy stops
at analysis and never links, so no MSVC toolchain is needed to reach it. This is the
README's documented command, run automatically instead of by hand.

⚠️ **`--features dialog` is load-bearing in the lint.** `dialog` is off by default, so
without it clippy never compiles `dialog.rs` and never lints a line of it. The justfile's
own recipe was corrected to match, because a gate stricter than what a developer runs by
hand surprises them in CI rather than at their desk.

➕ **A documentation job, added 2026-09-11 after eleven warnings were found by building
the docs for the first time.** 🚨 `cargo clippy --all-targets` does not run rustdoc at
all, so broken intra-doc links, links from public documentation into private items, and
links to a crate that is not a dependency were checked **nowhere**. One of the eleven had
been in `herdr-plugin-kit-build` since §6.1 was built and survived four stages of green
checks. It fails on warnings rather than printing them, because a warning nobody blocks
on is how eleven accumulated.

It runs with every feature off and with every feature on, because a link to a gated item
resolves only in the build that turned the feature on and warns in the build that did
not. 🪤 **It also passes `--document-private-items`, which is not thoroughness.** Measured
2026-09-11: without that flag rustdoc never resolves a link written on a **private**
item, so a doc comment naming a function somebody had deleted passed the job silently —
which is the defect that prompted the job. This kit documents its private functions as
carefully as its public ones, so those links earn the same check.

🚨 **The mutation runs are a separate workflow, and that placement is the decision.** The
harness recompiles once per mutation and there are 42 of them. Two failures were weighed:

- Folding it into the fast gate makes every pull request wait twenty minutes, and **a
  check people wait twenty minutes for is a check people learn to route around.**
- Leaving it to `just mutate` by hand is worse. **A check nobody runs on a schedule rots
  into a claim**, which is the one thing a mutation harness cannot afford to be.

So `kit-mutation.yml` runs on every push to `main` that touched the crate, the specs or
the harness, and on demand. Landing on `main` is where this repository actually works
today, so it is the tightest automatic trigger available, and it reports rather than
gates. 🔑 **It is its own file because a `paths` filter applies to a workflow and never
to one job inside it** — otherwise a documentation commit triggers twenty minutes of
recompiling. The two specs run as two legs, so the wall clock is one spec rather than both.

### 11.8.1 🔑 No minimum-toolchain job, because there is no minimum to assert

**Decided by Mike 2026-09-11: this workspace declares no `rust-version` at all.** The
reasoning is better than either alternative that was put to him. **Plugins built with the
kit ship as compiled binaries, so a consumer needs no toolchain — that is the whole point
of download-by-default (§9.2, §9.3).** A floor documents a requirement for people who will
never compile, and the number it would carry is whatever the dependency tree currently
demands rather than anything this project chose.

⚠️ **One case is recorded rather than argued with**, because it is the single one where
the field was not useless: `bin/build`'s fallback path *does* compile from source when a
fetch fails (§9.4), so a user on that path with an old toolchain gets a compile error.
That failure is loud and names itself. It does not justify keeping a claim that was
measurably false, and **removing a false claim beats keeping it in either direction**.

#### 📏 What the tree demands today — **1.85**, corrected 2026-09-12

This is a fact about the dependency graph rather than a promise, and it is worth
re-measuring whenever somebody asks what toolchain this needs. ⚠️ **The number below
replaces "above 1.80", which this section carried until 2026-09-12 and which understated
the floor by five releases.** recent-spaces was about to put 1.80 in its README.

**The floor is `edition = "2024"`, which stabilised in Rust 1.85**, and ✅ three crates in
the runtime crate's *normal* tree declare it:

| Crate | Declares `rust-version` | Reached |
|---|---|---|
| `regress 0.10.5` | ❌ **none at all** | **direct dependency** of the runtime crate |
| `hashbrown 0.17.1` | ✅ `1.85.0` | `toml` → `toml_edit` → `indexmap` |
| `indexmap 2.14.2` | ✅ `1.85` | `toml` → `toml_edit` |

🚨 **`regress` is the constraint that matters, and it is the one nothing advertises.** It
is a direct dependency with or without the `dialog` feature, it declares edition 2024 in
its own manifest, and it declares no `rust-version`, so nothing in `Cargo.lock` says a
word about it. The other two are transitive *and* self-declaring, which is the easier
case in both directions.

⚠️ **What this section said before was the weakest of the three reasons, and it read the
limit wrongly.** It cited only the `hashbrown` manifest-parse failure and framed the
floor as a cargo *parsing* limit four levels down. Parsing is merely where 1.80 stops
first: `regress` and `hashbrown` are both *compiled into* every consumer, so the floor is
about building an edition-2024 crate rather than about reading a manifest.

✅ **1.80 fails, reproduced again 2026-09-12:**

```sh
cargo +1.80 check --all-targets --features dialog --locked
# error: failed to parse manifest at `…/hashbrown-0.17.1/Cargo.toml`
#   feature `edition2024` is required
```

🚨 **1.85 is reasoned, not measured, and the difference is stated rather than buried.**
No toolchain between 1.80 and 1.97 is installed on this machine, so **nothing has been
built at the 1.84/1.85 boundary**. The figure comes from the declared edition, from when
edition 2024 stabilised, and from two crates declaring `1.85` outright. The 1.80 half is
reproducible here; the 1.85 half is not.

➕ **A second floor sits underneath that one and moves for a different reason.** The
generated types use `std::sync::LazyLock`, which `cargo-typify` emits for every
pattern-constrained string in Herdr's schema (§3) and which landed in **1.80**. That one
is a property of the generated file rather than of the lock, so re-resolving the tree
never lowers it and `just sync-api` can raise it.

✅ **Dropping the field changed no dependency resolution.** Cargo's MSRV-aware resolver
consults `rust-version`, so this was checked rather than assumed: `Cargo.lock` is
byte-identical after a full re-resolution, and `cargo metadata --locked` still passes.
🔑 It could not have changed, for a reason the lockfile already showed: this workspace is
`resolver = "2"`, and MSRV-aware resolution needs `resolver = "3"` or an explicit
`resolver.incompatible-rust-versions` setting. The field was present while the lock
resolved `hashbrown 0.17.1`, which is itself proof it was never steering anything.

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

- ➕ ✅ **Pin the expected asset URL in a test, whole, including the tag form.** ⚠️ A test
  that builds the URL the same way the shim builds it cannot catch a wrong tag form. It
  has to state the expected string. **Done 2026-09-11**, in
  `templates/test_templates.py`: `uname` is stubbed so that every part of the string is a
  literal except the commit, which cannot be one.
- ➕ ✅ **The gate must reject one of the two forms.** Tolerating both `0.8.0` and
  `v0.8.0` is exactly how two conventions drift apart and then disagree without saying
  so. **Done 2026-09-11**: `TAG_PREFIX` in `tools/plugin_gate.py` is one named constant,
  and the gate refuses the other form by name rather than ignoring it. It runs in
  `plugin-release.yml`'s guard job (§11.5), which is the one place that publishes and so
  the one place that can stop the two conventions crossing.

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
- ➕ **A source-breaking change is a minor bump at minimum**, added 2026-09-12. 🔑 **0.2.0
  is the first test of this rule, and it arrived before the protocol case it was written
  for.** `Client::call` became generic (§4.4), so every call site needs a turbofish that
  0.1.0 did not. ⚠️ A patch bump would have been worse than merely understated: Cargo
  treats 0.1.1 as compatible with 0.1.0, so a consumer on a version range would have
  taken it silently and then failed to compile. **The number is what tells a consumer
  whether to expect work**, and it is the only thing that tells them before they upgrade.
  ⚠️ **The second and third tests arrived the same day.** 0.3.0 widens every `ratio`
  from `f32` to `f64` (§3.2.1), which breaks any consumer that bound one to an `f32`.
  0.4.0 removes two reusable workflows consumers were told to call (§11), which breaks
  any plugin that wired one up — recent-spaces did. 🔑 **Three source-breaking changes in
  one day, on a rule with no exercise before any of them**, and the third is the one the
  rule's wording nearly missed: nothing about a deleted workflow is Rust source, and it
  breaks a consumer just the same.
- ➕ **A change that does not break a consumer is a patch**, added 2026-09-12, and 0.4.1
  is the first. ⚠️ **It read "a purely additive change" for one release**, which is
  narrower than the rule intends and did not cover 0.4.2, a documentation-only release
  that adds nothing at all. **Additive and documentation-only are two shapes of
  non-breaking, and the rule is about the class rather than either shape** — the same
  wording trap that nearly let a deleted workflow through as a patch, two clauses down.
  🔑 **The rule above had been tested three times in one day and every test was a break**,
  so this section spoke only to breaking changes while the common case went unstated and
  the next person would have inferred it. Stated beside the breaking clause so the
  contrast is visible rather than assembled.
  ⚠️ **Two cases from that same day show what "additive" does not mean.** A deleted
  reusable workflow is not Rust source and breaks a consumer just the same, so it took a
  minor bump. And restoring it is additive **against the release that removed it**, not
  against the release that had it: the interval a version describes is the one since the
  last tag, never the last time the tree looked this way.
- ➕ **The commit type and the version number answer different questions, and may
  disagree**, added 2026-09-13. The type says **what the change is**; the number says
  **what a correct upgrade asks of a consumer**. ⚠️ Conventional Commits maps `feat:` to
  a minor, and this project does not follow that mapping, because the clause below
  governs the number. ✅ Both directions are already in this history: **0.3.0 was a
  `fix:` at a minor** (widening `f32` to `f64` broke every consumer) and **0.4.1 was a
  `feat:` at a patch** (three additions broke none). 🔑 Forcing them to agree would have
  mis-numbered one of those two.
- ➕ **What decides the number is whether a *correct* upgrade needs work**, added
  2026-09-13, and it is the class the three clauses above were reaching for. A change
  that asks something of a consumer who upgrades **correctly** is a minor at minimum:
  0.2.0's turbofish, 0.3.0's `f32` to `f64`, 0.4.0's deleted caller. A change that only
  refuses an upgrade done **incorrectly** is a patch, because a consumer doing it right
  sees nothing. 🔑 **0.4.3 is the first of the second kind**: a plugin whose three kit
  pins agree is unaffected, and one that bumps only some of them was already publishing
  assets from a kit it does not depend on. ⚠️ **"It can make a build fail" is not the
  test**, or every new check would be a break and no check would ever ship.
- ➕ **A consumer may pin any kind of tag**, added 2026-09-12. All three of this kit's
  releases are annotated tags, and ✅ the probe in §11.2.1 measured both kinds behaving
  identically. `actions/checkout` takes either, and a `cargo` git dependency takes either.
  ⚠️ **If that ever stops being true, it belongs here rather than in whoever happens to
  remember it**: a check that serves one kind of tag is a trap for whoever tags the next
  release without knowing which kind they made.
- Promotion to crates.io stays open and needs no design change.

---

## 13. Migration

| Order | Plugin | Why |
|---|---|---|
| 1️⃣ | recent-spaces | Smallest at 158 lines of `api.rs`, no TUI, donates the best `version.rs`, and carries the live `v`-prefix hazard (§12.2) |
| 2️⃣ | agentic-panes-layout | Donates `issues.rs` |
| 3️⃣ | project-finder | Only one with a real behaviour change (§7.3), and donates the shim (§9.4) |

#### What migrating a plugin now involves

➕ **Rewritten 2026-09-12**, because two of the four steps changed the same day.

1. Pin the kit by tag, and **name a result type at every call site**. The pin alone makes
   the binary bigger, which is the next subsection.
2. `just sync-bin <plugin>`, then commit `bin/` and the `.gitignore` entry (§9.4.2).
3. Copy §11.3's recipe into the plugin's own CI. ⚠️ **Copy it; the kit no longer runs it
   for anybody** (§11), and a paraphrase is how three plugins end up running three
   different checks.
4. Wire up `plugin-release.yml` (§11.5), which the kit still runs. ➕ **Corrected
   2026-09-12**: this step said "build your own release job" for part of one day, while
   the workflow was deleted.

#### 🚨 Bumping the pin is half the migration, and half is worse than none

✅ **Measured 2026-09-12 by recent-spaces, the first real migration**, across three builds
of one tree at `opt-level = "s"` with `strip = true` on macOS arm64.

| Build | Bytes |
|---|---|
| kit 0.1.0, naming `ResponseResult` | 3,257,600 |
| kit 0.2.0, naming `ResponseResult` — **the pin bumped and nothing else** | **3,350,544** |
| kit 0.2.0, naming `WorkspaceListAnswer` | 1,878,832 |

🚨 **And a whole migration can still cost more than it saves.** ✅ project-finder
narrowed every one of its seven call sites and its binary grew 66% against the
hand-written client it replaced (§4.4). Narrowing was worth 1.1 MB to it and the kit cost
it more than that. **Neither figure generalises**: what the kit buys a plugin is one
description of Herdr's wire format instead of a hand-maintained one, and on a plugin that
touches much of the API it is not bought cheaply.

🚨 **A plugin that bumps the pin and stops is 92,944 bytes worse off than it was on
0.1.0.** The per-variant types are 128 new types every consumer compiles, and nothing
recovers that until a call site names one. Narrowing then saves **1,471,712 bytes,
43.9%**, and the net against 0.1.0 is **1,378,768, 42.3%**.

⚠️ **So the migration is the call sites, not the pin.** There is no signal when somebody
does half of it: the build succeeds, the plugin works, and it is simply bigger. §4.4 has
the mechanism and the kit's own figures.

**Ship the kit with `api`, `env`, and `version` only.** Hold `report` and `update` until
one real consumer has proven the boundaries. Designing abstractions with no consumer is
how they come out wrong.

#### The hold was overridden once, for `dialog`, on 2026-09-11

🔑 **Decided by Mike, deliberately and for that module alone.** The rule above still
stands and still holds `report` and `update`. This is an exception somebody made, not a
bar that quietly stopped being enforced.

⚠️ **Two different justifications are doing the work here, and a reader applying this bar
to something else needs to tell them apart.**

| Half | Basis | Does it satisfy the one-consumer bar? |
|---|---|---|
| **The mechanism** — popup placement, the file answer channel, the pid marker | agentic-panes-layout's `src/confirm.rs`, 668 lines solved once against a live server and shipping in production for months | ✅ **Yes.** This is exactly what the bar asks for: boundaries proven by a real consumer before promotion |
| **The four styled states** — the states, the glyph set, the button row, hover | None. New design | ❌ **No.** Mike asked for these by name, and chose to build them without one |

So the bar was **met** for the risky part and **waived** for the new part. That split is
the whole reason the override was safe to make: the machinery nobody could design blind
already had its consumer, and what had no consumer is a visual design Mike specified
himself and can look at.

⚠️ **`report` and `update` have neither half.** Nothing about this exception transfers to
them, and §7.5's mechanism being proven says nothing about §7.2's fall-back policy or
§8's updater.

**Consequences accepted with the override:**

- Two of three consumers gain a `crossterm` dependency (§7.5.7).
- ✅ **The palette shipped unmeasured and was reviewed and approved on 2026-09-11**
  (§7.5.4). It is the consequence on this list that closed, and it closed because
  `examples/preview.rs` made looking at it a local command.
- The module is feature-gated off by default, so a consumer that wants none of this
  carries none of it.

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
  to be a new release cut through `plugin-release.yml` (§11.5), because there is
  nothing to attach commit-keyed assets to retrospectively.
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
| 2 | Does a Windows asset **run**? | ➕ **Narrowed 2026-09-13.** The naming half is answered; the running half is untouched. See below |
| 3 | Is the socket reachable at `[[startup]]`? | ✅ **No longer a correctness question** (§8.3). Purely an optimisation now: measuring it could save a `plugin.list` call |

✅ **The naming half closed on 2026-09-13.** project-finder's 0.9.0 published
`pick-project-windows-arm64-0837eb5571b6.exe` and its x64 sibling, so the extension is no
longer a recommendation: it is what the producing and consuming halves both say, on a
real release (§11.5). **Reversing it is still one line in each of two places** (§9.6).

🚨 **The running half is untouched, and it is the half that matters.** Both Windows
binaries **compiled, linked and published. Nobody has run one.** Nobody on this project
has Windows hardware.

⚠️ **And there is a specific reason to expect trouble rather than a general one**, raised
by project-finder: **Windows has no session concept.** `DETACHED_PROCESS` gives a child
its own process group and no console, which is what `setsid` is being replaced with — but
it does **not** exempt that child from a parent job object. If Herdr runs plugin panes
inside a kill-on-close job, a detached layout child dies with the picker that spawned it,
on Windows only, and every Unix test passes.

💰 **A live instance is still the cheapest confirmation available**, and now there is
something to download: running either published `.exe` on any Windows machine answers the
half that is open.

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

### 15.2 Extended 2026-09-11: the `dialog` module

Everything below was measured against an isolated 0.9.0 server, or against the Unicode
database, in the session that built §7.5. None was relayed.

| § | What changed | Kind |
|---|---|---|
| 7.5 | New module. Four states, two variants, popup placement | ➕ new |
| 7.5.1 | `plugin.pane.open`'s response follows the **effective placement**. Only `popup` answers without a handle, because a popup is not in the pane tree | 📏 measurement |
| 7.5.1 | The file channel and the pid marker are therefore load-bearing rather than defensive | 🔧 design |
| 7.5.2 | A click into a plugin pane arrives rebased to pane-local coordinates, and keyboard keeps working alongside | 📏 measurement |
| 7.5.3 | The single-popup limit is **global, not per workspace**, so `ui_busy` falls back to a notification | 📏 measurement + 🔧 design |
| 7.5.4 | The glyph rule: one codepoint, no selector, East Asian Width `Neutral`. Four sets were tried | 📏 measurement |
| 7.5.5 | Workspace scoping is delivered by the default, and `workspace_id` is **refused** for popup placement | 📏 measurement |
| 13 | The one-consumer hold overridden for `dialog` alone, with the evidence and the decision recorded separately | 🔑 decision |

⚠️ **One thing in §7.5 was still unmeasured when this was written and said so in every
place it appeared:** the colour palette. ✅ **Closed 2026-09-11 by review rather than by
measurement** — Mike approved all four states from the `preview` example, in Terminal.app,
under his own theme (§7.5.4). ⚠️ The half that review cannot reach is still open and is
still recorded: the probe discarded SGR attributes, nobody has established how Herdr
colours its own dialogs, and nobody has put the two blues in one frame.

Related vault notes: `decisions/2026-09-10-herdr-plugin-kit-shared-crate.md`,
`decisions/2026-09-10-local-install-check-belongs-in-plugin-kit.md`,
`decisions/2026-09-10-unprefixed-release-tags.md`,
`insights/2026-09-10-typify-drops-discriminator-beside-sibling-property.md`,
`insights/2026-09-10-herdr-build-never-runs-for-local-installs.md`,
`insights/2026-09-10-caveat-decay-is-one-way.md`,
`insights/2026-09-11-plugin-pane-open-placement-decides-the-handle.md`.

### 15.3 Extended 2026-09-11: the transport and the handshake

The same session, later. §4.2 and §4.3 moved from specified to built.

| § | What changed | Kind |
|---|---|---|
| 4.2 | `api/client.rs`. One connection per call, newline-delimited JSON, `interprocess` over both platforms | ➕ new |
| 4.2 | Off Unix there is **no** fallback socket: `interprocess` refuses any path that does not already start `\\.\pipe\`, so reusing the Unix default would fail opaquely and inventing a pipe name would fail plausibly | 📏 measurement + 🔑 decision |
| 4.2 | The answer's id is checked against the request's, which is what makes the atomic counter load-bearing rather than decorative | 🔧 design |
| 4.2 | A receive timeout arrives as `WouldBlock` on macOS, established against a deliberately wedged server rather than inferred from the API | 📏 measurement |
| 4.2.1 | No second error-code list was built. `dialog::BUSY_CODE` already is that list, and the client routes through it | 🔑 decision |
| 4.3 | A mismatch is a returned value, never a side effect, following §6.2, §7.2 and §7.5 rather than departing from them | 🔑 decision |
| 7.5.6 | The `Transport` seam met a real client. No caller changed, as §7.5.6 predicted | ✅ confirmation |

⚠️ **Nothing in §4.2 or §4.3 has run against a live Herdr server.** The tests drive a
scripted peer over a real socket, so the bytes are genuine and the peer is not. Every
Windows path remains compile-verified only.

Related vault note:
`insights/2026-09-11-plugin-pane-open-placement-decides-the-handle.md`, which carries the
measured `[[build]]`-hook and event-hook environments that make the socket fallback an
ordinary path rather than a rare branch.

### 15.4 Extended 2026-09-11: CI, and §11 corrected against what shipped

§11 moved from specified to built, and four of its statements did not survive contact
with the code. Everything below was measured against a repository on disk, a live
toolchain, or project-finder's own shipped workflow. None was relayed.

| § | What changed | Kind |
|---|---|---|
| 11.3 | The three inputs are gone. Every one repeated a manifest fact, and an input that can disagree with the manifest publishes one asset name while every install requests another | 🔧 design |
| 11.2.1 | The rules live in `tools/plugin_gate.py` with their own suite, and the workflows are wrappers. YAML cannot be run, so nothing that can be wrong is written in it | ➕ new |
| 11.4 | The gate asks `bin/common` and `cargo metadata` rather than parsing TOML — §10.0's rule, one directory over — and makes four assertions rather than one | 🔧 design |
| 11.5 | No strip step. All three plugins already set `strip = true`, so cargo strips before macOS ad-hoc signs the Mach-O, and a later strip is a re-signing risk for nothing | 📏 measurement |
| 11.5 | Publishing is all six or none, counted against the table | ➕ new |
| 11.6, 9.6 | Linux is **musl**, not gnu. A gnu binary carries the builder's glibc floor, and that failure survives the checksum gate and then refuses to start | 📏 corrected to what shipped |
| 11.8 | The mutation runs are their own workflow, on `main` and on demand. A `paths` filter applies to a workflow and never to one job | 🔑 decision |
| 9.6, 14.2 | Windows assets carry `.exe`. **Recommended, not confirmed** | ➕ recommendation |
| 11.8.1 | `rust-version` is dropped entirely. A binary that ships compiled needs no toolchain, so the field documented a requirement for people who will never compile | 🔑 decision |
| 11.8.1 | What the tree demands: `hashbrown 0.17.1`, through `toml` → `toml_edit` → `indexmap`, needs `edition2024`, which 1.80's cargo lacks. The declared 1.80 was already false | 📏 measurement |
| 12.2 | The whole asset URL is now pinned in a test as a literal string, which §12.2 required and nothing had done | ➕ new |

⚠️ **One thing in §11 is unverifiable here and says so in every place it appears:** none
of these workflows has run. They were written on a machine with no `just`, no
`actionlint`, and no Windows, so the YAML is checked by a parser and the logic is checked
by a suite, and the first real run is the first real evidence.

✅ **Three claims in §11 were established by running them**, rather than reasoned about:
all six triples clippy-clean from macOS, 1.80 failing on `hashbrown`, and the lockfile
staying byte-identical when `rust-version` was dropped. The second is what §11.8.1
records; the third is why dropping the field is a documentation change and not a
resolution change.


### 15.5 Extended 2026-09-11: the dialog seen, and the palette closed

The same day, after Mike ran `examples/preview.rs` for the first time. Everything below
came from looking at rendered output, which is the only instrument a palette or a layout
has.

| § | What changed | Kind |
|---|---|---|
| 7.5.4 | Info is SGR **94**, the bright blue, rather than 34 | 🔑 decision |
| 7.5.4 | **The palette is reviewed and approved**, all four states | ✅ review |
| 7.5.4 | The bright-info asymmetry was reviewed with the rest and **stands** | ✅ review |
| 7.5.4 | The buttons **stack** when one row cannot hold both labels whole | 🔧 design |
| 7.5.4 | Truncation survives as the last resort, for one label too wide for a row of its own | ➕ new limit |
| 11.8 | A documentation job, after eleven rustdoc warnings were found by building the docs for the first time | ➕ new |

🚨 **Approved is not matched, and this document must not let the first swallow the
second.** Nobody has put the kit's blue beside Herdr's own in one frame, and nobody has
established how Herdr colours its own dialogs. §7.5.4 carries both halves and both are
load-bearing: the review answers "does this read correctly", which is the question a
palette has, and it does not answer "is this what Herdr does", which nothing has.

💰 **The transferable finding is about cost, not colour.** Three trips to a live Herdr
server failed to settle the palette. The fourth attempt was a local command and settled it
in one pass, because `examples/preview.rs` existed by then. **Build the thing that makes a
check trivial**, and the check happens; leave it a round trip away, and it does not. The
same example is what exposed the button truncation, which nobody had asked it to look for.

⚠️ **Two things in §7.5 are still genuinely unknown, and neither is closed by any of the
above.** Whether click forwarding reaches a **popup** specifically was measured against a
plugin pane rather than a popup, and the `TerminalState` teardown guard has no test. Both
stay flagged exactly as they are.

### 15.6 Extended 2026-09-12: one result type per variant

Sent back by the recent-spaces migration, which measured a tripled binary and attributed
it rather than guessing.

| § | What changed | Kind |
|---|---|---|
| 4.4 | `Client::call` is generic over the result the caller names | 🔑 decision |
| 4.4 | Per variant, never per method, because no method-to-variant mapping is derivable | 🔑 decision |
| 4.4 | **The saving is 1,006,096 bytes, 42% of a consumer's binary** | ✅ measured |
| 4.4 | **The generic signature costs nothing and saves 331,552 bytes** | ✅ measured |
| 3.2 | A fifth stage, splitting the response union, and three more guards | ➕ new |
| 3.3 | The untagged trap, met a third and fourth time | ➕ new |
| 3.5 | typify cannot be driven to a tagged newtype enum, four encodings tried | ✅ measured |
| 3.5 | A bare `true` is a legal schema, and Herdr writes one | ➕ new |
| 4.4 | `tests/response_sweep.rs`, 64 rows, closing a gap as old as the pipeline | ➕ new |

🔑 **The decision worth carrying forward is the one about where a check lives.** Making
each result type refuse its own wrong tag, rather than checking the tag inside `call`,
cost a `type_` field in 64 structs and bought a guarantee that holds wherever the type is
used. This project has chosen that direction every time it has come up.

💰 **And the transferable finding is a negative result about cost.** A generic parameter
on a hot path was expected to cost something, and it paid instead. **Measure the thing
you are about to reason about**, especially when the reasoning is confident: the
decomposition that started this work, the 42% that ended it, and the 331,552 bytes nobody
predicted were all measurements that contradicted a plausible expectation.

⚠️ **What is still unmeasured**: nothing here has run against a live Herdr server, and the
64 per-variant types are exercised by a schema-derived fixture each rather than by a real
answer. A fixture proves the shapes agree with the schema; it does not prove the schema
agrees with the server.
