# syntax=docker/dockerfile:1

# ─── Stage 1: Build Rust binary ─────────────────────────────────────────────
FROM rust:1-slim-bookworm AS rust-build
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev perl make \
    && rm -rf /var/lib/apt/lists/*
# perl (not just perl-base, which rust:*-slim ships) is required by
# openssl-sys's vendored build: it needs FindBin.pm to run OpenSSL's
# Configure script. Vendoring is intentional (see root Cargo.toml) so
# the release binary has no runtime libssl dependency on Linux.

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY xtask ./xtask
COPY agents ./agents
COPY packages ./packages
# captain-runtime/captain-kernel embed these via include_str! at compile time.
COPY docs/captain-tools ./docs/captain-tools
# captain-api embeds this via include_bytes! (webchat.rs) for the web UI's
# favicon/logo — missing this COPY fails the build with a file-not-found
# from the macro, not a missing-crate error, so it's easy to miss.
COPY assets ./assets

ARG LTO=thin
ARG CODEGEN_UNITS=8
# Thin LTO, not fat: fat LTO + codegen-units=1 (the workspace release
# profile's default) OOM-kills the linker inside memory-capped Docker VMs
# for a workspace this size — scripts/release-all.sh hit and documented the
# exact same failure for cross-compiled Linux targets. Overridable via
# --build-arg for anyone who wants the smaller fat-LTO binary and has the
# memory to spare.
# Without this every container reports the bare crate version (0.1.0),
# making it impossible to know which build a beta tester actually runs.
ARG CAPTAIN_BUILD_VERSION=""
ENV CARGO_PROFILE_RELEASE_LTO=${LTO} \
    CARGO_PROFILE_RELEASE_CODEGEN_UNITS=${CODEGEN_UNITS} \
    CAPTAIN_BUILD_VERSION=${CAPTAIN_BUILD_VERSION}

RUN cargo build --release -p captain-cli

# ─── Stage 2: Final runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*
# nodejs/npm are for agent tools (execute_code language=node, the npm
# package tool), not for a bundled frontend — there is no apps/web in
# this checkout to build or serve.

# Rust binary
COPY --from=rust-build /build/target/release/captain /usr/local/bin/captain
COPY --from=rust-build /build/agents /opt/captain/agents
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-entrypoint.sh

EXPOSE 50051
ENV CAPTAIN_HOME=/root/.captain

# Provision the local embeddings stack (ONNX Runtime dylib + ~90 MB model)
# at build time: there is no Ollama inside the container, so without this
# the daemon has no working embedding driver and Tool RAG silently degrades
# to a static tool filter. Must stay ABOVE the VOLUME declaration — with the
# legacy builder, writes under a declared volume path are discarded at the
# end of each RUN, and these files must land in the image layer so Docker
# copies them into a fresh named volume on first mount.
RUN captain embeddings install

# Persistent data volume
VOLUME /root/.captain

WORKDIR /root
ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["start"]
