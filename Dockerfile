# syntax=docker/dockerfile:1

# `source` (default) builds the static musl binary here. `prebuilt` copies the one from a release archive,
# extracted to prebuilt/<arch>/rlsspec, so the published image runs exactly the released binary.
ARG BINARY=source

FROM rust:1.98-alpine AS source
RUN apk add --no-cache musl-dev
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --release --locked && cp target/release/rlsspec /rlsspec

FROM scratch AS prebuilt
ARG TARGETARCH
COPY prebuilt/${TARGETARCH}/rlsspec /rlsspec

FROM ${BINARY} AS binary

# Distroless static: CA certificates, a nonroot user, no shell. The binary also embeds Mozilla's roots.
FROM gcr.io/distroless/static-debian13:nonroot
ARG VERSION=dev
LABEL org.opencontainers.image.title="rlsspec" \
      org.opencontainers.image.description="Check Postgres Row Level Security against a spec of expected access" \
      org.opencontainers.image.source="https://github.com/matheusspacifico/rlsspec" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.version="${VERSION}"
COPY --from=binary /rlsspec /usr/local/bin/rlsspec
USER nonroot:nonroot
WORKDIR /work
ENTRYPOINT ["/usr/local/bin/rlsspec"]
CMD ["--help"]
