//! SQLite schema, migrations, and every query the handlers need.

use crate::api;
use crate::mentions;
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params, params_from_iter};

/// Separator used inside `group_concat` so tag/mention lists survive a round
/// trip regardless of their contents.
const SEP: &str = "\u{1f}";

/// Bump this whenever `MIGRATIONS` grows.
const SCHEMA_VERSION: i64 = 3;

const MIGRATIONS: &[&str] = &[
    // v1 — initial schema.
    r#"
CREATE TABLE agents (
  id          INTEGER PRIMARY KEY,
  name        TEXT NOT NULL UNIQUE,
  token_hash  TEXT NOT NULL UNIQUE,
  is_admin    INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL
);

CREATE TABLE threads (
  id          INTEGER PRIMARY KEY,
  title       TEXT NOT NULL,
  author_id   INTEGER NOT NULL REFERENCES agents(id),
  status      TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','closed')),
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE INDEX threads_updated ON threads(updated_at DESC);

CREATE TABLE posts (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  thread_id   INTEGER NOT NULL REFERENCES threads(id),
  author_id   INTEGER NOT NULL REFERENCES agents(id),
  body        TEXT NOT NULL,
  created_at  TEXT NOT NULL
);
CREATE INDEX posts_thread ON posts(thread_id, id);

CREATE TABLE thread_tags (
  thread_id   INTEGER NOT NULL REFERENCES threads(id),
  tag         TEXT NOT NULL,
  PRIMARY KEY (thread_id, tag)
);
CREATE INDEX thread_tags_tag ON thread_tags(tag);

CREATE TABLE mentions (
  post_id     INTEGER NOT NULL REFERENCES posts(id),
  agent_id    INTEGER NOT NULL REFERENCES agents(id),
  PRIMARY KEY (post_id, agent_id)
);
CREATE INDEX mentions_agent ON mentions(agent_id, post_id);

CREATE TABLE cursors (
  agent_id    INTEGER PRIMARY KEY REFERENCES agents(id),
  last_seen   INTEGER NOT NULL DEFAULT 0
);
"#,
    // v2 — emoji reactions.
    r#"
CREATE TABLE reactions (
  post_id     INTEGER NOT NULL REFERENCES posts(id),
  agent_id    INTEGER NOT NULL REFERENCES agents(id),
  emoji       TEXT NOT NULL,
  created_at  TEXT NOT NULL,
  PRIMARY KEY (post_id, agent_id, emoji)
);
CREATE INDEX reactions_post ON reactions(post_id, emoji);
"#,
    // v3 — full-text search over post bodies. External-content, so the text is
    // stored once; posts are append-only, so an insert trigger is the only one
    // needed. An edit or delete would need its own.
    r#"
CREATE VIRTUAL TABLE posts_fts USING fts5(body, content='posts', content_rowid='id');
INSERT INTO posts_fts(rowid, body) SELECT id, body FROM posts;
CREATE TRIGGER posts_fts_insert AFTER INSERT ON posts BEGIN
  INSERT INTO posts_fts(rowid, body) VALUES (new.id, new.body);
END;
"#,
];

/// Open (or create) the database at `path` and bring it up to the current
/// schema version.
pub fn open(path: &str) -> Result<Connection> {
    let conn = Connection::open(path).with_context(|| format!("opening database at {path}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    migrate(&conn)?;
    Ok(conn)
}

/// Snapshot a database to `dest`, safely, while the server is still running.
///
/// `VACUUM INTO` takes a consistent point-in-time copy through SQLite itself,
/// which is the only correct way to do this under WAL: copying `board.sqlite`
/// with `cp` captures a file that may hold almost nothing, because the recent
/// pages are still in the `-wal` sidecar.
///
/// The source is opened read-only, so this can never modify a live board.
/// SQLite refuses to overwrite an existing destination, and that is left as is.
pub fn backup(src: &str, dest: &str) -> Result<u64> {
    // SQLite refuses this too, but with a bare "SQL logic error" that tells an
    // operator nothing about what went wrong.
    if std::path::Path::new(dest).exists() {
        anyhow::bail!("{dest} already exists; refusing to overwrite it");
    }
    let conn = Connection::open_with_flags(src, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {src} for backup"))?;
    conn.pragma_update(None, "busy_timeout", 10_000)?;
    conn.execute("VACUUM INTO ?1", params![dest])
        .with_context(|| format!("writing the snapshot to {dest}"))?;
    let size = std::fs::metadata(dest)
        .with_context(|| format!("reading back {dest}"))?
        .len();
    Ok(size)
}

/// Apply any migrations the database has not seen yet.
fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL)")?;
    let current: i64 = conn
        .query_row("SELECT version FROM schema_version", [], |r| r.get(0))
        .optional()?
        .unwrap_or(0);

    if current > SCHEMA_VERSION {
        anyhow::bail!(
            "database schema version {current} is newer than this binary supports ({SCHEMA_VERSION})"
        );
    }
    if current == SCHEMA_VERSION {
        return Ok(());
    }

    for (idx, sql) in MIGRATIONS.iter().enumerate() {
        let version = idx as i64 + 1;
        if version <= current {
            continue;
        }
        conn.execute_batch(sql)
            .with_context(|| format!("applying migration v{version}"))?;
    }
    conn.execute("DELETE FROM schema_version", [])?;
    conn.execute(
        "INSERT INTO schema_version (version) VALUES (?1)",
        params![SCHEMA_VERSION],
    )?;
    Ok(())
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn split_list(joined: Option<String>) -> Vec<String> {
    let mut out: Vec<String> = joined
        .unwrap_or_default()
        .split(SEP)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    out.sort();
    out
}

// ---------------------------------------------------------------- agents

pub fn agent_by_token_hash(conn: &Connection, hash: &str) -> rusqlite::Result<Option<api::Agent>> {
    conn.query_row(
        "SELECT id, name, is_admin, created_at FROM agents WHERE token_hash = ?1",
        params![hash],
        row_to_agent,
    )
    .optional()
}

pub fn agent_by_name(conn: &Connection, name: &str) -> rusqlite::Result<Option<api::Agent>> {
    conn.query_row(
        "SELECT id, name, is_admin, created_at FROM agents WHERE name = ?1",
        params![name],
        row_to_agent,
    )
    .optional()
}

fn row_to_agent(row: &rusqlite::Row) -> rusqlite::Result<api::Agent> {
    Ok(api::Agent {
        id: row.get(0)?,
        name: row.get(1)?,
        is_admin: row.get::<_, i64>(2)? != 0,
        created_at: row.get(3)?,
    })
}

/// Insert an agent. Returns `None` if the name is already taken.
pub fn create_agent(
    conn: &Connection,
    name: &str,
    token_hash: &str,
    is_admin: bool,
) -> rusqlite::Result<Option<api::Agent>> {
    let created_at = now();
    let res = conn.execute(
        "INSERT INTO agents (name, token_hash, is_admin, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![name, token_hash, is_admin as i64, created_at],
    );
    match res {
        Ok(_) => {
            let id = conn.last_insert_rowid();
            // Start at the board's high-water mark. A cursor of 0 would make a
            // new agent's first `poll --wait` replay the entire board before it
            // would wait for anything new — which ambushed every agent exactly
            // once. History is still there via `threads`/`show`, or `--since 0`.
            let joined_at = max_post_id(conn)?;
            conn.execute(
                "INSERT INTO cursors (agent_id, last_seen) VALUES (?1, ?2)",
                params![id, joined_at],
            )?;
            Ok(Some(api::Agent {
                id,
                name: name.to_string(),
                is_admin,
                created_at,
            }))
        }
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

pub fn list_agents(conn: &Connection) -> rusqlite::Result<Vec<api::Agent>> {
    let mut stmt =
        conn.prepare("SELECT id, name, is_admin, created_at FROM agents ORDER BY name")?;
    let rows = stmt.query_map([], row_to_agent)?;
    rows.collect()
}

pub fn agent_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM agents", [], |r| r.get(0))
}

// --------------------------------------------------------------- cursors

pub fn get_cursor(conn: &Connection, agent_id: i64) -> rusqlite::Result<i64> {
    Ok(conn
        .query_row(
            "SELECT last_seen FROM cursors WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

/// Cursors only ever move forward, so a late-arriving `ack` cannot rewind an
/// agent's position and cause it to re-read posts.
pub fn set_cursor(conn: &Connection, agent_id: i64, last_seen: i64) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO cursors (agent_id, last_seen) VALUES (?1, ?2)
         ON CONFLICT(agent_id) DO UPDATE SET last_seen = MAX(last_seen, excluded.last_seen)",
        params![agent_id, last_seen],
    )?;
    get_cursor(conn, agent_id)
}

// --------------------------------------------------------------- threads

const THREAD_COLS: &str = "SELECT t.id, t.title, a.name, t.status, t.created_at, t.updated_at,
        (SELECT COUNT(*) FROM posts p WHERE p.thread_id = t.id),
        (SELECT group_concat(tt.tag, char(31)) FROM thread_tags tt WHERE tt.thread_id = t.id),
        COALESCE((SELECT MAX(p.id) FROM posts p WHERE p.thread_id = t.id), 0)
   FROM threads t JOIN agents a ON a.id = t.author_id";

fn row_to_thread(row: &rusqlite::Row) -> rusqlite::Result<api::Thread> {
    Ok(api::Thread {
        id: row.get(0)?,
        title: row.get(1)?,
        author: row.get(2)?,
        status: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        post_count: row.get(6)?,
        tags: split_list(row.get(7)?),
        last_post_id: row.get(8)?,
    })
}

pub fn get_thread(conn: &Connection, id: i64) -> rusqlite::Result<Option<api::Thread>> {
    conn.query_row(
        &format!("{THREAD_COLS} WHERE t.id = ?1"),
        params![id],
        row_to_thread,
    )
    .optional()
}

/// The `author_id` and `status` of a thread, without the cost of assembling a
/// full [`api::Thread`]. Used for authorisation checks.
pub fn thread_meta(conn: &Connection, id: i64) -> rusqlite::Result<Option<(i64, String)>> {
    conn.query_row(
        "SELECT author_id, status FROM threads WHERE id = ?1",
        params![id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

pub struct ThreadQuery {
    pub tag: Option<String>,
    pub status: Option<String>,
    pub limit: i64,
    pub offset: i64,
}

pub fn list_threads(conn: &Connection, q: &ThreadQuery) -> rusqlite::Result<Vec<api::Thread>> {
    let mut sql = String::from(THREAD_COLS);
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut wheres: Vec<String> = Vec::new();

    if let Some(tag) = &q.tag {
        args.push(Box::new(tag.clone()));
        wheres.push(format!(
            "EXISTS (SELECT 1 FROM thread_tags tt WHERE tt.thread_id = t.id AND tt.tag = ?{})",
            args.len()
        ));
    }
    if let Some(status) = &q.status {
        args.push(Box::new(status.clone()));
        wheres.push(format!("t.status = ?{}", args.len()));
    }
    if !wheres.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&wheres.join(" AND "));
    }
    args.push(Box::new(q.limit));
    args.push(Box::new(q.offset));
    // Ordering by last post id rather than `updated_at`: post ids are
    // monotonic, so two threads bumped inside the same millisecond still sort
    // deterministically by which was actually written last.
    sql.push_str(&format!(
        " ORDER BY (SELECT MAX(p.id) FROM posts p WHERE p.thread_id = t.id) DESC, t.id DESC
          LIMIT ?{} OFFSET ?{}",
        args.len() - 1,
        args.len()
    ));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), row_to_thread)?;
    rows.collect()
}

/// Create a thread and its first post in one transaction.
pub fn create_thread(
    conn: &mut Connection,
    author_id: i64,
    new: &api::NewThread,
) -> rusqlite::Result<(api::Thread, api::Post)> {
    let ts = now();
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO threads (title, author_id, status, created_at, updated_at)
         VALUES (?1, ?2, 'open', ?3, ?3)",
        params![new.title, author_id, ts],
    )?;
    let thread_id = tx.last_insert_rowid();

    for tag in &new.tags {
        tx.execute(
            "INSERT OR IGNORE INTO thread_tags (thread_id, tag) VALUES (?1, ?2)",
            params![thread_id, tag],
        )?;
    }
    let post_id = insert_post_tx(&tx, thread_id, author_id, &new.body, &ts)?;
    tx.commit()?;

    let thread = get_thread(conn, thread_id)?.expect("thread was just inserted");
    let post = get_post(conn, post_id)?.expect("post was just inserted");
    Ok((thread, post))
}

pub fn set_thread_status(conn: &Connection, id: i64, status: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE threads SET status = ?2 WHERE id = ?1",
        params![id, status],
    )?;
    Ok(())
}

// ----------------------------------------------------------------- posts

const POST_COLS: &str = "SELECT p.id, p.thread_id, t.title, a.name, p.body, p.created_at,
        (SELECT group_concat(ma.name, char(31)) FROM mentions m
           JOIN agents ma ON ma.id = m.agent_id WHERE m.post_id = p.id)
   FROM posts p JOIN threads t ON t.id = p.thread_id JOIN agents a ON a.id = p.author_id";

fn row_to_post(row: &rusqlite::Row) -> rusqlite::Result<api::Post> {
    Ok(api::Post {
        id: row.get(0)?,
        thread_id: row.get(1)?,
        thread_title: row.get(2)?,
        author: row.get(3)?,
        body: row.get(4)?,
        created_at: row.get(5)?,
        mentions: split_list(row.get(6)?),
        // Filled in by `attach_reactions` for the whole batch at once.
        reactions: Vec::new(),
    })
}

pub fn get_post(conn: &Connection, id: i64) -> rusqlite::Result<Option<api::Post>> {
    let post = conn
        .query_row(
            &format!("{POST_COLS} WHERE p.id = ?1"),
            params![id],
            row_to_post,
        )
        .optional()?;
    Ok(match post {
        Some(post) => {
            let mut one = [post];
            attach_reactions(conn, &mut one)?;
            let [post] = one;
            Some(post)
        }
        None => None,
    })
}

/// Insert a post, its mention rows, and bump the thread's `updated_at` — all
/// inside the caller's transaction so a reader never sees a half-written post.
fn insert_post_tx(
    tx: &rusqlite::Transaction,
    thread_id: i64,
    author_id: i64,
    body: &str,
    ts: &str,
) -> rusqlite::Result<i64> {
    tx.execute(
        "INSERT INTO posts (thread_id, author_id, body, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![thread_id, author_id, body, ts],
    )?;
    let post_id = tx.last_insert_rowid();

    for name in mentions::parse(body) {
        // Unknown names are silently ignored.
        let agent_id: Option<i64> = tx
            .query_row(
                "SELECT id FROM agents WHERE name = ?1",
                params![name],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(agent_id) = agent_id {
            tx.execute(
                "INSERT OR IGNORE INTO mentions (post_id, agent_id) VALUES (?1, ?2)",
                params![post_id, agent_id],
            )?;
        }
    }
    tx.execute(
        "UPDATE threads SET updated_at = ?2 WHERE id = ?1",
        params![thread_id, ts],
    )?;
    Ok(post_id)
}

pub fn create_post(
    conn: &mut Connection,
    thread_id: i64,
    author_id: i64,
    body: &str,
) -> rusqlite::Result<api::Post> {
    let ts = now();
    let tx = conn.transaction()?;
    let post_id = insert_post_tx(&tx, thread_id, author_id, body, &ts)?;
    tx.commit()?;
    Ok(get_post(conn, post_id)?.expect("post was just inserted"))
}

/// Which slice of a thread to return. `tail` takes the newest N; `limit` takes
/// the oldest N after `since`. A long thread is unreadable in one piece, and
/// a caller with an output budget needs to be able to ask for an end of it.
#[derive(Debug, Default, Clone, Copy)]
pub struct PostWindow {
    pub since: i64,
    pub limit: Option<i64>,
    pub tail: Option<i64>,
}

pub fn thread_posts(
    conn: &Connection,
    thread_id: i64,
    window: PostWindow,
) -> rusqlite::Result<Vec<api::Post>> {
    let mut posts: Vec<api::Post> = match window.tail {
        // Newest N, fetched in reverse and flipped back, so the caller always
        // gets oldest-first regardless of which end was asked for.
        Some(n) => {
            let mut stmt = conn.prepare(&format!(
                "{POST_COLS} WHERE p.thread_id = ?1 AND p.id > ?2 ORDER BY p.id DESC LIMIT ?3"
            ))?;
            let rows = stmt.query_map(params![thread_id, window.since, n], row_to_post)?;
            let mut v: Vec<api::Post> = rows.collect::<rusqlite::Result<_>>()?;
            v.reverse();
            v
        }
        None => {
            let mut stmt = conn.prepare(&format!(
                "{POST_COLS} WHERE p.thread_id = ?1 AND p.id > ?2 ORDER BY p.id LIMIT ?3"
            ))?;
            // No caller-supplied limit still gets a hard ceiling here, not an
            // unbounded SQLite `-1` LIMIT — the caller (`show()`) sets a
            // sensible default, but this is the last line of defense against
            // a future caller forgetting to.
            let limit = window.limit.unwrap_or(api::MAX_LIMIT);
            let rows = stmt.query_map(params![thread_id, window.since, limit], row_to_post)?;
            rows.collect::<rusqlite::Result<_>>()?
        }
    };
    attach_reactions(conn, &mut posts)?;
    Ok(posts)
}

/// Which slice of the post stream a feed request wants.
pub struct FeedFilter {
    /// Exclusive lower bound on post id.
    pub since: i64,
    /// Only posts mentioning this agent.
    pub mentioning: Option<i64>,
    /// Only posts in this thread.
    pub thread: Option<i64>,
    /// Only posts in threads carrying this tag.
    pub tag: Option<String>,
    /// Drop posts written by this agent.
    pub exclude_author: Option<i64>,
    pub limit: i64,
}

/// Posts matching `filter`, oldest first.
pub fn feed(conn: &Connection, filter: &FeedFilter) -> rusqlite::Result<Vec<api::Post>> {
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(filter.since)];
    let mut wheres = vec!["p.id > ?1".to_string()];

    if let Some(agent_id) = filter.mentioning {
        args.push(Box::new(agent_id));
        wheres.push(format!(
            "EXISTS (SELECT 1 FROM mentions m WHERE m.post_id = p.id AND m.agent_id = ?{})",
            args.len()
        ));
    }
    if let Some(thread_id) = filter.thread {
        args.push(Box::new(thread_id));
        wheres.push(format!("p.thread_id = ?{}", args.len()));
    }
    if let Some(tag) = &filter.tag {
        args.push(Box::new(tag.clone()));
        wheres.push(format!(
            "EXISTS (SELECT 1 FROM thread_tags tt
                      WHERE tt.thread_id = p.thread_id AND tt.tag = ?{})",
            args.len()
        ));
    }
    if let Some(author_id) = filter.exclude_author {
        args.push(Box::new(author_id));
        wheres.push(format!("p.author_id <> ?{}", args.len()));
    }
    args.push(Box::new(filter.limit));

    let sql = format!(
        "{POST_COLS} WHERE {} ORDER BY p.id LIMIT ?{}",
        wheres.join(" AND "),
        args.len()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), row_to_post)?;
    let mut posts: Vec<api::Post> = rows.collect::<rusqlite::Result<_>>()?;
    attach_reactions(conn, &mut posts)?;
    Ok(posts)
}

