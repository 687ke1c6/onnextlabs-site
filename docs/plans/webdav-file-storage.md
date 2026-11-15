# Plan: WebDAV file storage

Implements [ADR 0001](../adr/0001-file-storage-over-webdav.md). Each step leaves the app
working, and can be reviewed and committed on its own.

## Step 0: rclone in the devcontainer ✅

- `.devcontainer/Dockerfile` installs a pinned rclone release (`RCLONE_VERSION`, currently
  v1.71.2) from GitHub into `/usr/local/bin`, the same way `just` is installed. Debian's
  packaged rclone is several versions behind.
- Takes effect when the container is rebuilt ("Dev Containers: Rebuild Container").
  Until then, step 2's manual checks against real rclone wait for that rebuild.

## Step 1: Storage interface, local only (no visible change) ✅

Done as below, with one difference: the local backend also uploads to a hidden temporary
file in the target folder and commits it without overwriting (using a hard link), the
same as WebDAV will. So the `$DATA_DIR/.uploads` staging folder is gone. The interface is
`list_dirs`, `make_dirs`, `upload` + `commit` (two steps, so numbering can retry names),
`get` and `delete`.

- Add `src/storage/` with a `Storage` type that holds the backend chosen at startup (an
  enum, not a trait object). Operations take paths relative to the storage root, as
  validated segments, never raw strings:
  - `list_dirs(path)`: names of the subfolders, one level.
  - `make_dirs(path)`: create every missing level, parents first.
  - `put(path, body stream, max_bytes)`: write to a temporary name, then move into place
    without overwriting. Returns the size and SHA-256. Cleans up on any error.
  - `get(path, range)`: returns a stream, the total length, and the content range.
  - `delete(path)`.
