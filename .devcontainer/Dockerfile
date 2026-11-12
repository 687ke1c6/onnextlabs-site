FROM oven/bun:1.4.3 AS bun

FROM mcr.microsoft.com/devcontainers/rust:1-bookworm
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun
RUN ln -s /usr/local/bin/bun /usr/local/bin/bunx

ARG JUST_VERSION=1.40.0
RUN arch="$(uname -m)" \
    && curl -fsSL "https://github.com/casey/just/releases/download/${JUST_VERSION}/just-${JUST_VERSION}-${arch}-unknown-linux-musl.tar.gz" \
    | tar -xz -C /usr/local/bin just

# WebDAV file storage for local dev (`just files`); see docs/adr/0001-file-storage-over-webdav.md.
# Pinned release: Debian's packaged rclone is several versions behind.
ARG RCLONE_VERSION=v1.71.2
RUN apt-get update && apt-get install -y --no-install-recommends unzip \
    && rm -rf /var/lib/apt/lists/* \
    && case "$(uname -m)" in x86_64) arch=amd64 ;; aarch64) arch=arm64 ;; *) echo "unsupported arch" >&2; exit 1 ;; esac \
    && curl -fsSL -o /tmp/rclone.zip "https://github.com/rclone/rclone/releases/download/${RCLONE_VERSION}/rclone-${RCLONE_VERSION}-linux-${arch}.zip" \
    && unzip -j -o /tmp/rclone.zip "rclone-${RCLONE_VERSION}-linux-${arch}/rclone" -d /usr/local/bin \
    && chmod 755 /usr/local/bin/rclone \
    && rm /tmp/rclone.zip
