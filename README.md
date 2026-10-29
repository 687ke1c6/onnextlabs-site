# OnNext Labs — Coming Soon

Solid + TypeScript + Tailwind v4 + shadcn-style components (`web/`), served by an Axum server (`src/main.rs`).

## Develop (devcontainer)
```sh
cargo run                 # axum on :3000 (serves web/dist, /api/health)
bun --cwd web run dev     # vite on :5173, proxies /api to :3000
```

## Build & run with Docker
```sh
docker build -t onnextlabs-site .
docker run --rm -p 3000:3000 onnextlabs-site
```

UI primitives in `web/src/components/ui` follow the shadcn conventions (cva + tailwind-merge);
`web/components.json` is set up for the shadcn-solid CLI to add more.
