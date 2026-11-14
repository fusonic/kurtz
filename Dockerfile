# syntax=docker/dockerfile:1

ARG RUST_VERSION=1.99
ARG ALPINE_VERSION=3.24

# 1. Fetch the pinned Tabler stylesheet. build.rs verifies it against the SRI
# hash in assets/tabler.lock before it is embedded.
FROM alpine:${ALPINE_VERSION} AS tabler
RUN apk add --no-cache curl
COPY assets/tabler.lock /tabler.lock
RUN . /tabler.lock \
 && mkdir /out \
 && curl -fsSL -o /out/tabler.min.css \
      "https://cdn.jsdelivr.net/npm/@tabler/core@${TABLER_VERSION}/dist/css/tabler.min.css"

# 2. Build a static musl binary with Tabler embedded.
FROM rust:${RUST_VERSION}-alpine${ALPINE_VERSION} AS build
RUN apk add --no-cache musl-dev gcc
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY migrations ./migrations
COPY templates ./templates
COPY assets ./assets
COPY --from=tabler /out/tabler.min.css ./assets/dist/tabler.min.css
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked \
 && cp target/release/kurtz /kurtz

FROM alpine:${ALPINE_VERSION} AS certs
RUN apk add --no-cache ca-certificates

# 3. Minimal runtime: the binary and CA certificates, nothing else.
FROM scratch
COPY --from=certs /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /kurtz /kurtz
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 CMD ["/kurtz", "healthcheck"]
ENTRYPOINT ["/kurtz"]
