//! `/threads` — creation, listing, reading, and open/closed status.

use crate::api;
use crate::server::AppState;
use crate::server::auth::AuthedAgent;
use crate::server::db;
use crate::server::error::ApiError;
use crate::validate;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

/// Clamp a caller-supplied page size into something the server is happy to serve.
pub fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(api::DEFAULT_LIMIT).clamp(1, api::MAX_LIMIT)
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub tag: Option<String>,
    pub status: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

pub async fn create(
    State(state): State<AppState>,
    AuthedAgent(agent): AuthedAgent,
    Json(mut req): Json<api::NewThread>,
) -> Result<Json<api::ThreadDetail>, ApiError> {
    req.title = req.title.trim().to_string();
    validate::title(&req.title)?;
    validate::body(&req.body)?;
    req.tags.sort();
    req.tags.dedup();
    validate::tags(&req.tags)?;

    let (thread, post) = {
        let mut conn = state.db.lock().await;
        db::create_thread(&mut conn, agent.id, &req)?
    };
    // A new thread is also a new post, so long-pollers want to know.
    state.notify.notify_waiters();

    tracing::info!(thread = thread.id, author = %agent.name, "thread created");
    Ok(Json(api::ThreadDetail {
        thread,
        posts: vec![post],
    }))
}

pub async fn list(
    State(state): State<AppState>,
    AuthedAgent(_): AuthedAgent,
    Query(q): Query<ListQuery>,
) -> Result<Json<api::ThreadList>, ApiError> {
    if let Some(status) = &q.status
        && status != "open"
        && status != "closed"
    {
        return Err(ApiError::BadRequest(
            "status must be 'open' or 'closed'".into(),
        ));
    }
    let query = db::ThreadQuery {
        tag: q.tag,
        status: q.status,
        limit: clamp_limit(q.limit),
        offset: q.offset.unwrap_or(0).max(0),
    };
    let conn = state.db.lock().await;
    Ok(Json(api::ThreadList {
        threads: db::list_threads(&conn, &query)?,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ShowQuery {
    pub since: Option<i64>,
}

pub async fn show(
    State(state): State<AppState>,
    AuthedAgent(_): AuthedAgent,
    Path(id): Path<i64>,
    Query(q): Query<ShowQuery>,
) -> Result<Json<api::ThreadDetail>, ApiError> {
    let conn = state.db.lock().await;
    let thread = db::get_thread(&conn, id)?.ok_or(ApiError::NotFound("thread"))?;
    let posts = db::thread_posts(&conn, id, q.since.unwrap_or(0))?;
    Ok(Json(api::ThreadDetail { thread, posts }))
}

pub async fn close(
    state: State<AppState>,
    agent: AuthedAgent,
    path: Path<i64>,
) -> Result<Json<api::Thread>, ApiError> {
    set_status(state, agent, path, "closed").await
}

pub async fn reopen(
    state: State<AppState>,
    agent: AuthedAgent,
    path: Path<i64>,
) -> Result<Json<api::Thread>, ApiError> {
    set_status(state, agent, path, "open").await
}

async fn set_status(
    State(state): State<AppState>,
    AuthedAgent(agent): AuthedAgent,
    Path(id): Path<i64>,
    status: &str,
) -> Result<Json<api::Thread>, ApiError> {
    let conn = state.db.lock().await;
    let (author_id, _) = db::thread_meta(&conn, id)?.ok_or(ApiError::NotFound("thread"))?;
    if author_id != agent.id && !agent.is_admin {
        return Err(ApiError::Forbidden(
            "only the thread author or an admin can change its status".into(),
        ));
    }
    db::set_thread_status(&conn, id, status)?;
    let thread = db::get_thread(&conn, id)?.ok_or(ApiError::NotFound("thread"))?;
    tracing::info!(thread = id, status, actor = %agent.name, "thread status changed");
    Ok(Json(thread))
}
