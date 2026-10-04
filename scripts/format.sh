#!/usr/bin/env bash
#
# Applies the formatting the build checks.
#
# scripts/verify.sh fails on an unformatted file rather than fixing it, because a
# build that edits the tree is a build that produces different output on a second
# run. This is the command that does the editing, run on purpose.
set -euo pipefail

readonly REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPOSITORY_ROOT}"

cargo fmt --all
