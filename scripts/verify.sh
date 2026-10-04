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
# the same way: mutating a statement would re-run a transaction per mutant, and
# the Java sibling's round showed what that produces, which is a run whose
# timeouts score as kills and a number that flatters itself.
#
# What that scope costs is worth stating plainly, because it cost two defects of
# the same kind: the SQL in `infrastructure` is reached by no test here. Those
# statements are exercised by scripts/smoke.sh, against a real database.
announce 'mutation'
require cargo-mutants mutation

# The report directory goes first, so the file the report reads was written by
# this run or not at all. Without this, a run that died partway could leave the
# previous run's report in place and the gate would score work nobody repeated.
rm -rf mutants.out

# cargo-mutants exits non-zero when a mutant survives, which would abort here
# before the report could read what the run measured. The code is kept and handed
# to the report rather than discarded: this step used to end in '|| true', which
# treated a crash, an interrupted run and a tree that would not build as
# indistinguishable from a survivor. The report is still the single gate, and now
# it fails on an exit code nothing in the outcomes explains.
MUTATION_STATUS=0
cargo mutants --package domain --package application --minimum-test-timeout 30 \
  || MUTATION_STATUS=$?
python3 scripts/mutation_report.py "${MUTATION_THRESHOLD}" "${MUTATION_STATUS}"

announce 'dependency audit'
require cargo-deny audit
cargo deny --all-features check

# A secret in history is a secret to rotate, and finding it after a push is
# finding it too late. The Go sibling runs this and neither the C# nor the Java
# one does, which is the gap this closes.
#
# Both the history and the working tree, because they answer different questions
# and this step used to ask only the second one while claiming the first. `git`
# reads every commit, which is what makes a committed secret findable at all;
# `dir` reads what is on disk now, which catches the one a developer has written
# and not yet committed. A shallow clone has no history to read, so the workflow
# fetches all of it.
announce 'secrets'
require gitleaks secrets
require git secrets

# Proof that there is a history to read, before a clean answer from it is
# believed. gitleaks reports no leaks and exits zero when it cannot read the
# repository at all, which it demonstrated here against a bind mount git refused
# as dubiously owned: nought commits scanned, no leaks found, gate passed. These
# two commands fail instead, loudly, on a repository git will not read and on a
# shallow clone whose history is a slice of itself.
git rev-list --count HEAD > /dev/null
if [ -f "$(git rev-parse --git-dir)/shallow" ]; then
  echo 'this clone is shallow, so a history scan would read part of the history' >&2
  echo 'and report on all of it. Fetch the full history first.' >&2
  exit 1
fi

gitleaks git --redact --no-banner .
gitleaks dir --redact --no-banner .

announce 'every gate passed'
