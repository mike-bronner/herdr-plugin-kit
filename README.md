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

Early. Every module `SCOPE.md` section 13 plans is now built. The transport has still
**never run against a live server**, which is the gap that matters most: every module
that sends anything is exercised against fakes answering what a measured Herdr answers,
and no further. See below.

| Piece | State |
|---|---|
| `api::generated` — 102 request methods, 64 result types, 187 schema types | ✅ generated and committed |
| `api::Request` — the hand-written envelope | ✅ |
| `api::client` — the socket transport and the protocol handshake | ✅ **never run against a live server**, see below |
| `env` — the reader for Herdr's launch contract | ✅ |
| `version` — what this binary is, and where it came from | ✅ |
| `dialog` — four styled popup states | ✅ feature-gated, off by default |
| `herdr-plugin-kit-build` — the build-script stamp | ✅ |
| Shell templates — `bin/build`, the launcher, and their sync task | ✅ |
| PowerShell templates | ⚠️ shipped **unrun**, see below |
| `report` — the delivery reason, and a pane when nothing was delivered | ✅ feature-gated, off by default. Needs a `[[panes]]` entry in the plugin, which a crate cannot supply |
| `update` — the release check and the refresh | ✅ feature-gated, off by default. Answers a decision and acts only when told to |
| CI workflows | ✅ the kit's own, and `plugin-release.yml` for a plugin. 🔻 No plugin test workflow, see below |

Generated against **Herdr `v0.9.0`**, protocol 22, schema version 1.

## Requirements

**This crate declares no `rust-version`, on purpose.** Plugins built with it ship as
compiled binaries and a user installs one without a toolchain, which is the whole point
of download-by-default. A floor would document a requirement for people who will never
compile, and the number it carried would be whatever the dependency tree currently
demanded rather than anything this project chose.

**What the tree demands today is 1.85**, corrected 2026-09-12 from a weaker reading that
said "more than 1.80". Three crates in the runtime crate's normal tree declare
`edition = "2024"`, which stabilised in Rust 1.85: `regress 0.10.5`, which is a **direct**
dependency and declares no `rust-version` at all, plus `hashbrown 0.17.1` and
`indexmap 2.14.2`, which both declare `1.85` outright and arrive through `toml`.

⚠️ **1.85 is reasoned rather than measured.** No toolchain between 1.80 and 1.97 is
installed here, so nothing has been built at the boundary. What is reproducible is that
1.80 fails. Underneath all of it sits an independent 1.80 floor that moves for its own
reason: the generated types use `std::sync::LazyLock`, which `cargo-typify` emits for
every pattern-constrained string in Herdr's schema. `SCOPE.md` §11.8.1 carries both
halves and says which is which.

⚠️ **One path does still compile**, and it is the reason the number above is written down
at all: `bin/build` falls back to compiling from source when a fetch fails, so a user on
that path with an old toolchain gets a compile error. It is loud and it names itself,
which is why it does not justify a claim that was measurably false.

The crate depends on `serde`, `serde_json`, `regress`, `toml`, and `interprocess`, and it
carries no build script, no build dependencies, and no proc macro of its own. `regress`
arrives with the generated types. `toml` is used only to parse `herdr-plugin.toml`, and
all three donor plugins already depend on it directly, so it costs them nothing new and
its own floor of 1.66 sits well under this crate's.

`interprocess` is the one dependency that costs every consumer something new, because no
donor plugin has it today. It is bought for portability rather than convenience:
`HERDR_SOCKET_PATH` is a Unix socket on Unix and a **named pipe** on Windows, so all
three donors' `UnixStream` clients are Unix-only and none of them could be promoted as
written. It is the same crate Herdr itself depends on, and at run time it pulls in `libc`
and nothing else.

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
herdr-plugin-kit = { git = "https://github.com/mike-bronner/herdr-plugin-kit", tag = "0.4.4" }
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

Name the result you expect:

```rust
use herdr_plugin_kit::api::generated::{PaneListAnswer, PaneListParams, RequestMethod};

let panes = client.call::<PaneListAnswer>(RequestMethod::PaneList(PaneListParams {
    workspace_id: None,
}))?;

for pane in panes.panes {
    println!("{}", pane.pane_id);
}
```

