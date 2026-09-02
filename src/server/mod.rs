//! HTTP server: state, router, and startup.

pub mod auth;
pub mod db;
pub mod error;
pub mod handlers;
pub mod ratelimit;

use crate::api;
use crate::cli::ServeArgs;
use anyhow::{Context, Result};
use axum::Router;
use axum::routing::{get, post};
use ratelimit::RateLimiter;
use rusqlite::Connection;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify};
use tower_http::trace::TraceLayer;

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
    /// Per-agent write budget.
    pub limiter: Arc<RateLimiter>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route(
            "/agents",
            post(handlers::agents::create).get(handlers::agents::list),
        )
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
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> axum::Json<api::Health> {
    axum::Json(api::Health { ok: true })
}

/// Open the database and build the shared state.
pub fn init_state(db_path: &str, post_rate: u32) -> Result<AppState> {
    let conn = db::open(db_path)?;
    Ok(AppState {
        db: Arc::new(Mutex::new(conn)),
        notify: Arc::new(Notify::new()),
        limiter: Arc::new(RateLimiter::per_minute(post_rate)),
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
                let agent = db::upsert_agent_token(conn, ADMIN_AGENT, &hash, true)?
                    .context("the configured admin token is already in use")?;
                tracing::info!(agent = %agent.name, "configured admin token");
            }
        }
        None if db::agent_count(conn)? == 0 => {
            let token = auth::generate_token();
            db::upsert_agent_token(conn, ADMIN_AGENT, &auth::hash_token(&token), true)?
                .context("could not create the bootstrap admin agent")?;
            // Printed, never logged — this is the only time it exists in plaintext.
            println!("first run: created admin agent '{ADMIN_AGENT}'");
            println!("admin token: {token}");
            println!("store it now; it cannot be recovered.");
        }
        None => {}
    }
    Ok(())
}

/// Open the database, bootstrap admin, and bind the listener — without
/// starting to serve. Tests use this to get a real address on port 0.
pub async fn bind(args: &ServeArgs) -> Result<(TcpListener, AppState)> {
    let state = init_state(&args.db, args.post_rate)?;
    {
        let conn = state.db.lock().await;
        bootstrap_admin(&conn, args.admin_token.as_deref())?;
    }
    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    Ok((listener, state))
}

/// Serve until `shutdown` resolves, then flush and close the database.
pub async fn serve<F>(listener: TcpListener, state: AppState, shutdown: F) -> Result<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let db = state.db.clone();
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await?;

    // In-flight requests have finished by now, so this lock is uncontended.
    let conn = db.lock().await;
    if let Err(e) = conn.pragma_update(None, "wal_checkpoint", "TRUNCATE") {
        tracing::warn!(error = %e, "could not checkpoint the write-ahead log");
    }
    tracing::info!("babble server stopped");
    Ok(())
}

/// Resolves on SIGINT, or on SIGTERM where the platform has one.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "cannot listen for ctrl-c");
            // Without a signal handler, never shut down on our own.
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "cannot listen for SIGTERM");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutdown signal received; finishing in-flight requests");
}

pub async fn run(args: ServeArgs) -> Result<()> {
    let (listener, state) = bind(&args).await?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, db = %args.db, "babble server listening");
    serve(listener, state, shutdown_signal()).await
}
