# Tasks for herdr-plugin-kit.
#
# Every recipe here is a one-line wrapper. `just` is not installed on every
# machine that has to be able to regenerate these types, so the real work lives
# in `codegen/` and in cargo, and both can be run directly. Keep it that way:
# a recipe that grows logic of its own becomes a path nobody can test without
# installing `just` first.

# List the available recipes.
default:
    @just --list

# Regenerate the Herdr wire types from a published Herdr release tag.
#
# The tag keeps its leading `v`, because Herdr's own release tags carry one.
# Without `just`, run `python3 codegen/sync_api.py <tag>`, which is the same
# entry point. A human reviews the resulting diff before it lands.
sync-api tag:
    python3 codegen/sync_api.py {{tag}}

# Copy the shell templates into a plugin's bin/ directory.
#
# Nothing is substituted: the files land byte-identical in every plugin, and
# each shim reads that plugin's own name and binary from its own files at run
# time. So a `diff` between two plugins' bin/ directories is drift and nothing
# else, which is the whole reason these live here.
#
# Without `just`, run `python3 templates/sync_bin.py <checkout>`, which is the
# same entry point. A human reviews the diff in that repository before it lands.
sync-bin plugin:
    python3 templates/sync_bin.py {{plugin}}

# Ask whether a plugin's bin/ still matches the kit. Writes nothing.
#
# This is what a plugin's own CI runs, and it is what turns shell drift into a
# failing build instead of a discovery.
check-bin plugin:
    python3 templates/sync_bin.py {{plugin}} --check

# Ask whether a plugin's manifests, its tags and its binary name still agree.
#
# The other half of what a plugin's own CI runs, beside `check-bin`. SCOPE.md
# §11 carries the two-command recipe verbatim, because a paraphrase is how
# three plugins end up running three different checks. Every disagreement this
# names is silent in production: the install still works and simply stops
# downloading.
gate plugin:
    python3 tools/plugin_gate.py versions {{plugin}}

# Run every test: the codegen guards, the shell templates, the mutation
# harness's own tests, the plugin conformance gate, the one block that has to
# live in YAML, then the Rust suite.
test:
    python3 codegen/test_codegen.py
    python3 templates/test_templates.py
    python3 tools/test_mutate.py
    python3 tools/test_plugin_gate.py
    python3 tools/test_kit_pin.py
    cargo test --all-features

# Check the tests actually test: break one thing at a time and confirm the
# suite reddens.
#
# One spec per file, and this takes one of them. `ls tools/mutations` is the
# list, and CI reads that same directory rather than a list anybody maintains.
# The harness reads a single spec by design, so run it once per spec rather
# than teaching this recipe to loop.
#
# Not part of `check`, because it recompiles once per mutation and takes
# minutes rather than seconds. Run it when adding or changing a guard.
#
# Without `just`, run `python3 tools/mutate.py <spec>`, which is the same
# entry point.
mutate spec="tools/mutations/dialog.json":
    python3 tools/mutate.py {{spec}}

# Format every Rust file.
fmt:
    cargo fmt --all

# Check formatting without changing anything.
fmt-check:
    cargo fmt --all -- --check

# Lint with warnings denied.
#
# ⚠️ `--all-features` is load-bearing, not thoroughness. Every feature is off by
# default, so without them clippy never compiles those files and never lints a
# line of them. CI carries the same flag: a gate stricter than what a developer
# runs by hand surprises them in CI instead of at their desk.
#
# 🔑 `--all-features` rather than naming them, because a named list goes stale
# the day a feature is added and says nothing when it does. `update` was added
# to a list that read `--features dialog`.
lint:
    cargo clippy --all-targets --all-features -- -D warnings

# Build the documentation, and fail on anything rustdoc complains about.
#
# 🔑 Not covered by `lint`. `cargo clippy` does not run rustdoc at all, so
# broken intra-doc links, links into private items, and links to a crate that
# is not a dependency are checked nowhere else. One of those survived four
# stages of green checks because nothing had ever built the docs.
#
# ⚠️ Both feature combinations. A link to a `dialog` item resolves only when
# that feature is on, and warns when it is off.
#
# 🪤 `--document-private-items` is not thoroughness. Without it rustdoc never
# resolves a link written on a private item, so a doc comment naming a deleted
# function passes silently. This repository documents its private functions as
# carefully as its public ones.
docs:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items

# Everything that has to be green before a change lands.
check: fmt-check lint docs test
