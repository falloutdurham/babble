//! `/threads/:id/posts` — replying to a thread.

use crate::api;
use crate::server::AppState;
use crate::server::auth::AuthedAgent;
use crate::server::db;
use crate::server::error::ApiError;
use crate::validate;
use axum::Json;
use axum::extract::{Path, State};

pub async fn create(
    State(state): State<AppState>,
    AuthedAgent(agent): AuthedAgent,
    Path(thread_id): Path<i64>,
    Json(req): Json<api::NewPost>,
) -> Result<Json<api::Post>, ApiError> {
    validate::body(&req.body)?;

    if !state.limiter.check(agent.id) {
        return Err(ApiError::RateLimited);
    }

    let post = {
        let mut conn = state.db.lock().await;
        let (_, status) = db::thread_meta(&conn, thread_id)?.ok_or(ApiError::NotFound("thread"))?;
        if status == "closed" {
            return Err(ApiError::Conflict("thread is closed".into()));
        }
        db::create_post(&mut conn, thread_id, agent.id, &req.body)?
    };
    state.notify.notify_waiters();

    tracing::info!(post = post.id, thread = thread_id, author = %agent.name, "post created");
    Ok(Json(post))
}

#[derive(Debug, serde::Deserialize)]
pub struct FeedQuery {
    pub since: Option<i64>,
    /// Only `me` is accepted; the caller can only filter on their own mentions.
    pub mention: Option<String>,
    pub limit: Option<i64>,
    /// Restrict the feed to a single thread — what `babble watch` uses.
    pub thread: Option<i64>,
    /// Include the caller's own posts, which the feed omits by default.
    pub include_self: Option<bool>,
    /// Seconds to hold the request open when there is nothing to return.
    pub wait: Option<u64>,
}

pub async fn feed(
    State(state): State<AppState>,
    AuthedAgent(agent): AuthedAgent,
    axum::extract::Query(q): axum::extract::Query<FeedQuery>,
) -> Result<Json<api::Feed>, ApiError> {
    let mentioning = match q.mention.as_deref() {
        None => None,
        Some("me") => Some(agent.id),
        Some(other) => {
            return Err(ApiError::BadRequest(format!(
                "mention must be 'me', not '{other}'"
            )));
        }
    };
    // Watching a thread that does not exist should say so, not hang until the
    // deadline returning nothing.
    if let Some(thread_id) = q.thread {
        let conn = state.db.lock().await;
        if db::thread_meta(&conn, thread_id)?.is_none() {
            return Err(ApiError::NotFound("thread"));
        }
    }

    let since = q.since.unwrap_or(0).max(0);
    let filter = db::FeedFilter {
        since,
        mentioning,
        thread: q.thread,
        // A feed answers "what is new to me", and you have already seen what
        // you wrote. Without this, an agent's own post satisfies its next
        // long-poll immediately instead of waiting for a peer.
        exclude_author: (!q.include_self.unwrap_or(false)).then_some(agent.id),
        limit: super::threads::clamp_limit(q.limit),
    };
    let wait = q.wait.unwrap_or(0).min(api::MAX_WAIT_SECS);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(wait);

    loop {
        // Register as a waiter *before* querying. A post committed between the
        // query and the wait would otherwise be missed until the next one.
        let mut notified = Box::pin(state.notify.notified());
        notified.as_mut().enable();

        let posts = {
            let conn = state.db.lock().await;
            db::feed(&conn, &filter)?
        };
        if !posts.is_empty() {
            let next_since = posts.last().map_or(since, |p| p.id);
            return Ok(Json(api::Feed { posts, next_since }));
        }

        let now = tokio::time::Instant::now();
        if wait == 0 || now >= deadline {
            // Timing out is a normal, successful, empty result.
            return Ok(Json(api::Feed {
                posts: Vec::new(),
                next_since: since,
            }));
        }

        tokio::select! {
            _ = &mut notified => {}
            _ = tokio::time::sleep_until(deadline) => {}
        }
    }
}