💰 **Naming one result rather than the union saves about a megabyte**, and that is the
saving rather than the whole story. `ResponseResult` carries all 64 shapes Herdr can
answer with, serde generates parsing code for every one, and nothing can drop them while
they are all reachable through one type. Two consumers measured it on macOS arm64 at
`opt-level = "s"` with `strip = true`:

| Plugin | Result types named | Narrowing saved |
|---|---|---|
| recent-spaces | 1 | 1,006,096 bytes, **43.9%** |
| project-finder | 7 | 1,103,760 bytes, **27.7%** |

🚨 **What the kit costs is a separate question from what narrowing saves, and the answer
is not the same for every plugin.** project-finder narrowed every call site it has and
its binary still grew **66%** against the hand-written client it replaced. recent-spaces
narrowed its one and came out ahead. The difference is not the saving — those are within
10% of each other in bytes — it is everything else the kit brings: a fixed floor of
static tables, a regex engine and a transport that every consumer pays once, plus a
per-type cost that scales with how much of the API you touch.

⚠️ **So do not read either percentage as yours.** `SCOPE.md` §4.4 has both measurements,
the attribution behind them, and what is still unmeasured — including where the crossover
sits for a plugin that names most of the 64, which nobody has measured at all.

🚨 **The kit may make your binary bigger even when you do everything right**, and
project-finder is the worked example above. What it buys you is one description of
Herdr's wire format, generated from Herdr's own schema, instead of a hand-maintained one
that drifts. Whether that is worth the bytes is yours to decide with the numbers in front
of you rather than a slogan.

🚨 **Upgrading the pin without naming a result makes your binary bigger, not smaller.**
The per-variant types are 128 new types every consumer compiles, and nothing recovers
them until a call site asks for one. ✅ Measured 2026-09-12 by the first real migration:
the same plugin built **3,257,600 bytes on 0.1.0 and 3,350,544 on 0.2.0** with its call
sites untouched, then **1,878,832** once one named `WorkspaceListAnswer`. There is no
signal when you stop halfway — the build succeeds and the plugin works. `SCOPE.md` §13
carries this beside the migration order.

The union is still there, and `client.call::<ResponseResult>(…)` still works. It is a
narrower option rather than a replacement, so a caller that genuinely wants any answer
can still say so and pay for it on purpose.

🚨 **Which result a method answers with is yours to establish, and the schema cannot help
you.** It declares no link between the two, and the obvious guess is wrong on the first
plugin that looked: `workspace.move` answers `workspace_list`, carrying the sidebar after
the move, and `workspace_moved` is not a result type at all. A wrong guess is a
`CallError::Protocol` that names the method, the tag that arrived, and the tag expected —
which is the correction you need, rather than a wrong parse.

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

⚠️ **The generated types diverge from the published schema in exactly one place, on
purpose.** Herdr declares `format: float` eight times and `double` never, `cargo-typify`
maps the first to `f32` correctly, and an `f32` cannot hold what the server sends: a
ratio of `0.69` reads back as `0.6899999976158142`. Measured 2026-09-12 against a live
0.9.0 server. The pipeline widens those eight to `double` before generating, so a reader
comparing `f64` against `float` is looking at a decision rather than a defect. `SCOPE.md`
§3.2.1 carries the measurement.

**Never read a green round-trip test as evidence that these types are correct.**
`tests/method_sweep.rs` is the test that actually discriminates: it sweeps all 102
discriminators and asserts each reaches its own named variant.

The response side is generated twice from one description: once as `ResponseResult`, the
union of all 64 shapes, and once as one type per variant — `PongAnswer`,
`PaneListAnswer`, `OkAnswer`, and 61 more. Both come out of the same pass over the same
schema branch, and the union is byte-identical to what it was before the per-variant
types existed.

`tests/response_sweep.rs` is what holds the two together. Per variant it asserts that the
discriminator reaches its own union variant, that both renderings agree on the wire, and
that an answer tagged for another variant is refused. That last one is what makes naming
a narrow type safe rather than optimistic.

## Regenerating

