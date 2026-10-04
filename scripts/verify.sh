#!/usr/bin/env bash
#
# Every gate, in one command.
#
# The gates are not optional and not ordered by taste. Formatting and lint run
# first because they are the cheapest; the tests run before mutation because a
# broken rule should not wait on an analysis; the audit and the secrets scan run
# last because they reach the network and a reader should see the code gates
# answer before anything leaves the machine.
#
# Requires the Rust toolchain named in rust-toolchain.toml, plus cargo-mutants,
# cargo-deny and gitleaks. On a machine without them, use
# scripts/verify-in-docker.sh, which pins every one by digest.
set -euo pipefail

readonly REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPOSITORY_ROOT}"

readonly MUTATION_THRESHOLD=70

announce() {
  printf '\n=== %s ===\n' "$1"
}

# A tool that is absent fails rather than being skipped, for the same reason a
# gate fails closed: a check that quietly did not run is indistinguishable from
# one that found nothing.
require() {
  if ! command -v "$1" > /dev/null; then
    echo "$1 is not on PATH, so the ${2} gate cannot run." >&2
    echo 'Install it, or use scripts/verify-in-docker.sh which pins it.' >&2
    exit 1
  fi
}

announce 'format'
cargo fmt --all --check

# Warnings are denied in Cargo.toml, so this fails on one. --all-targets covers
# the tests too: a lint that applied only to shipped code would leave the tests
# as the one place the standards do not reach.
announce 'lint'
cargo clippy --workspace --all-targets

announce 'test'
cargo test --workspace

# The conventions crate asserts the rules the README states. Run again by name
# so a failure here reads as a convention broken rather than as a test failing.
announce 'conventions'
cargo test -p conventions

# Mutation analysis, over the two crates that hold the rules.
#
# Scoped to domain and application deliberately, as the C# sibling scopes Stryker
# the same way. The stores and the edge are covered by tests that need a
# database and a socket, and mutating them re-runs those per mutant: the Java
# sibling's round showed what that produces, which is a run whose timeouts score
# as kills and a number that flatters itself.
announce 'mutation'
require cargo-mutants mutation

# cargo-mutants exits non-zero when a mutant survives, which would abort here
# before the report could read what the run measured. Its exit code is allowed
# to pass and the report is the gate, so one place decides and that place reads
# the outcomes rather than a summary line.
cargo mutants --package domain --package application --minimum-test-timeout 30 || true
python3 scripts/mutation_report.py "${MUTATION_THRESHOLD}"

announce 'dependency audit'
require cargo-deny audit
cargo deny --all-features check

# A secret in history is a secret to rotate, and finding it after a push is
# finding it too late. The Go sibling runs this and neither the C# nor the Java
# one does, which is the gap this closes.
announce 'secrets'
require gitleaks secrets
gitleaks dir --redact --no-banner .

announce 'every gate passed'
