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
FROM rust:alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app

# Backend (cache dependencies with a stub main)
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release && rm -rf src target/release/onnextlabs-site*
COPY src src
RUN touch src/main.rs && cargo build --release

# ---- Stage 3: minimal runtime ----
FROM alpine:3.20
RUN adduser -D -u 10001 app
WORKDIR /app
COPY --from=builder /app/target/release/onnextlabs-site /usr/local/bin/onnextlabs-site
COPY --from=web /app/web/dist ./web/dist
USER app
ENV PORT=3000 STATIC_DIR=/app/web/dist
EXPOSE 3000
CMD ["onnextlabs-site"]
