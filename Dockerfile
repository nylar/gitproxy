FROM rust:1.97-slim AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./

RUN mkdir src && echo "fn main() {}" > src/main.rs

RUN cargo build --release

RUN rm -rf src target/release/deps/gitproxy* target/release/gitproxy*

COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/gitproxy /usr/local/bin/

CMD ["gitproxy"]
