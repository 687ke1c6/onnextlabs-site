# ADR 0001: Store organisation files on a WebDAV server

- **Status:** Proposed
- **Date:** 2026-11-12

## Context

Organisation files (today: the document store) live on the site's own disk, under
`$DATA_DIR/orgs/<slug>/documents/...`. We want those bytes on a separate host, so that
storage can be sized, backed up and inspected independently of the app.

Requirements:

- The stored files stay **browsable**: the layout on the storage host is the same folder
  tree people see in the portal.
- The site stays the single place that decides **who may do what**. The storage host
  knows nothing about users, orgs or permissions.
- Keep it simple. No folder table, no renaming files to ids. Drift between the database
  and the files is accepted for now and handled separately later.
- No signed links: the browser only ever talks to the site.

## Decision

1. **The storage host speaks WebDAV.** We run a stock server, [`rclone serve webdav`],
   on a filesystem volume. We don't write our own file server.
2. **The layout mirrors the portal.** Files live at the same paths as today, relative to
   the WebDAV root: `orgs/<slug>/documents/<folder>/<file>`.
3. **The site proxies and streams.** Uploads and downloads pass through the site, which
   checks permissions first. Bodies are streamed in both directions, never buffered
   whole in memory. The upload size limit and the SHA-256 checksum stay in the site.
4. **The database keeps the document details it has today** (uploader, upload date,
   size, checksum, public flag), keyed by org, folder and file name, as now. Folders are
   not tracked in the database: folder listings come from the storage host.
5. **Only a small part of WebDAV is used**:

   | Method          | Used for                                                            |
   | --------------- | ------------------------------------------------------------------- |
   | `PROPFIND` (one level deep) | List a folder's subfolders                              |
   | `MKCOL`         | Create a folder, one level per call (parents first)                 |
   | `PUT`           | Upload to a temporary name (see 6)                                  |
   | `MOVE`          | Rename the finished upload to its real name, never overwriting      |
   | `GET` / `HEAD`  | Download, including ranged reads                                    |
   | `DELETE`        | Remove a failed or abandoned upload                                 |

   Locks, properties and deep (whole-tree) listings are not used.
6. **Uploads become visible only when complete.** The site `PUT`s to a hidden temporary
   name in the target folder (`.<name>.partial-<random>`), then `MOVE`s it to the real
   name with `Overwrite: F`. If the upload fails, is cut off, or goes over the size
   limit, the site `DELETE`s the temporary file. This doesn't rely on how the WebDAV
   server handles interrupted uploads (see Notes). As a side effect, an existing file is
   never overwritten.
7. **Downloads support byte ranges.** The site forwards `Range`/`If-Range` and returns
   `206 Partial Content`, so downloads can resume and media and PDF viewers can seek.
   The site always sets its own safety headers: download as an attachment, `nosniff`,
   `CSP: sandbox`.
8. **Server-to-server auth is HTTP Basic, over plain HTTP.** This is acceptable because
   the storage host is only reachable over Railway's private network, never publicly.
9. **Local storage remains an option.** With `FILES_URL` unset, the site keeps using
   `$DATA_DIR/orgs` on its own disk, exactly as today. Local dev and tests need no
   WebDAV server. Both modes sit behind one storage interface in the site.
10. **Failure handling.** If the storage host is unreachable, file requests get
    `503 Storage unavailable` (not a generic 500). Every call has a timeout. The site
    still starts when storage is down, and logs a warning at startup.

### Configuration

| Variable         | Meaning                                                              |
| ---------------- | -------------------------------------------------------------------- |
| `FILES_URL`      | WebDAV root, e.g. `http://files.railway.internal:8080/`. Unset = local disk |
| `FILES_USER`     | Basic auth user                                                      |
| `FILES_PASSWORD` | Basic auth password                                                  |

## Consequences

- **A second Railway service:** rclone with a volume. Back up two volumes: the database
  (site) and the files (rclone).
- **Folder browsing costs one `PROPFIND` per folder.** The upload dialog's full folder
  list walks the tree one level at a time (it already does this against the disk, with a
  depth cap of 8). Acceptable at our size.
- **Creating an org takes two `MKCOL`s** (`orgs/<slug>`, then `orgs/<slug>/documents`),
  because `MKCOL` only creates one level.
- **Names must be URL-encoded per path segment** (spaces, `#`, `%`, non-ASCII). Existing
  name and folder checks (no traversal, no dot-files, depth cap) stay in force.
- **The database and the storage host can disagree** (a file without a row, a row
  without a file). Accepted for now. Downloads of a row whose file is missing already
  return 404.
- **Existing files are not migrated.** They are disposable test data.
- **rclone caches directory listings** (`--dir-cache-time`). This is harmless while only
  the site writes. If people also edit the store directly, shorten it.

## Alternatives considered

- **Our own file server (axum):** more code to run and maintain, for nothing WebDAV
  doesn't already give us.
- **S3-compatible object storage:** not browsable as a filesystem, and has no real
  (empty) folders.
- **Signed links (browser talks to storage directly):** offloads bandwidth, but exposes
  the storage host publicly, and WebDAV has no standard for signed links. Not needed.
- **Folders and files tracked by id in the database:** robust to renames, but the store
  stops being browsable. Rejected for now.

## Notes

- **Interrupted uploads in `rclone serve webdav`:** rclone's copy commands write to a
  temporary `.partial` name and rename at the end ([rclone flags], `--inplace`,
  `--partial-suffix`). `serve webdav` writes through rclone's virtual-filesystem layer
  instead, and the docs don't say what an interrupted `PUT` leaves behind. Reports on
  the rclone forum show truncated files surviving dropped uploads in related setups.
  So we assume a cut-off upload **can** leave a partial file, and decision 6 makes that
  harmless. This is still to be confirmed by a test against real rclone (plan, step 2).
- **Railway private networking has used IPv6 internal addresses.** If so, rclone must
  listen on `[::]` (`--addr [::]:8080`). To be confirmed on Railway.

[`rclone serve webdav`]: https://rclone.org/commands/rclone_serve_webdav/
[rclone flags]: https://rclone.org/docs/
