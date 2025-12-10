FROM rust:1.91-slim AS build
WORKDIR /build

RUN rustup target add x86_64-unknown-linux-musl && \
    apt-get update && \
    apt-get install -y musl-tools musl-dev && \
    update-ca-certificates

# Build an empty project with only the Cargo files to improve the cache
# performance of the container build. The src directory is expected to change
# most frequently invalidating later caches.
# This downloads and builds the dependencies early allowing built dependencies
# to be cached.
RUN mkdir src && echo 'fn main() {}' > src/main.rs
COPY Cargo.toml Cargo.lock ./

RUN --mount=type=cache,target=/usr/local/cargo/registry cargo build --release --target x86_64-unknown-linux-musl

COPY ./static ./static
COPY ./src ./src
COPY ./templates ./templates

RUN --mount=type=cache,target=/usr/local/cargo/registry <<EOF
    set -e
    # update timestamps to force a new build
    touch src/main.rs
    cargo build --release --locked --target x86_64-unknown-linux-musl
EOF

FROM alpine:3.23

COPY --from=build /build/target/x86_64-unknown-linux-musl/release/glazed glazed

RUN adduser -u 65532 -D -H nonroot

USER nonroot

ENTRYPOINT ["/glazed"]
CMD ["serve"]