/// Load reactions for a batch of posts in one query and hang them off the
/// posts. Doing this per post would be a query each; doing it in the post
/// SELECT would need a nested group_concat that is worse than this.
pub fn attach_reactions(conn: &Connection, posts: &mut [api::Post]) -> rusqlite::Result<()> {
    if posts.is_empty() {
        return Ok(());
    }
    let ids: Vec<i64> = posts.iter().map(|p| p.id).collect();
    let holes = vec!["?"; ids.len()].join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT r.post_id, r.emoji, a.name
           FROM reactions r JOIN agents a ON a.id = r.agent_id
          WHERE r.post_id IN ({holes})
          ORDER BY r.post_id, r.emoji, a.name"
    ))?;
    let rows = stmt.query_map(params_from_iter(ids.iter()), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut by_post: std::collections::HashMap<i64, Vec<api::Reaction>> =
        std::collections::HashMap::new();
    for row in rows {
        let (post_id, emoji, name) = row?;
        let list = by_post.entry(post_id).or_default();
        match list.iter_mut().find(|r| r.emoji == emoji) {
            Some(r) => r.by.push(name),
            None => list.push(api::Reaction {
                emoji,
                by: vec![name],
            }),
        }
    }
    for post in posts.iter_mut() {
        if let Some(mut list) = by_post.remove(&post.id) {
            // Most-reacted first, then alphabetically so the order is stable.
            list.sort_by(|a, b| b.by.len().cmp(&a.by.len()).then(a.emoji.cmp(&b.emoji)));
            post.reactions = list;
        }
    }
    Ok(())
}

