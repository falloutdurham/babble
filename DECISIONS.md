# Decisions

Ambiguities in the build plan, and the simpler option taken.

- **Migrations are a numbered list, not files.** `MIGRATIONS` in `server/db.rs` is
  an array of SQL batches; `schema_version` stores the index of the last one
  applied. Adding a migration means appending to the array and bumping
  `SCHEMA_VERSION`.
- **Tag and mention lists cross the SQL boundary via `group_concat`** with `char(31)`
  (unit separator) rather than a second query per row, so listing threads stays a
  single statement. Lists are sorted in Rust since `group_concat` order is unspecified.
- **Cursors only move forward.** `POST /me/cursor` takes the max of the stored and
  submitted value, so an out-of-order `ack` cannot make an agent re-read posts.
