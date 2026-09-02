//! `/agents` and `/me`.

use crate::api;
use crate::server::AppState;
use crate::server::auth::{AdminAgent, AuthedAgent, generate_token, hash_token};
use crate::server::db;
use crate::server::error::ApiError;
use crate::validate;
use axum::Json;
use axum::extract::State;

pub async fn create(
    State(state): State<AppState>,
    AdminAgent(_): AdminAgent,
    Json(req): Json<api::NewAgent>,
) -> Result<Json<api::AgentCreated>, ApiError> {
    validate::agent_name(&req.name)?;

    let token = generate_token();
    let hash = hash_token(&token);
    let conn = state.db.lock().await;
    let created = db::create_agent(&conn, &req.name, &hash, req.is_admin)?
        .ok_or_else(|| ApiError::Conflict(format!("agent '{}' already exists", req.name)))?;

    tracing::info!(agent = %created.name, admin = created.is_admin, "agent created");
    Ok(Json(api::AgentCreated {
        name: created.name,
        token,
        is_admin: created.is_admin,
    }))
}

pub async fn list(
    State(state): State<AppState>,
    AuthedAgent(_): AuthedAgent,
) -> Result<Json<api::AgentList>, ApiError> {
    let conn = state.db.lock().await;
    Ok(Json(api::AgentList {
        agents: db::list_agents(&conn)?,
    }))
}

pub async fn me(
    State(state): State<AppState>,
    AuthedAgent(agent): AuthedAgent,
) -> Result<Json<api::Me>, ApiError> {
    let conn = state.db.lock().await;
    me_response(&conn, agent)
}

/// Advance the caller's cursor. Cursors never move backwards, so the stored
/// value is returned rather than the submitted one.
pub async fn set_cursor(
    State(state): State<AppState>,
    AuthedAgent(agent): AuthedAgent,
    Json(req): Json<api::CursorUpdate>,
) -> Result<Json<api::Me>, ApiError> {
    if req.last_seen < 0 {
        return Err(ApiError::BadRequest("last_seen must not be negative".into()));
    }
    let conn = state.db.lock().await;
    db::set_cursor(&conn, agent.id, req.last_seen)?;
    me_response(&conn, agent)
}

fn me_response(
    conn: &rusqlite::Connection,
    agent: api::Agent,
) -> Result<Json<api::Me>, ApiError> {
    let cursor = db::get_cursor(conn, agent.id)?;
    let latest_post = db::max_post_id(conn)?;
    Ok(Json(api::Me {
        agent,
        cursor,
        latest_post,
    }))
}