```sh
just sync-api v0.9.0
```

Or, where `just` is not installed:

```sh
python3 codegen/sync_api.py v0.9.0
```

Both run the same module. Five stages: fetch the schema at that tag, extract the five
sub-schemas into one document, lift the request envelope's `id`, split the response union
into one type per variant, and generate. Output is byte-identical across runs for a given
tag and a given `cargo-typify`.

Regenerating needs `cargo-typify` 0.8.0 (`cargo install cargo-typify`), Python 3.9 or
newer, and network access. **Building the crate needs none of the three.**

A human reviews the diff before it lands. That is the whole review gate for a Herdr
upgrade, so it is not a formality.

### When the pipeline stops

It is built to stop rather than guess. Seven schema changes halt it on purpose, each
with a message naming what changed and what to do:

- a `$ref` that points outside its own sub-schema, which would mean Herdr had started
  sharing definitions across sub-schemas
- a type name defined twice with two different bodies, which is either a shared type
  changed on one side only or a new type that took a taken name
- a change to the request envelope's `id` property, which `envelope.rs` hand-writes and
  therefore cannot track by itself
- a response union that is no longer a choice of branches
- a response branch with no required `const` tag, which no result type could refuse a
  wrong answer for
- a result type name Herdr has taken for itself, which would otherwise replace a type the
  generated file refers to
- a schema that declares no fractional number at all, which would mean the one deliberate
  divergence above had quietly stopped happening

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

### Building from source while you work on a plugin

Create an empty `BUILD_FROM_SOURCE` file in the plugin root, and that checkout compiles
instead of downloading:

```sh
touch BUILD_FROM_SOURCE     # this tree builds from source
rm BUILD_FROM_SOURCE        # back to fetch-or-build
```

🔑 **A file rather than an environment variable, because Herdr runs as a launchd agent.**
A shell export never reaches a script Herdr launches, a file works whoever started the
process, and somebody who has never read the shim can still find it in a directory
listing.

⚠️ **Do not commit it.** Every install of that release would then compile from source,
the plugin would still work, and the only signal would be one line in a server log during
an install nobody is watching. `sync-bin` appends the entry to the plugin's root
`.gitignore`, and `check-bin` fails when it is missing:

```
# The kit's developer override: its presence forces a source build.
/BUILD_FROM_SOURCE
```

That `.gitignore` is the one file the sync task writes that the plugin owns rather than
the kit, so the edit is append-only: nothing already in it is rewritten or reordered, and
a second sync adds nothing.

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

**The kit runs your release and none of your tests.** It ships two gates you run
yourself, against a kit you check out at your own pin, and it publishes your assets
through a reusable workflow you call.

> 🔻 **A second reusable workflow ran the tests, and it is gone.** Not because it could
> not work: the kit does not run another repository's tests, which is a scope decision.
> ⚠️ It was deleted believing the problem was technical — a called workflow is told
> nothing about which of its own versions a caller pinned — and that turned out to be
> solvable. It reads the pin out of your `Cargo.toml`, because your repository is the
> one checked out in front of it. `SCOPE.md` §11.2.1 has the measurement and the
> correction.

### The two gates, in the plugin's own CI

**Copy this rather than paraphrasing it.** A recipe somebody restates is how three
plugins end up running three different checks.

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

Three things in it are load-bearing:

- 🚨 **`fetch-depth: 0`.** The version gate reads tag history, and a shallow clone lets it
  pass by seeing no releases at all.
- **The tag comes from your own dependency pin**, read through `cargo metadata` rather
  than out of the TOML, so it works for a workspace-inherited dependency too and needs no
  network.
- **A pin that is not a tag stops the job**, rather than checking out nothing. A branch or
  a commit pin has no version to check against.

What the two commands settle:

- **`bin/` still matches the kit.** Without it the kit is a suggestion, and drift becomes
  a discovery rather than a failing build.
- **The versions and the tag form agree.** `herdr-plugin.toml`, `Cargo.toml`, the binary
  cargo builds, and the repository's release tags all have to say the same thing.

Your suite, your formatting and your lint are your own business: they need nothing from
the kit.