- `Local` backend: today's behaviour, moved out of `documents.rs`.
- Move `documents.rs` and org creation onto `Storage`. Keep the duplicate-name numbering
  as it is (it's decided by the database).
- Tests: the existing ones still pass. Add unit tests for the local backend: put,
  no-overwrite, cleanup on error, list.

## Step 2: WebDAV backend ✅

Done as below, except that `GET` doesn't forward `Range` yet: that's step 3, for both
backends. Notes:

- Uploads stream through a small channel into the `PUT` body, so the multipart field never
  has to be `'static`. If the upload fails, the `PUT` is aborted (not finished short), and
  the temporary file is deleted.
- Redirects are not followed, so the credentials are only ever sent to `FILES_URL`.
- Status mapping: 404/409 → not found; 401/403/5xx, connection errors and timeouts → 503;
  anything else → 500.
- The `PROPFIND` test reply in `src/storage/testdata/` was recorded from rclone v1.71.2.
- Manual checks against `rclone serve webdav` (2026-11-12), all passed: org creation,
  upload into nested folders with `#`, `%`, `&`, spaces and non-ASCII names, duplicate-name
  numbering, listings, download (byte-identical), oversize upload (413, nothing left),
  upload cut off midway (temporary file deleted), storage down (503 everywhere, a failed
  org creation is rolled back, startup warns but still starts), wrong password (startup
  warns with the 401).


- Config: `FILES_URL`, `FILES_USER`, `FILES_PASSWORD` (clap/env). With `FILES_URL` set,
  use `WebDav`; otherwise `Local`. Log which one at startup.
- Use `reqwest` directly (already in the build via the sign-in library; enable
  `stream`). Add `quick-xml` to read `PROPFIND` responses.
- Build URLs by percent-encoding each path segment.
- One shared client, with a connect timeout and a read timeout.
- Implement the operations as in the ADR's method table:
  - `PROPFIND` with `Depth: 1`, parsing the folder entries (skip the folder itself).
  - `MKCOL` level by level, treating "already exists" (405) as success.
  - `PUT` to `.<name>.partial-<random>` with a streamed body. The size cap and SHA-256
    are applied while streaming.
  - `MOVE` with `Overwrite: F`; if the target already exists (412), report a name
    conflict.
  - `DELETE` the temporary file on any failure.
  - `GET` with `Range` forwarded.
- Map connection errors and timeouts to `503 Storage unavailable`.
- At startup, send one `PROPFIND` to the root. Warn if it fails; don't exit.
- Tests:
  - Unit tests for URL encoding and `PROPFIND` parsing (using recorded XML samples).
  - Manual check against a local `rclone serve webdav` (step 4's recipe):
    - upload, download, ranged download, listing, org creation;
    - an upload cut off midway (this confirms the ADR note on partial files);
    - an oversize upload;
    - storage down.

## Step 3: Range requests on downloads (both backends) ✅

Done as below, plus:

- Downloads send a strong `ETag` (the stored SHA-256). Stored files never change, so
  `If-Range` is checked in the site against that, and isn't passed on to storage. An
  `If-Range` date never matches (no `Last-Modified` is sent), so the whole file is sent.
- Downloads are no longer gzipped: compression would drop `Accept-Ranges`, and ranges
  count stored bytes. The handler marks its responses (`api::Uncompressed`) and the
  compression layer skips them. Site assets are still compressed.
- WebDAV: `Range` is passed on; the reply's `Content-Range` is checked. A server that
  ignores the range (200) gets the whole file sent.
- Checked by hand with `curl -r` on both backends (2026-11-12): `0-99`, `1000-`, `-500`,
  past the end (416 with `bytes */len` and the safety headers), multi-range (200),
  `If-Range` matching (206) and stale (200), `HEAD`.


- Parse `Range` (a single range only; multi-range falls back to `200` with the whole
  file) and `If-Range`.
- Reply `206` with `Content-Range`, or `416` for an unsatisfiable range. Advertise
  `Accept-Ranges: bytes`.
- Keep the attachment, `nosniff` and `CSP: sandbox` headers on every response.
- Tests: unit tests for range parsing; a manual `curl -r` check.

## Step 4: Dev tooling, docs, deployment notes ✅

Done as below. The Railway notes use rclone's `RCLONE_USER`/`RCLONE_PASS` variables rather
than `--user`/`--pass`, so the password isn't in the start command (checked: both work,
and `--addr [::]:…` answers on IPv4 and IPv6). `just files` checked end to end with the
`.env.example` values. `just check`, `cargo test` and the web build all pass.


- `just files`: runs `rclone serve webdav` on `./data-dav` with a dev user and password.
  The README explains how to point `FILES_URL` at it.
- `.env.example`: add the three `FILES_*` variables (commented out, so the default is
  local disk). Add `data-dav` to `.gitignore` and `.dockerignore`.
- README, "File storage" section: both modes, the layout, the rclone command.
- Railway notes, also in the README:
  - an rclone service with a volume, e.g.
    `rclone serve webdav /data --addr [::]:8080 --user … --pass …`;
  - the private-network URL as `FILES_URL`;
  - the IPv6 caveat;
  - `--dir-cache-time`;
  - backing up both volumes.
- Run `cargo fmt --check`, clippy (`-D warnings`), `cargo test` and the web build.

## After step 4

- Drift: listing a folder reconciles it with storage (README, "File storage").
- Local storage removed (ADR 0001, decision 9 superseded). `FILES_URL` is required; `just
  dev` starts rclone; the storage tests run against a real `rclone serve webdav`.

## Out of scope (later)

- ~~Fixing drift between the database and the files.~~ Done after step 4: listing a
  folder reconciles it with storage (README, "File storage").
- Renaming, moving or deleting documents and folders from the portal.
- Moving existing files: they're disposable.
- Signed links.

## Open items

- ~~Confirm rclone's behaviour on an interrupted `PUT`.~~ Confirmed (v1.71.2): it keeps the
  truncated file (256 KiB of a 900 KB upload). The temporary-name-then-`MOVE` approach
  handles this.
- Confirm how Railway's private networking resolves and listens (IPv4 or IPv6). Listening
  on `[::]` covers both, so this only needs a look at the first deploy.
