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

Early. The report and update modules and the CI workflows land in later stages, in the
order `SCOPE.md` section 13 sets out.

| Piece | State |
|---|---|
| `api::generated` — 102 request methods, 187 schema types | ✅ generated and committed |
| `api::Request` — the hand-written envelope | ✅ |
| `api::client` — the socket transport and the protocol handshake | ✅ **never run against a live server**, see below |
| `env` — the reader for Herdr's launch contract | ✅ |
| `version` — what this binary is, and where it came from | ✅ |
| `dialog` — four styled popup states | ✅ feature-gated, off by default |
| `herdr-plugin-kit-build` — the build-script stamp | ✅ |
| Shell templates — `bin/build`, the launcher, and their sync task | ✅ |
| PowerShell templates | ⚠️ shipped **unrun**, see below |
| `report`, `update` | ⏳ held until one real consumer proves the boundaries |
| CI workflows | ⏳ later stage |

Generated against **Herdr `v0.9.0`**, protocol 22, schema version 1.

## Requirements

Rust **1.80** or newer. That is a raise from the 1.75 the three plugins declare today,
and it is not a preference: `cargo-typify` emits `std::sync::LazyLock` for every
pattern-constrained string in Herdr's schema, and that landed in 1.80. The generated
file is never hand-edited, so the floor moves with it.

The crate depends on `serde`, `serde_json`, `regress`, `toml`, and `interprocess`, and it
carries no build script, no build dependencies, and no proc macro of its own. `regress`
arrives with the generated types. `toml` is used only to parse `herdr-plugin.toml`, and
all three donor plugins already depend on it directly, so it costs them nothing new and
its own floor of 1.66 sits well under this crate's.

`interprocess` is the one dependency that costs every consumer something new, because no
donor plugin has it today. It is bought for portability rather than convenience:
`HERDR_SOCKET_PATH` is a Unix socket on Unix and a **named pipe** on Windows, so all
three donors' `UnixStream` clients are Unix-only and none of them could be promoted as
written. It is the same crate Herdr itself depends on, its floor of 1.75 sits under this
crate's, and at run time it pulls in `libc` and nothing else.

`crossterm` reaches a consumer only through the `dialog` feature, which is off by
default.

The build-script stamp is a **second crate**, `herdr-plugin-kit-build`, with no
dependencies at all. It goes in a plugin's `[build-dependencies]` and never reaches the
shipped binary. That separation is the only reason two crates exist rather than one, and
a test in the kit fails if the two are ever wired together.

## Windows is compile-verified only

Windows ships in the first release: the PowerShell shims and all six target triples,
including `aarch64-pc-windows-msvc`. Both decisions are recorded in `SCOPE.md` sections
10.1 and 14.

**Nobody on this project has Windows hardware.** CI proves the code compiles on Windows.
It never proves a plugin runs there. Every Windows path in the Rust — the transport that
has to speak named pipes rather than Unix sockets — is verified by the compiler and by
nothing else.

Two Windows behaviours in `api::client` follow from that, and both are deliberate:

