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

# Run every test: the codegen guards first, then the Rust suite.
test:
    python3 codegen/test_codegen.py
    cargo test

# Format every Rust file.
fmt:
    cargo fmt --all

# Check formatting without changing anything.
fmt-check:
    cargo fmt --all -- --check

# Lint with warnings denied.
lint:
    cargo clippy --all-targets -- -D warnings

# Everything that has to be green before a change lands.
check: fmt-check lint test
