set dotenv-load

image := "onnextlabs-site"
port := env("PORT", "3000")
files_user := env("FILES_USER", "dev")
files_password := env("FILES_PASSWORD", "dev")
db_token := env("DB_TOKEN", "dev-db-token-0123456789")
export DB_TOKEN := db_token

# list recipes
default:
    @just --list

# install frontend deps
install:
    bun install --cwd web

# run WebDAV file storage (:8090), axum (:3000) and vite (:5173) together; Ctrl-C stops all
dev: install
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    # File storage: reuse one that's already running (e.g. `just files` in another terminal).
    if curl -s -o /dev/null http://127.0.0.1:8090/; then
        echo "WebDAV file storage already running on :8090"
    else
        just files &
    fi
    cargo run &
    bun run --cwd web dev &
    wait -n  # if any server exits (e.g. port in use), stop them all

# run only the axum server
server:
    cargo run

# serve ./data-dav over WebDAV on :8090 (the FILES_URL for dev; `just dev` starts it too)
files:
    mkdir -p data-dav
    RCLONE_USER={{quote(files_user)}} RCLONE_PASS={{quote(files_password)}} rclone serve webdav data-dav --addr 127.0.0.1:8090 --dir-cache-time 5s

# run the database service on :3100 against ./data (the site uses it when DB_URL is set)
db:
    DB_TOKEN={{quote(db_token)}} PORT=3100 cargo run -p db-server

# download a consistent backup from the database service into ./backups (DB_URL, DB_TOKEN)
db-backup:
    mkdir -p backups
    curl -fsS ${DB_PROXY:+--proxy "$DB_PROXY"} -H "Authorization: Bearer $DB_TOKEN" \
        "${DB_URL:-http://127.0.0.1:3100}/v1/backup" \
        -o "backups/onnextlabs-$(date -u +%Y%m%dT%H%M%SZ).db"
    @ls -t backups | head -1

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
    cargo clippy --workspace --all-targets -- -D warnings
    cd web && bunx tsc --noEmit

# run rust tests
test:
    cargo test --workspace

# build the docker image
docker-build:
    docker build -t {{image}} .

# build the database service's docker image
docker-build-db:
    docker build -f crates/db-server/Dockerfile -t {{image}}-db .

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
