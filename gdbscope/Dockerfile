FROM rust:1-alpine AS builder

RUN apk add --no-cache musl-dev file

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/

ENV RUSTFLAGS="-C target-feature=+crt-static"
RUN cargo build --release --target x86_64-unknown-linux-musl

RUN file target/x86_64-unknown-linux-musl/release/gdbscope | grep -qE "static(-pie)? linked" \
    || (echo "ERROR: binary is not statically linked" && file target/x86_64-unknown-linux-musl/release/gdbscope && exit 1)

FROM scratch
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/gdbscope /gdbscope