> **Trigger on `push` as well as `pull_request`.** When a pull request cannot compute a
> merge ref against `main`, Actions skips its `pull_request` workflows entirely — no run,
> no error, and the checks simply never appear. The push trigger is what keeps a
> conflicted branch covered.

### Releasing a plugin

**The kit still publishes releases**, which is the one thing it runs for a plugin. It has
to: download-by-default means a plugin that publishes nothing has every install compiling
from source.

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
    uses: mike-bronner/herdr-plugin-kit/.github/workflows/plugin-release.yml@0.4.4
    permissions:
      contents: write
```

> 🚨 **The caller must grant `contents: write`.** A called workflow runs on the caller's
> permissions and cannot raise its own, so a caller that omits it fails before the run
> starts. It is needed to create the release and attach the twelve files to it.

It reads which kit to use from your own `Cargo.toml` pin, the same way the recipe above
does, then builds six targets on native runners and publishes a raw binary and a `.sha256`
beside it for each, named exactly as `bin/build` asks for them:

```
https://github.com/<owner>/<repo>/releases/download/<version>/
  <binary>-<platform>-<commit12>            and .sha256 beside each
```

Nothing is published unless all six arrive. Five platforms published and a sixth missing
is not a partial success — it is one platform silently compiling on every install, and
nothing would say so.

🚨 **If one target cannot compile, you get no release at all, and the run is red.** A
failed leg fails the build job, the publish job needs it and is skipped, and nothing
creates a release or uploads a file. That is deliberate: five platforms published and a
sixth missing is one platform compiling on every install forever, with nothing to say so.
⚠️ So a plugin that cannot build for every target cannot release at all until it can —
project-finder split its two Unix-only reaches behind `#[cfg]` for exactly that reason.

✅ **It works, measured 2026-09-13.** project-finder's 0.9.0 published **12 assets**, six
platforms with a `.sha256` beside each, every name keyed on the same commit. It then
fetched what its own shim would ask for and matched the checksum.

> 🚧 **No leg has failed in a real run.** The success path is measured; the failure chain
> above is read off the workflow rather than watched.

## Layout

```
crates/herdr-plugin-kit/        the runtime crate
crates/herdr-plugin-kit-build/  the build-script stamp, a build-dependency only
codegen/                        the five-stage pipeline and its tests
templates/                      the shell shims every plugin's bin/ is synced from
tools/                          the mutation harness and the plugin conformance gate
.github/workflows/              the kit's own CI, fast and slow
SCOPE.md                        the specification
justfile                        task wrappers, all one line each
```

## Testing

```sh
just test          # or run the five entry points the recipe wraps, directly
just check         # formatting, lints, and every suite
```

Six suites, because an untested guard is a claim rather than a check: the codegen guards,
the shell templates, the mutation harness's own tests, the plugin conformance gate, the
kit-pin resolution, and the Rust suite. None needs a network, and none of the five Python
ones needs anything beyond the standard library.

The kit-pin suite is the odd one. Every other piece of CI logic lives in `tools/` because
YAML cannot be run, and this one cannot: it decides which kit to check out, so it runs
before there is a `tools/` to call. So the test goes to it — extracting the block from the
release workflow and from both copies of the recipe, proving all three are identical, and
running the real text against what `cargo metadata` actually answers.

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
just mutate tools/mutations/client.json    # the transport's 20 guards
just mutate tools/mutations/dialog.json    # the dialogs' 24
just mutate tools/mutations/report.json    # the issue reports' 19
just mutate tools/mutations/surface.json   # the shared seam's 2
just mutate tools/mutations/update.json    # the update check's 19
```

CI reads that directory rather than a list of those five paths. A spec added
without a matrix entry is a module nobody checks, and that already happened
once: `update` shipped with no spec at all and the matrix stayed green.

The template suite drives the real shims against fixture plugin trees, on Herdr's own
launchd `PATH` of `/usr/bin:/bin:/usr/sbin:/sbin`, with stub binaries for `cargo`,
`curl`, `git` and `herdr`. It allocates a pty where a terminal is the thing under test.
It never runs PowerShell, because it cannot.

## Licence

MIT. See `LICENSE`.
