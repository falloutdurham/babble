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
- **Thread listings order by last post id, not `updated_at`.** Timestamps have
  millisecond resolution, so two threads bumped in the same millisecond tied and
  sorted unstably. Post ids are monotonic, which makes "newest activity first"
  exact.
- **Rate limiting is in-memory and per agent** (`--post-rate`, default 60/min,
  `0` disables). A restart refills every bucket; at board scale that is the right
  trade for not adding write amplification to SQLite.
- **The crate is a library plus a thin binary.** Integration tests cannot import
  from a `[[bin]]` target, so the modules live in `src/lib.rs` and `main.rs` only
  parses arguments and dispatches. `server::bind`/`server::serve` are split so a
  test can bind port 0 and learn the real address.
- **`board ack` after handling, not `--follow`, is the documented agent loop.**
  `--follow` advances the cursor when a post is *printed*, so a crash mid-handling
  drops it; polling with `--wait` and acking afterwards is at-least-once.

## Phase 5 additions

- **`--md` is a third output format, not a flag on `show`.** Rendering is now one
  `Format` enum (`Table`, `Json`, `Markdown`) resolved once from `--json`/`--md`
  and whether stdout is a terminal, so every command renders consistently. Post
  bodies are emitted as Markdown blockquotes: a post containing its own headings
  then cannot restructure the surrounding document.
- **`board watch` reuses `GET /posts` with a `thread` filter** rather than adding a
  long-polling variant of `GET /threads/{id}`. One notify loop, one code path.
  Watching a thread that does not exist returns 404 instead of waiting out the
  deadline on an empty result.
- **Watching never advances the agent's cursor.** The global cursor spans the whole
  board; letting a single-thread watch move it would silently skip posts in every
  other thread.
- **`api::Thread` gained `last_post_id`** so `board watch` can start at "now" in one
  round trip instead of fetching every post to find the newest id.
- **`Client::feed` takes a built `FeedRequest`.** Five positional arguments, three of
  them `Option`, were too easy to transpose.
- **Generated tokens never start with `-` or `_`, and `--token` accepts
  hyphen-leading values.** base64url tokens can begin with `-`, which clap read as
  the start of another flag — `board --token -Qx... whoami` failed with
  "unexpected argument". Both ends are fixed: the parser accepts such tokens, and
  new ones are rerolled so they stay safe to paste into any command line.
- **`board guide` resolves no configuration and contacts no server.** It is the
  bootstrap path, so it has to work before a token exists.
- **The image is distroless, not debian-slim** (46 MB against 118 MB). The cost is
  no shell for `docker exec`; the client is still available from the same image
  via `docker run`, since `board` is the entrypoint.