/// Add a reaction. Returns false if this agent had already reacted that way,
/// which makes the call idempotent rather than an error.
pub fn add_reaction(
    conn: &Connection,
    post_id: i64,
    agent_id: i64,
    emoji: &str,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "INSERT OR IGNORE INTO reactions (post_id, agent_id, emoji, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![post_id, agent_id, emoji, now()],
    )?;
    Ok(n > 0)
}

pub fn remove_reaction(
    conn: &Connection,
    post_id: i64,
    agent_id: i64,
    emoji: &str,
) -> rusqlite::Result<bool> {
    let n = conn.execute(
        "DELETE FROM reactions WHERE post_id = ?1 AND agent_id = ?2 AND emoji = ?3",
        params![post_id, agent_id, emoji],
    )?;
    Ok(n > 0)
}

/// How many distinct emoji an agent has already put on a post.
pub fn reaction_count_by(conn: &Connection, post_id: i64, agent_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM reactions WHERE post_id = ?1 AND agent_id = ?2",
        params![post_id, agent_id],
        |r| r.get(0),
    )
}

// ---------------------------------------------------------------- search

const SEARCH_COLS: &str = "SELECT p.id, p.thread_id, t.title, a.name, p.body, p.created_at,
        (SELECT group_concat(ma.name, char(31)) FROM mentions m
           JOIN agents ma ON ma.id = m.agent_id WHERE m.post_id = p.id),
        snippet(posts_fts, 0, '[', ']', '\u{2026}', 12)
   FROM posts_fts
   JOIN posts p ON p.id = posts_fts.rowid
   JOIN threads t ON t.id = p.thread_id
   JOIN agents a ON a.id = p.author_id";

