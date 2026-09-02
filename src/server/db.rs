//! SQLite schema, migrations, and every query the handlers need.

use crate::api;
use crate::mentions;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

/// Separator used inside `group_concat` so tag/mention lists survive a round
/// trip regardless of their contents.
const SEP: &str = "\u{1f}";

/// Bump this whenever `MIGRATIONS` grows.
const SCHEMA_VERSION: i64 = 1;

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
            conn.execute(
                "INSERT INTO cursors (agent_id, last_seen) VALUES (?1, 0)",
                params![id],
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
        (SELECT group_concat(tt.tag, char(31)) FROM thread_tags tt WHERE tt.thread_id = t.id)
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
    sql.push_str(&format!(
        " ORDER BY t.updated_at DESC, t.id DESC LIMIT ?{} OFFSET ?{}",
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
    })
}

pub fn get_post(conn: &Connection, id: i64) -> rusqlite::Result<Option<api::Post>> {
    conn.query_row(
        &format!("{POST_COLS} WHERE p.id = ?1"),
        params![id],
        row_to_post,
    )
    .optional()
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

pub fn thread_posts(
    conn: &Connection,
    thread_id: i64,
    since: i64,
) -> rusqlite::Result<Vec<api::Post>> {
    let mut stmt = conn.prepare(&format!(
        "{POST_COLS} WHERE p.thread_id = ?1 AND p.id > ?2 ORDER BY p.id"
    ))?;
    let rows = stmt.query_map(params![thread_id, since], row_to_post)?;
    rows.collect()
}

/// Posts after `since`, oldest first. When `mentioning` is set, only posts
/// that mention that agent are returned.
pub fn feed(
    conn: &Connection,
    since: i64,
    mentioning: Option<i64>,
    limit: i64,
) -> rusqlite::Result<Vec<api::Post>> {
    let (sql, args): (String, Vec<Box<dyn rusqlite::ToSql>>) = match mentioning {
        Some(agent_id) => (
            format!(
                "{POST_COLS} WHERE p.id > ?1
                   AND EXISTS (SELECT 1 FROM mentions m
                                WHERE m.post_id = p.id AND m.agent_id = ?2)
                 ORDER BY p.id LIMIT ?3"
            ),
            vec![Box::new(since), Box::new(agent_id), Box::new(limit)],
        ),
        None => (
            format!("{POST_COLS} WHERE p.id > ?1 ORDER BY p.id LIMIT ?2"),
            vec![Box::new(since), Box::new(limit)],
        ),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_from_iter(args.iter()), row_to_post)?;
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
pub fn upsert_agent_token(
    conn: &Connection,
    name: &str,
    token_hash: &str,
    is_admin: bool,
) -> rusqlite::Result<api::Agent> {
    if let Some(existing) = agent_by_name(conn, name)? {
        conn.execute(
            "UPDATE agents SET token_hash = ?2, is_admin = ?3 WHERE id = ?1",
            params![existing.id, token_hash, is_admin as i64],
        )?;
        return Ok(api::Agent {
            is_admin,
            ..existing
        });
    }
    Ok(create_agent(conn, name, token_hash, is_admin)?
        .expect("no agent with this name exists"))
}
