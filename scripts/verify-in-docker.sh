#!/usr/bin/env bash
#
# Runs scripts/verify.sh inside a container that already has every tool.
#
# This exists because a verification depending on what happens to be installed
# verifies the machine as much as the code. Pinning the toolchain in an image
# means the gate answers the same way on a laptop with no Rust at all as it does
# in CI.
#
# The registry cache is a named volume, so a second run does not re-download
# every crate. The target directory stays inside the repository, so a developer
# can read a report the run produced.
set -euo pipefail

readonly REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

readonly TOOLCHAIN_IMAGE='rust-standards-toolchain'
readonly TOOLCHAIN_DOCKERFILE='scripts/toolchain.Dockerfile'
readonly REGISTRY_CACHE='rust-standards-cargo'

# Built rather than pulled, because the gate needs the compiler, cargo-mutants,
# cargo-deny and gitleaks in one place and no published image carries all four.
# Docker caches it, so this costs nothing after the first run.
docker build \
  --file "${REPOSITORY_ROOT}/${TOOLCHAIN_DOCKERFILE}" \
  --tag "${TOOLCHAIN_IMAGE}" \
  "${REPOSITORY_ROOT}" > /dev/null

exec docker run --rm \
  --volume "${REPOSITORY_ROOT}:/work" \
  --workdir /work \
  --volume "${REGISTRY_CACHE}:/usr/local/cargo/registry" \
  "${TOOLCHAIN_IMAGE}" \
  ./scripts/verify.sh
