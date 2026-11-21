# syntax=docker/dockerfile:1

# ---- Stage 1: build frontend with bun ----
FROM oven/bun:1.4.3-alpine AS web
WORKDIR /app/web
# deps first for layer caching
COPY web/package.json web/bun.lock* ./
RUN bun install --frozen-lockfile
COPY web ./
RUN bun run build

# ---- Stage 2: build backend on rust:alpine ----
# cargo-chef builds the dependencies from a recipe of the workspace manifests, so that layer
# is cached until Cargo.toml/Cargo.lock change, however many crates the workspace has.
FROM rust:alpine AS chef
RUN apk add --no-cache musl-dev && cargo install cargo-chef --locked --version 0.1.78
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release -p onnextlabs-site --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY migrations migrations
COPY crates crates
RUN cargo build --release -p onnextlabs-site

# ---- Stage 3: minimal runtime ----
FROM alpine:3.20
RUN adduser -D -u 10001 app
WORKDIR /app
COPY --from=builder /app/target/release/onnextlabs-site /usr/local/bin/onnextlabs-site
COPY --from=web /app/web/dist ./web/dist
# The database lives here; mount a volume at /app/data to persist it across deploys.
# Organisation files are on a WebDAV server (FILES_URL).
RUN mkdir -p /app/data && chown app:app /app/data
USER app
ENV PORT=3000 STATIC_DIR=/app/web/dist DATA_DIR=/app/data
EXPOSE 3000
CMD ["onnextlabs-site"]
