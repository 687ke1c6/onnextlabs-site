set dotenv-load

image := "onnextlabs-site"
port := env("PORT", "3000")
files_user := env("FILES_USER", "dev")
files_password := env("FILES_PASSWORD", "dev")

# list recipes
default:
    @just --list

# install frontend deps
install:
    bun install --cwd web

# run axum (:3000) and vite (:5173) together; Ctrl-C stops both
dev: install
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    cargo run &
    bun run --cwd web dev &
    wait -n  # if either server exits (e.g. port in use), stop both

# run only the axum server
server:
    cargo run

# serve ./data-dav over WebDAV on :8090, for FILES_URL=http://127.0.0.1:8090/ (README: File storage)
files:
    mkdir -p data-dav
    RCLONE_USER={{quote(files_user)}} RCLONE_PASS={{quote(files_password)}} rclone serve webdav data-dav --addr 127.0.0.1:8090

# run only the vite dev server
web:
    bun run --cwd web dev

# build frontend and release binary
build: install
    bun run --cwd web build
    cargo build --release

# build everything, then serve web/dist from the release binary
serve: build
    PORT={{port}} ./target/release/onnextlabs-site

# format rust code
fmt:
    cargo fmt

# fmt check, clippy, and typecheck
check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cd web && bunx tsc --noEmit

# run rust tests
test:
    cargo test

# build the docker image
docker-build:
    docker build -t {{image}} .

# build and run the docker image
docker-run: docker-build
    docker run --rm -p {{port}}:3000 {{image}}

# hit the health endpoint
health:
    curl -fsS localhost:{{port}}/api/health

# remove build artifacts
clean:
    cargo clean
    rm -rf web/dist
