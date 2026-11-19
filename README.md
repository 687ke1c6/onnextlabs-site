# OnNext Labs — Coming Soon

Solid + TypeScript + Tailwind v4 + shadcn-style components (`web/`), served by an Axum server (`src/main.rs`).

## Develop (devcontainer)
```sh
just dev                  # all of the below, together
just files                # WebDAV file storage on :8090 (./data-dav), see File storage
cargo run                 # axum on :3000 (serves web/dist, /api/health)
bun run --cwd web dev     # vite on :5173, proxies /api to :3000
```
Copy `.env.example` to `.env` first: the server needs `FILES_URL` and won't start without it.

## Configuration
All settings are env vars (or matching `--flags`; see `cargo run -- --help`). See
[.env.example](.env.example); copy it to `.env` for local dev.

### Sign-in (Google OAuth)
1. In Google Cloud console, create an OAuth client ID (type: Web application) and add the
   redirect URI `<PUBLIC_URL>/api/auth/callback/google` for each environment
   (e.g. `http://localhost:5173/api/auth/callback/google` and `https://<your-domain>/api/auth/callback/google`).
   For [Statements](#statements) in Google Sheets, also add `<PUBLIC_URL>/api/statements/google/callback`.
2. Set `PUBLIC_URL`, `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`.
3. Set `ADMIN_EMAILS` to your Google address. `/login` signs you in and lands on `/portal`.

Signing in alone grants nothing. Access comes from:

- **Admins** can do everything, in every organisation. Admin is a flag on the account:
  admins set it for others at `/portal/users` (Manage → Users), which lists everyone who
  has signed in. Verified `ADMIN_EMAILS` addresses are always admins (use it to bootstrap
  the first one). You can't remove your own admin flag. Org roles never make anyone an admin.
- **Organisations** are created by admins. Their name can be changed later (Manage →
  Organisations, by admins or anyone with `manage-orgs` there); their URL name can't, as it's
  also their folder in file storage. Admins can delete one there too: a preview lists
  everything that goes with it (worked out from the database's foreign keys), and the name is
  typed to confirm. Memberships, document entries, statements, types and rules are deleted;
  accounts stay, and files in storage aren't touched.
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
  | `read-statements`  | View saved bank statements; add them to your own Google Sheet  |
  | `write-statements` | Upload and delete bank statements; edit the type rules         |
  | `manage-orgs` | Add and remove members, change their roles (`/portal/organisations`) |
  | `write-roles` | Create, edit and delete roles (site-wide, since roles are shared)   |

  Only admins create organisations. Anyone else can only assign, change or remove a role whose
  permissions they hold themselves in that org. Everyone can see their own roles at
  `/portal/access` (account menu → Your access).

Each organisation's files are stored under `orgs/<slug>/` (documents in `documents/`) on
the WebDAV file server: see [File storage](#file-storage).

The same can be done on the command line (people must have signed in once to be added):
```sh
onnextlabs-site grant-admin someone@example.com    # recovery; revoke-admin undoes it
onnextlabs-site create-org "World Health Organisation"   # → /portal/world-health-organisation
onnextlabs-site rename-org world-health-organisation "WHO"   # the name only; the URL stays
onnextlabs-site add-member world-health-organisation someone@example.com   # as Member
onnextlabs-site add-member world-health-organisation someone@example.com --role Admin-org   # or change role
onnextlabs-site remove-member world-health-organisation someone@example.com
onnextlabs-site list-orgs
onnextlabs-site list-users
```

### File storage
Organisation files live on a WebDAV server: a stock [`rclone serve webdav`], with its own
volume in production. The site still decides who may do what; uploads and downloads stream
through it. Only the database lives in `DATA_DIR`.

The layout is the folder tree people see in the portal, so the store can be browsed
directly: `orgs/<slug>/documents/<folder>/<file>`. Uploads appear under their name
only once complete (they're written to a hidden `.<name>.partial-…` file first, then
renamed), and a file is never overwritten. The design is in
[ADR 0001](docs/adr/0001-file-storage-over-webdav.md).

**The files in storage are the source of truth.** The database only adds details to them
(uploader, upload date, checksum, public flag); folders aren't tracked in it at all. Files
can be added, replaced or removed on the storage host directly, and each folder is brought
back in line when someone opens it in the portal:

- A file without a database entry gets one: **private**, no uploader, dated when it was found.
- A file whose size changed takes the new size (and loses its checksum).
- An entry whose file is gone is shown as **File missing** and can't be downloaded. Anyone
  with `write-docs` can delete it from the list. Entries whose file exists can't be deleted
  from the portal.

Hidden files (names starting with `.`) are never listed.

| Variable         | Meaning                                                            |
| ---------------- | ------------------------------------------------------------------ |
| `FILES_URL`      | WebDAV root, e.g. `http://127.0.0.1:8090/`. Required               |
| `FILES_USER`     | Basic auth user                                                    |
| `FILES_PASSWORD` | Basic auth password                                                |

In dev, `just files` (also started by `just dev`) serves `./data-dav` on `127.0.0.1:8090`
as user `dev`, password `dev` (or your `FILES_USER`/`FILES_PASSWORD`), which is what
`.env.example` points at. The storage tests (`cargo test`) start their own rclone, so it
must be installed; it is in the devcontainer.

If the WebDAV server is down or rejects the credentials, the site still starts (with a
warning in the log) and file requests answer `503` until it's back.

#### On Railway
1. Add a service for the files, from the Docker image `rclone/rclone:1.71.2`, with:
   - a volume mounted at `/data`;
   - start command `serve webdav /data --addr [::]:8080`. Listen on `[::]`: Railway's
     private network can use IPv6 addresses, and `[::]` accepts IPv4 too;
   - variables `RCLONE_USER` and `RCLONE_PASS` (rclone reads its flags from these, which
     keeps the password out of the start command);
   - no public domain. It's only reached over the private network, which is why plain
     HTTP with Basic auth is acceptable.
2. On the site, set `FILES_URL=http://${{<files service>.RAILWAY_PRIVATE_DOMAIN}}:8080/`,
   and `FILES_USER`/`FILES_PASSWORD` to the same credentials.
3. Back up **both** volumes: the site's (database) and the files service's (files). A
   restore should use copies taken at about the same time, as the database refers to files.

rclone caches directory listings for 5 minutes (`--dir-cache-time`). Changes made through
rclone (by the site, or by someone using the WebDAV share) show up at once. Changes made on
the volume underneath it show up only when the cache expires, so if people do that, add
`--dir-cache-time 10s` (or similar) to the start command. `just files` uses 5 seconds, so
files you add to `data-dav/` by hand show up quickly.

[`rclone serve webdav`]: https://rclone.org/commands/rclone_serve_webdav/

### Statements
Each organisation has a **Statements** section (`/portal/<slug>/statements`, needs
`read-statements`), built around **accounts**:

- An **account** has a name (required) and an optional identifier, usually the bank account
  number. Nothing else in Statements is available until the organisation has one; people with
  `write-statements` create, edit and delete accounts (Statements → Accounts, or "New account…"
  in the picker). Names and identifiers are unique within the organisation. Deleting an account
  shows a preview first and deletes its saved statements, rows and type rules; documents stay.
- The page header names the **account you're working in**; the picker at the top right switches
  (remembered per browser).
- **Transactions**: every row saved in the account, in a grid you can search, filter (month and
  type from their column headers), sort (shift-click for more than one column) and page
  through, with a status bar (count, sum/average/min/max of Amount, paging, Compact and Wide).
  `month` ("Aug 2024") is worked out from the date, exactly as the Google Sheet's is. Click a row
  for its details, type and attached documents.
- **Uploads**: drop a bank statement CSV (only Standard Bank CSV so far) to save it into the
  account. When the file states an account number, the preview says whether it **matches** the
  account's identifier. A mismatch doesn't stop the save, but the button becomes "Save to …
  anyway", and if another account has that identifier it offers to switch.
- Each row's id is the n8n hash of its date, description and amount, the same id the Google
  Sheets use. A row is stored once per account, so saving a statement again (or one that
  overlaps) updates rows instead of duplicating them.
- **Types** are a list per organisation, shared by every account: add, rename (renames it on
  every row and rule) or delete one nobody uses. People with `write-statements` can pick a
  row's type in the grid or in the upload preview; a picked type is *hand-set* and saving a
  statement never changes it. Clearing it lets the next save type the row from the rules.
- **Type rules** belong to an account (Statements → Type rules, for the selected account) and
  set each row's `type` when a statement is saved into it. They're tried in order and the
  first match wins; a row no rule matches keeps the type it already had.
- **Documents on rows**: attach any existing document from Documents to a row (`write-statements`
  and `read-docs`), or remove the link; the document itself is never changed.
- The original file is always kept in Documents, in `Statements/<account name>`, saved on the
  uploader's behalf and **private** (only members with `read-docs` can open it). Saving a file
  that's already in Documents reuses that document instead of adding a copy.
- Deleting a saved statement (after a preview of what goes) deletes its rows that no other
  saved statement contains, with their attached-document links. Its file and attached
  documents stay in Documents.

From a saved statement, anyone with `read-statements` can **send it to their own Google
Sheet**. "Connect Google Sheets" asks Google for access to their spreadsheets (separately
from signing in), then they pick or create a spreadsheet and tab. Every row is sent: rows
whose id isn't in the tab are added (with a header row on an empty tab), and rows already
there get their `type` cell updated. Nothing else in an existing row is changed, and a blank
type never replaces one. The connection is held in server memory for about an hour and
never stored; a restart forgets it.

Google Sheets uses the sign-in OAuth client (`GOOGLE_CLIENT_ID`/`GOOGLE_CLIENT_SECRET`), so
it's on whenever sign-in is. In Google Cloud console:
1. Add the redirect URI `<PUBLIC_URL>/api/statements/google/callback` to the client.
2. Enable the **Google Sheets API** and the **Google Drive API** for the project.
3. On the OAuth consent screen, add the scopes `.../auth/spreadsheets` and
   `.../auth/drive.metadata.readonly`. `spreadsheets` is a *sensitive* scope: while the
   app's publishing status is **Testing**, only its test users (up to 100) can connect, and
   they see an "unverified app" notice once. Opening it to everyone needs Google's
   verification. Signing in to the portal is unaffected either way: it only asks for
   identity.

## Build & run with Docker
```sh
docker build -t onnextlabs-site .
docker run --rm -p 3000:3000 -v onnextlabs-data:/app/data --env-file .env \
  -e FILES_URL=http://<webdav-host>:8080/ onnextlabs-site
```
Mount a volume at `/app/data` (`DATA_DIR`) so the database survives redeploys. Files are on
the WebDAV server (`FILES_URL`), which must be reachable from the container: `127.0.0.1` in
`.env` means the container itself. See [On Railway](#on-railway) for running rclone
alongside.

UI primitives in `web/src/components/ui` follow the shadcn conventions (cva + tailwind-merge);
`web/components.json` is set up for the shadcn-solid CLI to add more.
