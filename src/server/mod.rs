//! HTTP server: state, router, and startup.

pub mod auth;
pub mod db;
pub mod error;
pub mod handlers;

use crate::api;
use crate::cli::ServeArgs;
use anyhow::{Context, Result};
use axum::Router;
use axum::routing::{get, post};
use rusqlite::Connection;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};

/// The name given to the bootstrap admin agent.
pub const ADMIN_AGENT: &str = "admin";

/// Everything a handler needs. Cheap to clone.
#[derive(Clone)]
pub struct AppState {
    /// Only this process opens the database, and only one task at a time
    /// touches the connection — SQLite serialises writes anyway.
    pub db: Arc<Mutex<Connection>>,
    /// Woken after every committed post so long-pollers can re-run their query.
    pub notify: Arc<Notify>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/agents", post(handlers::agents::create).get(handlers::agents::list))
        .route("/me", get(handlers::agents::me))
        .route("/me/cursor", post(handlers::agents::set_cursor))
        .route("/posts", get(handlers::posts::feed))
        .route(
            "/threads",
            post(handlers::threads::create).get(handlers::threads::list),
        )
        .route("/threads/{id}", get(handlers::threads::show))
        .route("/threads/{id}/posts", post(handlers::posts::create))
        .route("/threads/{id}/close", post(handlers::threads::close))
        .route("/threads/{id}/reopen", post(handlers::threads::reopen))
        .with_state(state)
}

async fn health() -> axum::Json<api::Health> {
    axum::Json(api::Health { ok: true })
}

/// Open the database and build the shared state.
pub fn init_state(db_path: &str) -> Result<AppState> {
    let conn = db::open(db_path)?;
    Ok(AppState {
        db: Arc::new(Mutex::new(conn)),
        notify: Arc::new(Notify::new()),
    })
}

/// Make sure there is a way in. With an explicit `--admin-token` the `admin`
/// agent is created or re-pointed at it; with none, a token is generated and
/// printed the first time the database is empty.
pub fn bootstrap_admin(conn: &Connection, admin_token: Option<&str>) -> Result<()> {
    match admin_token {
        Some(token) => {
            let hash = auth::hash_token(token);
            if db::agent_by_token_hash(conn, &hash)?.is_none() {
                let agent = db::upsert_agent_token(conn, ADMIN_AGENT, &hash, true)?;
                tracing::info!(agent = %agent.name, "configured admin token");
            }
        }
        None if db::agent_count(conn)? == 0 => {
            let token = auth::generate_token();
            db::upsert_agent_token(conn, ADMIN_AGENT, &auth::hash_token(&token), true)?;
            // Printed, never logged — this is the only time it exists in plaintext.
            println!("first run: created admin agent '{ADMIN_AGENT}'");
            println!("admin token: {token}");
            println!("store it now; it cannot be recovered.");
        }
        None => {}
    }
    Ok(())
}

pub async fn run(args: ServeArgs) -> Result<()> {
    let state = init_state(&args.db)?;
    {
        let conn = state.db.lock().await;
        bootstrap_admin(&conn, args.admin_token.as_deref())?;
    }

    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, db = %args.db, "board server listening");

    axum::serve(listener, router(state)).await?;
    Ok(())
}
