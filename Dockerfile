# The LinkJar PDS image (docs/SPEC.md section 14.5).
#
# The build stage compiles the binary with the pinned toolchain; the runtime
# stage holds the binary, CA certificates and nothing else. The server runs
# as uid 1000, the uid of the reference image, so a data directory moves
# between the two without a change of owner (section 8.2).

FROM rust:1.99-slim-trixie AS build
WORKDIR /src
# rust-toolchain.toml follows "stable"; the image tag is the pin here.
ENV RUSTUP_TOOLCHAIN=1.99.0
COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY xtask xtask
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --package linkjar-pds \
 && cp target/release/linkjar-pds /linkjar-pds

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install --yes --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 1000 --create-home pds \
 && mkdir -p /app/data \
 && chown pds /app/data
COPY --from=build /linkjar-pds /usr/local/bin/linkjar-pds

VOLUME /app/data
EXPOSE 3000
ENV PDS_PORT=3000
USER pds
# The same check the parity stack runs: the health route of this server.
HEALTHCHECK --interval=10s --timeout=5s --retries=6 CMD ["linkjar-pds", "health"]
CMD ["linkjar-pds", "serve"]

LABEL org.opencontainers.image.source=https://github.com/linkjar/pds
LABEL org.opencontainers.image.description="LinkJar PDS: an AT Protocol Personal Data Server in Rust"
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"
