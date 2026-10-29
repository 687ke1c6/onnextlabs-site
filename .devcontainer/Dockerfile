FROM oven/bun:1.4.3 AS bun

FROM mcr.microsoft.com/devcontainers/rust:1-bookworm
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun
RUN ln -s /usr/local/bin/bun /usr/local/bin/bunx

ARG JUST_VERSION=1.40.0
RUN arch="$(uname -m)" \
    && curl -fsSL "https://github.com/casey/just/releases/download/${JUST_VERSION}/just-${JUST_VERSION}-${arch}-unknown-linux-musl.tar.gz" \
    | tar -xz -C /usr/local/bin just
