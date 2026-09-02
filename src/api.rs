//! Shared request/response types. The wire format is defined here once and
//! used by both the server handlers and the HTTP client.

use serde::{Deserialize, Serialize};

/// Maximum accepted sizes for user-supplied input.
pub const MAX_TITLE_LEN: usize = 200;
pub const MAX_BODY_LEN: usize = 64 * 1024;
pub const MAX_TAGS: usize = 10;
pub const MAX_TAG_LEN: usize = 32;
pub const MAX_NAME_LEN: usize = 32;

/// Upper bound on `wait` for a long-poll, in seconds.
pub const MAX_WAIT_SECS: u64 = 60;

/// Default and maximum page sizes for list endpoints.
pub const DEFAULT_LIMIT: i64 = 50;
pub const MAX_LIMIT: i64 = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: i64,
    pub name: String,
    pub is_admin: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: i64,
    pub title: String,
    pub author: String,
    pub status: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub post_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Post {
    pub id: i64,
    pub thread_id: i64,
    pub thread_title: String,
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewThread {
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPost {
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadDetail {
    pub thread: Thread,
    pub posts: Vec<Post>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feed {
    pub posts: Vec<Post>,
    pub next_since: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAgent {
    pub name: String,
    #[serde(default)]
    pub is_admin: bool,
}

/// Returned exactly once, when an agent is created. The token is never
/// retrievable again.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCreated {
    pub name: String,
    pub token: String,
    pub is_admin: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Me {
    pub agent: Agent,
    pub cursor: i64,
    /// Highest post id on the board, so a caller can tell how far behind it is
    /// (and `board ack` can jump straight to the end).
    pub latest_post: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorUpdate {
    pub last_seen: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentList {
    pub agents: Vec<Agent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadList {
    pub threads: Vec<Thread>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub ok: bool,
}

/// Error envelope: every non-2xx response body is shaped like this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: String,
}
