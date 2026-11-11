# OnNext Labs — Coming Soon

Solid + TypeScript + Tailwind v4 + shadcn-style components (`web/`), served by an Axum server (`src/main.rs`).

## Develop (devcontainer)
```sh
cargo run                 # axum on :3000 (serves web/dist, /api/health)
bun run --cwd web dev     # vite on :5173, proxies /api to :3000
```

## Configuration
All settings are env vars (or matching `--flags`; see `cargo run -- --help`). See
[.env.example](.env.example); copy it to `.env` for local dev.

### Sign-in (Google OAuth)
1. In Google Cloud console, create an OAuth client ID (type: Web application) and add the
   redirect URI `<PUBLIC_URL>/api/auth/callback/google` for each environment
   (e.g. `http://localhost:5173/api/auth/callback/google` and `https://<your-domain>/api/auth/callback/google`).
2. Set `PUBLIC_URL`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`.
3. Set `ADMIN_EMAILS` to your Google address. `/login` signs you in and lands on `/admin`.

Signing in alone grants nothing. Admin access comes from `ADMIN_EMAILS` or from a role granted
on the command line:
```sh
onnextlabs-site grant-role someone@example.com admin   # they must have signed in once
onnextlabs-site revoke-role someone@example.com admin
onnextlabs-site list-users
```

## Build & run with Docker
```sh
docker build -t onnextlabs-site .
docker run --rm -p 3000:3000 -v onnextlabs-data:/app/data --env-file .env onnextlabs-site
```
Mount a volume at `/app/data` so the SQLite database survives redeploys.

UI primitives in `web/src/components/ui` follow the shadcn conventions (cva + tailwind-merge);
`web/components.json` is set up for the shadcn-solid CLI to add more.
