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
3. Set `ADMIN_EMAILS` to your Google address. `/login` signs you in and lands on `/portal`.

Signing in alone grants nothing. Access comes from:

- **Admins** can do everything, in every organisation. Admin is a flag on the account:
  admins set it for others at `/portal/users` (Manage → Users), which lists everyone who
  has signed in. Verified `ADMIN_EMAILS` addresses are always admins (use it to bootstrap
  the first one). You can't remove your own admin flag. Org roles never make anyone an admin.
- **Organisation membership**: members land on `/portal`, see their organisations, and open
  each at `/portal/<slug>`. Each membership has one **role**, so someone can be an Admin-org in
  one org and a Member in another.
- **Roles** are named sets of permissions, shared by every org and edited at
  `/portal/roles`. Two exist to start with: `Admin-org` (every permission, within
  the orgs where you hold it) and `Member` (`read-docs`).
  Permissions are fixed in code (`src/rbac.rs`):

  | Permission    | Allows, in that org                                                 |
  | ------------- | ------------------------------------------------------------------- |
  | `read-docs`   | Browse documents, download private ones                             |
  | `write-docs`  | Upload documents, create folders                                    |
  | `manage-orgs` | Add and remove members, change their roles (`/portal/organisations`) |
  | `write-roles` | Create, edit and delete roles (site-wide, since roles are shared)   |

  Only admins create organisations. Anyone else can only assign, change or remove a role whose
  permissions they hold themselves in that org. Everyone can see their own roles at
  `/portal/access` (account menu → Your access).

Each organisation's files are stored under `$DATA_DIR/orgs/<slug>/` (documents in
`documents/`).

The same can be done on the command line (people must have signed in once to be added):
```sh
onnextlabs-site grant-admin someone@example.com    # recovery; revoke-admin undoes it
onnextlabs-site create-org "World Health Organisation"   # → /portal/world-health-organisation
onnextlabs-site add-member world-health-organisation someone@example.com   # as Member
onnextlabs-site add-member world-health-organisation someone@example.com --role Admin-org   # or change role
onnextlabs-site remove-member world-health-organisation someone@example.com
onnextlabs-site list-orgs
onnextlabs-site list-users
```

## Build & run with Docker
```sh
docker build -t onnextlabs-site .
docker run --rm -p 3000:3000 -v onnextlabs-data:/app/data --env-file .env onnextlabs-site
```
Mount a volume at `/app/data` (`DATA_DIR`) so the database and organisation files survive redeploys.

UI primitives in `web/src/components/ui` follow the shadcn conventions (cva + tailwind-merge);
`web/components.json` is set up for the shadcn-solid CLI to add more.