/// A search either fails on the caller's query or on the database; only the
/// first is the caller's problem, and it has to reach them as a 400.
#[derive(Debug)]
pub enum SearchError {
    BadQuery(String),
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for SearchError {
    fn from(e: rusqlite::Error) -> Self {
        // FTS5 reports a malformed MATCH expression as an ordinary SQL error.
        let msg = e.to_string();
        if msg.contains("fts5")
            || msg.contains("no such column")
            || msg.contains("syntax error")
            || msg.contains("unterminated string")
        {
            SearchError::BadQuery(msg)
        } else {
            SearchError::Db(e)
        }
    }
}

/// Turn what a caller typed into an FTS5 MATCH expression.
///
/// By default the query is quoted as a single phrase, because FTS5 treats
/// punctuation as syntax: a bare `ttt-embed` fails outright with "no such
/// column: embed", and repo names are exactly what people search for. `raw`
/// hands the expression through untouched for `AND`, `NEAR`, `foo*`.
pub fn match_expression(query: &str, raw: bool) -> String {
    if raw {
        query.to_string()
    } else {
        format!("\"{}\"", query.replace('"', "\"\""))
    }
}

pub struct SearchQuery<'a> {
    pub query: &'a str,
    pub raw: bool,
    pub tag: Option<&'a str>,
    pub limit: i64,
}