- **There is no default socket off Unix.** `HERDR_SOCKET_PATH` unset on Windows is an
  error naming that variable, not a guess. `interprocess` accepts only paths already
  starting `\\.\pipe\`, so the Unix default cannot be reused, and nobody has measured
  what Herdr names its pipe. An invented default would fail with a message pointing at a
  path Herdr never used, which reads as plausible and sends the reader down the wrong
  road.
- **A receive timeout is matched on two error kinds.** `WouldBlock` is measured on macOS.
  `TimedOut` is the documented Windows mapping and is compile-verified only. Both are
  kept, because dropping the unverifiable one would leave a wedged server hanging on the
  platform nobody can check.

The compile half was run rather than assumed:

```sh
cargo clippy --all-targets --features dialog --target x86_64-pc-windows-msvc -- -D warnings
```

**The PowerShell shims do not reach even that bar.** PowerShell has no compiler and no CI
job, and it is not installed on the machine they were written on, so `templates/bin/*.ps1`
has never been **run or parsed by anything**. Each file says so in its own header, and a
test asserts that every one of them still does. They are deliberately plainer than their
shell counterparts — no progress display, no download-failure classification — because
unverified code should be small.

One Windows question has an answer nobody has confirmed: whether a release asset's name
carries `.exe`. **It does, and that is a recommendation rather than a measurement.** A
file without that extension is not executable on Windows, and somebody downloading from
the releases page should get something that runs.

It is decided in exactly two assignments, and they are tested against each other:
`WINDOWS_ASSET_EXTENSION` in `tools/plugin_gate.py` is what the release workflow
publishes, and `$AssetNameExtension` in `templates/bin/common.ps1` is what the shim asks
for. Reversing the recommendation is one line in each. No shell template names a Windows
asset at all, and tests hold every part of that up, so a producer and a consumer cannot
drift apart here without the suite saying so.

Being wrong about it costs a 404 and a compile, never a wrong binary: the checksum gate
below the fetch does not care what the file was called.

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

Call Herdr:

```rust
use herdr_plugin_kit::api::client::{Client, Socket};
use herdr_plugin_kit::env::Environment;

let env = Environment::from_process();
let socket = Socket::resolve(&env).expect("no socket could be named");
let client = Client::new(socket, "mikebronner.my-plugin");

match client.ping() {
    Ok(handshake) => {
        if let Some(mismatch) = handshake.mismatch() {
            eprintln!("warning: {}", mismatch);
        }
    }
    Err(error) => eprintln!("herdr is not answering: {}", error),
}
```

That example is a doctest on `api::client::Client`, so it is compiled by `cargo test`
rather than asserted here.

**A protocol mismatch is a diagnosis, never a hard failure.** A plugin that still works
must keep working, so the kit hands the finding back and the plugin decides. The
`Display` above writes the whole warning line, naming both protocol numbers, so nobody
has to invent the sentence. Nothing prints by itself: a client that wrote to stderr would
be the first place the kit decided something on a plugin's behalf, and a headless watcher
like recent-spaces would get output it never asked for.

The constants behind that comparison are exported too:

```rust
use herdr_plugin_kit::api::{GENERATED_FOR_HERDR_TAG, GENERATED_PROTOCOL};
```

⚠️ **Do not assume `HERDR_SOCKET_PATH` is set.** Measured on Herdr 0.9.0: a `[[build]]`
hook during `herdr plugin install` receives **zero** `HERDR_*` variables, and a plugin
event hook receives pane and workspace variables but not this one. `Socket::resolve`
falls back to `~/.config/herdr/herdr.sock` on Unix, and `Socket` records which of the two
answered so a connection failure can say.

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

## Reporting a version

A plugin gets `--version` from two pieces. Its `build.rs` calls the stamp:

```rust
fn main() {
    herdr_plugin_kit_build::stamp();
}
```

And it asks for the report through a macro:

```rust
use herdr_plugin_kit::env::Environment;

let environment = Environment::from_process();
print!("{}", herdr_plugin_kit::version_report!("watch", &environment));
```

```text
watch 0.5.0 (a1b2c3d, built 2026-09-11T04:39:22Z)
manifest 0.5.0 at /p/herdr-plugin.toml
built from source on this machine
```

**It has to be a macro.** `env!` resolves in whichever crate it is written in, so a plain
kit function would capture the kit's version and the kit's commit, and every plugin
calling it would report them as its own. The output would still look like a version
report, which is what makes that bug worth a macro to avoid.

**Nothing in the report fails.** Every lookup degrades to a word, because this is what
somebody runs when the plugin is already broken. It never touches the socket either:
`--version` has to answer when the server is down, which is exactly when it gets run.

A plugin that never calls `stamp()` still compiles and still reports. The commit and
build instant just read `unknown`.

## The shell templates

A crate cannot ship `bin/build` or a launcher, so the kit holds them as templates and
syncs them into each plugin:

```sh
just sync-bin ../herdr-plugin-recent-spaces      # or: python3 templates/sync_bin.py …
just check-bin ../herdr-plugin-recent-spaces     # writes nothing, non-zero on drift
```

**Nothing is substituted.** The files land byte-identical in every plugin, so a `diff`
between two plugins' `bin/` directories shows drift and nothing else. That is the whole
point: three hand-maintained copies diverge in silence, one template diverges in a diff
somebody has to read. `--check` is what a plugin's CI runs, and it is what turns drift
into a failing build rather than a discovery.

Each shim therefore reads two facts from the plugin's own files at run time: the binary's
name from `Cargo.toml`'s `[[bin]]` section, and the plugin's own name from
`herdr-plugin.toml`'s **top-level** `id`, after the last dot.

> **The top-level qualifier is not pedantry.** `herdr-plugin.toml` carries further `id`
> keys in `[[panes]]` and `[[actions]]` entries — `id = "picker"`, `id = "apply"`. A
> line-anchored read returns the wrong one in two of the three plugins, silently, and a
> log prefix reading `picker` looks entirely plausible.

Two things the templates fix that the three plugins each got to separately:

- **A Herdr `[[startup]]` command is handed no `$TERM` and no terminal.** Found once,
  fixed three times, three different shapes. It is now `can_draw()` in one file, and
  every caller that draws asks there.
- **A fetched binary records how it arrived**, in a `KEY=value` note beside itself that
  `version::provenance_of` reads back. Without it a downloaded binary reports itself as
  built from source, and then offers a remedy needing a toolchain its user does not have.

The progress display is a seam. `bin/progress` holds it, `bin/build` makes two calls into
it, and the terminal spinner is one named implementation rather than the default. A Herdr
dialog is coming for every path that can reach one — an install compile is measured to
reach none, so the drawn path stays for that case permanently.

## Continuous integration

A crate cannot ship a workflow: Actions only runs files physically present in a repo's
own `.github/workflows/`. So the kit holds two **reusable** workflows, and each plugin
calls one of them. Bumping the pinned ref propagates to all three plugins, the same model
as pinning the crate.

### Conformance, on every push and every pull request

```yaml
# .github/workflows/ci.yml in the plugin
name: CI
on:
  push:
  pull_request:
jobs:
  conformance:
    uses: mike-bronner/herdr-plugin-kit/.github/workflows/plugin-ci.yml@0.1.0
```

There are no required inputs. Every fact it needs is already stated in the plugin's own
manifests, and an input repeating one of them is a second copy that can disagree with the
first. The one optional input is `test_os`, for a plugin whose suite cannot run on Linux.

It runs the suite, the formatting, clippy with warnings denied, a compile of all six
targets, and two conformance checks that have no local equivalent:

- **`bin/` still matches the kit.** Without it the kit is a suggestion, and drift becomes
  a discovery rather than a failing build.
- **The versions and the tag form agree.** `herdr-plugin.toml`, `Cargo.toml`, the binary
  cargo builds, and the repository's release tags all have to say the same thing.

> **Trigger on `push` as well as `pull_request`.** When a pull request cannot compute a
> merge ref against `main`, Actions skips its `pull_request` workflows entirely — no run,
> no error, and the checks simply never appear. The push trigger is what keeps a
> conflicted branch covered.

### Release, on a tag

```yaml
# .github/workflows/release.yml in the plugin
name: Release
on:
  release:
    types: [created]
permissions:
  contents: write
jobs:
  release:
    uses: mike-bronner/herdr-plugin-kit/.github/workflows/plugin-release.yml@0.1.0
    permissions:
      contents: write
```

> 🚨 **The caller must grant `contents: write`.** A called workflow runs on the caller's
> permissions and cannot raise its own, so a caller that omits it fails before the run
> starts. It is needed to create the release and attach the twelve files to it.

It builds six targets on native runners and publishes a raw binary and a `.sha256` beside
it for each, named exactly as `bin/build` asks for them:

```
https://github.com/<owner>/<repo>/releases/download/<version>/
  <binary>-<platform>-<commit12>            and .sha256 beside each
```

Nothing is published unless all six arrive. Five platforms published and a sixth missing
is not a partial success — it is one platform silently compiling on every install, and
nothing would say so.

## Layout

```
crates/herdr-plugin-kit/        the runtime crate
crates/herdr-plugin-kit-build/  the build-script stamp, a build-dependency only
codegen/                        the four-stage pipeline and its tests
templates/                      the shell shims every plugin's bin/ is synced from
tools/                          the mutation harness and the plugin conformance gate
.github/workflows/              the kit's own CI, and the two reusable workflows
SCOPE.md                        the specification
justfile                        task wrappers, all one line each
```

## Testing

```sh
just test          # or run the five entry points the recipe wraps, directly
just check         # formatting, lints, and every suite
```

Five suites, because an untested guard is a claim rather than a check: the codegen
guards, the shell templates, the mutation harness's own tests, the plugin conformance
gate, and the Rust suite. None needs a network, and none of the four Python ones needs
anything beyond the standard library.

The conformance gate's suite runs both sides of every agreement it asserts. It extracts
the release workflow's own asset-naming line and executes it, then asks the real shim
what it would download, and compares the two answers. A test that rebuilt the name in
Python would agree with itself while both sides were wrong together.

The transport suite stands up a scripted server on a real local socket, so the bytes are
genuine even though the peer is not Herdr. That is also how the call timeout is checked:
`set_recv_timeout` existing proves nothing, and one set on the wrong handle only shows
itself against a server that accepts a connection and then says nothing. So the suite
builds exactly that server and times the call.

```sh
just mutate tools/mutations/client.json   # the transport's 17 guards
just mutate tools/mutations/dialog.json   # the dialogs' 25
```

The template suite drives the real shims against fixture plugin trees, on Herdr's own
launchd `PATH` of `/usr/bin:/bin:/usr/sbin:/sbin`, with stub binaries for `cargo`,
`curl`, `git` and `herdr`. It allocates a pty where a terminal is the thing under test.
It never runs PowerShell, because it cannot.

## Licence

MIT. See `LICENSE`.
