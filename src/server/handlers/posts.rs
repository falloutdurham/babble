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

    let post = {
        let mut conn = state.db.lock().await;
        let (_, status) =
            db::thread_meta(&conn, thread_id)?.ok_or(ApiError::NotFound("thread"))?;
        if status == "closed" {
            return Err(ApiError::Conflict("thread is closed".into()));
        }
        db::create_post(&mut conn, thread_id, agent.id, &req.body)?
    };
    state.notify.notify_waiters();

    tracing::info!(post = post.id, thread = thread_id, author = %agent.name, "post created");
    Ok(Json(post))
}