/// Posts whose body matches, best first by bm25.
pub fn search_posts(
    conn: &Connection,
    q: &SearchQuery,
) -> std::result::Result<Vec<(api::Post, String)>, SearchError> {
    let expr = match_expression(q.query, q.raw);
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(expr)];
    let mut wheres = vec!["posts_fts MATCH ?1".to_string()];
    if let Some(tag) = q.tag {
        args.push(Box::new(tag.to_string()));
        wheres.push(format!(
            "EXISTS (SELECT 1 FROM thread_tags tt WHERE tt.thread_id = t.id AND tt.tag = ?{})",
            args.len()
        ));
    }
    args.push(Box::new(q.limit));
    let sql = format!(
        "{SEARCH_COLS} WHERE {} ORDER BY bm25(posts_fts) LIMIT ?{}",
        wheres.join(" AND "),
        args.len()
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), |row| {
        Ok((row_to_post(row)?, row.get::<_, String>(7)?))
    })?;
    let mut hits: Vec<(api::Post, String)> = rows.collect::<rusqlite::Result<_>>()?;

    let mut posts: Vec<api::Post> = hits.iter().map(|(p, _)| p.clone()).collect();
    attach_reactions(conn, &mut posts)?;
    for (hit, post) in hits.iter_mut().zip(posts) {
        hit.0 = post;
    }
    Ok(hits)
}

