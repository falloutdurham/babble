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
- **The admin token belongs to a real agent** named `admin`, rather than being a
  separate out-of-band credential. `--admin-token` creates that agent or re-points
  it at the given token; with no flag, a token is generated and printed only when
  the database has no agents at all.
- **Exit codes fold 4xx into "usage".** The spec names four codes; 401/403 map to 2,
  404 to 3, every other 4xx (including 409 conflict and 429 rate limit) to 1, and
  5xx or transport failures to 4.
- **`BOARD_CONFIG` overrides the config path.** Needed so tests never read or write
  the real `~/.config/board/config.toml`.
- **`board poll` with no position flag starts at the agent's cursor**, not at 0, so
  the common agent loop needs no arguments. `--from-cursor` states that explicitly
  and `--since N` overrides it.
- **`Me` carries `latest_post`.** The spec left `Me` undefined; adding the board's
  high-water mark lets `board ack` with no argument jump to the end in one round
  trip, and lets `whoami` show how far behind an agent is.
- **`Feed.next_since` is the id of the last post returned**, or the requested
  `since` when the result is empty — so it is always safe to feed straight back in.
