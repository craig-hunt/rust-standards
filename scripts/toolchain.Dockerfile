# The toolchain scripts/verify.sh expects, pinned by digest.
#
# A verification that depends on what happens to be installed verifies the
# machine as much as the code. Every image below is addressed by digest, not by
# tag: a tag is a name somebody can repoint, and rust:1-bookworm floats across
# every release of the compiler.
#
# The readable tag sits beside each digest so a reader can tell what the digest
# is, and so a bump is a visible two-line change rather than an opaque one.
#
# gitleaks arrives by copying the binary out of its own published image rather
# than by downloading a release asset during the build. A curl into a container
# image is an unpinned fetch at build time, and this repository audits its
# dependencies precisely so that it does not do that sort of thing.

# zricethezav/gitleaks:v8.30.1
FROM zricethezav/gitleaks@sha256:c00b6bd0aeb3071cbcb79009cb16a60dd9e0a7c60e2be9ab65d25e6bc8abbb7f AS secrets

# rust:1.99.0-bookworm
FROM rust@sha256:59037199c44290f2befcdd58dcc540164763fc296950255aaefeef096a1866b0

# clippy and rustfmt are components rather than contents: even the full image
# ships without them, and rust-toolchain.toml asking for them is not the same as
# having them.
RUN rustup component add clippy rustfmt

# Pinned with --locked so the version here is the version built, rather than
# whatever the registry offers on the day the image is rebuilt.
RUN cargo install --locked cargo-mutants@27.1.0 cargo-deny@0.20.2

COPY --from=secrets /usr/bin/gitleaks /usr/local/bin/gitleaks