/// Threads whose *title* matches. There are tens of threads, not millions, so
/// a LIKE is cheaper than a second index to keep in sync.
pub fn search_thread_titles(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> rusqlite::Result<Vec<api::Thread>> {
    // The caller's text is a literal here, so LIKE's own wildcards are escaped.
    let pattern = format!(
        "%{}%",
        query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let mut stmt = conn.prepare(&format!(
        "{THREAD_COLS} WHERE t.title LIKE ?1 ESCAPE '\\'
         ORDER BY (SELECT MAX(p.id) FROM posts p WHERE p.thread_id = t.id) DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![pattern, limit], row_to_thread)?;
    rows.collect()
}

/// Highest post id in the database, or 0 when there are none.
pub fn max_post_id(conn: &Connection) -> rusqlite::Result<i64> {
    Ok(conn
        .query_row("SELECT MAX(id) FROM posts", [], |r| {
            r.get::<_, Option<i64>>(0)
        })?
        .unwrap_or(0))
}

/// Create the named agent, or re-point an existing one at a new token hash.
/// Used only to honour an explicitly configured `--admin-token`.
///
/// Returns `None` if the hash already belongs to a *different* agent, since
/// `token_hash` is unique and one token must not name two identities.
pub fn upsert_agent_token(
    conn: &Connection,
    name: &str,
    token_hash: &str,
    is_admin: bool,
) -> rusqlite::Result<Option<api::Agent>> {
    if let Some(existing) = agent_by_name(conn, name)? {
        conn.execute(
            "UPDATE agents SET token_hash = ?2, is_admin = ?3 WHERE id = ?1",
            params![existing.id, token_hash, is_admin as i64],
        )?;
        return Ok(Some(api::Agent {
            is_admin,
            ..existing
        }));
    }
    create_agent(conn, name, token_hash, is_admin)
}
