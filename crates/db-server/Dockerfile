# syntax=docker/dockerfile:1
# Build from the repo root: docker build -f crates/db-server/Dockerfile .
# (On Railway: RAILWAY_DOCKERFILE_PATH=/crates/db-server/Dockerfile.)

FROM rust:alpine AS chef
RUN apk add --no-cache musl-dev && cargo install cargo-chef --locked --version 0.1.78
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release -p db-server --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY migrations migrations
COPY crates crates
RUN cargo build --release -p db-server

FROM alpine:3.20
RUN adduser -D -u 10001 app
WORKDIR /app
COPY --from=builder /app/target/release/db-server /usr/local/bin/db-server
# The database file lives here: mount a volume at /app/data.
RUN mkdir -p /app/data && chown app:app /app/data
USER app
# DB_TOKEN must be set (16+ characters).
ENV PORT=3100 DATA_DIR=/app/data
EXPOSE 3100
CMD ["db-server"]
