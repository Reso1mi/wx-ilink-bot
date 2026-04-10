FROM rust:1.90-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY static ./static

RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/wx-ilink-bot /app/wx-ilink-bot

RUN mkdir -p /app/state

ENV BOT_STATE_DIR=/app/state \
    BOT_LOG_LEVEL=info \
    BOT_HTTP_PORT=3000

EXPOSE 3000

VOLUME ["/app/state"]

CMD ["./wx-ilink-bot"]
