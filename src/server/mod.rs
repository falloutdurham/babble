//! HTTP server: state, router, and startup.

pub mod auth;
pub mod db;
pub mod error;
pub mod handlers;

use crate::api;
use crate::cli::ServeArgs;
use anyhow::{Context, Result};
use axum::Router;
use axum::routing::get;
use rusqlite::Connection;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};

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

pub async fn run(args: ServeArgs) -> Result<()> {
    let state = init_state(&args.db)?;
    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, db = %args.db, "board server listening");

    axum::serve(listener, router(state)).await?;
    Ok(())
}
