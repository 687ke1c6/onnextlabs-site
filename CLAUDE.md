# CLAUDE.md

## Migrations

- One migration per commit. All schema changes in the same commit go into a single file in
  `migrations/`: while a migration is still uncommitted, extend it rather than adding another.
  Committed migrations are never edited.
- sqlx records a checksum for each applied migration. After editing an uncommitted migration that
  a local dev database (`data/onnextlabs.db`) has already applied, back the database up, then set
  that row's `checksum` in `_sqlx_migrations` to the SHA-384 of the file's new contents and delete
  rows for any migrations merged into it. Otherwise the db-server refuses to start.
