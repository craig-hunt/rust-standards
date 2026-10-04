# Two runnable images from one build: the migrator and the API.
#
# A multi-stage build so the shipped layers carry no compiler, no sources and no
# registry cache. What reaches a deployment is two binaries and the runtime they
# need, which is both smaller and a smaller attack surface.
#
# The builder is addressed by digest for the same reason the toolchain image is:
# a file claiming to pin a toolchain while naming a floating tag is worse than
# one that makes no claim.

# rust:1.99.0-bookworm
FROM rust@sha256:59037199c44290f2befcdd58dcc540164763fc296950255aaefeef096a1866b0 AS build
WORKDIR /build

# The manifests land first and resolve on their own layer, so a source change
# does not re-download the dependency tree. Each crate needs a source file to
# exist before cargo will resolve it, hence the placeholders.
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY domain/Cargo.toml domain/
COPY application/Cargo.toml application/
COPY infrastructure/Cargo.toml infrastructure/
COPY web/Cargo.toml web/
COPY conventions/Cargo.toml conventions/
RUN mkdir -p domain/src application/src infrastructure/src conventions/src web/src/bin \
    && touch domain/src/lib.rs application/src/lib.rs infrastructure/src/lib.rs \
             conventions/src/lib.rs web/src/lib.rs \
    && echo 'fn main() {}' > web/src/bin/api.rs \
    && echo 'fn main() {}' > web/src/bin/migrate.rs \
    && cargo fetch --locked

COPY domain/src domain/src
COPY application/src application/src
COPY infrastructure/src infrastructure/src
COPY web/src web/src

# The image build does not run the tests. scripts/verify.sh and the CI workflow
# run them, and the integration gates need tools this image deliberately does not
# carry. An image build that skipped a gate while appearing to run it would be
# worse than one that plainly does not.
RUN cargo build --locked --release --package web

FROM debian:bookworm-slim AS runtime
# The binaries are dynamically linked against the C library and OpenSSL is not
# used, so this is the whole runtime they need.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# An unprivileged user, because nothing either binary does needs root and a
# container escape is cheaper to exploit from an account that has it.
RUN useradd --system --create-home --shell /usr/sbin/nologin standards
USER standards
WORKDIR /app

FROM runtime AS migrator
COPY --from=build /build/target/release/migrate /usr/local/bin/migrate
ENTRYPOINT ["migrate"]

FROM runtime AS api
COPY --from=build /build/target/release/api /usr/local/bin/api
EXPOSE 8080
ENTRYPOINT ["api"]
