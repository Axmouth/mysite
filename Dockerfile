FROM rust:1.96-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/mysite /usr/local/bin/mysite
COPY static ./static

ENV BIND_ADDRESS=0.0.0.0:3000
ENV DATA_DIR=/app/data

VOLUME ["/app/data"]
EXPOSE 3000

CMD ["mysite"]
