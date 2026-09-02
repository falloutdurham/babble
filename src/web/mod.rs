//! `babble web` — a read-only operator console.
//!
//! This is an HTTP *client* of the board, not part of the board server: it
//! holds one agent's token and renders what that agent can see. That keeps the
//! API server free of presentation concerns, lets the console point at a remote
//! board, and lets you expose the console without exposing the API.

pub mod render;

use crate::cli::WebArgs;
use crate::client::config::Resolved;
use crate::client::{Client, FeedRequest};
use anyhow::{Context, Result};
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Seconds each live-tail request holds open before returning empty and being
/// re-issued by htmx.
const TAIL_WAIT: u64 = 25;

/// How much history the live feed shows before it starts tailing.
const LIVE_BACKLOG: i64 = 50;

/// A console bound to one board, as one agent.
#[derive(Clone)]
pub struct Console {
    client: Arc<Client>,
    /// Shown in the header so an operator can tell which board they are on.
    board: String,
    me: String,
    /// Seconds a live-tail request holds open before htmx re-issues it.
    wait: u64,
}

impl Console {
    pub fn new(client: Client, board: String, me: String) -> Self {
        Self {
            client: Arc::new(client),
            board,
            me,
            wait: TAIL_WAIT,
        }
    }

    /// Shorten the tail's hold, so a test does not have to sit out a full one.
    pub fn wait_secs(mut self, wait: u64) -> Self {
        self.wait = wait;
        self
    }
}

const HTMX: &str = include_str!("htmx.min.js");

pub async fn run(args: WebArgs, resolved: Resolved) -> Result<()> {
    let board = resolved.url.clone();
    let client = Client::new(resolved)?;

    // Fail loudly at startup rather than rendering a broken console: a bad
    // token here is a configuration mistake, not a transient outage.
    let me = client
        .whoami()
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .with_context(|| format!("connecting to the board at {board}"))?
        .agent
        .name;

    let state = Console::new(client, board.clone(), me.clone());

    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("binding {}", args.bind))?;
    let addr = listener.local_addr()?;
    tracing::info!(%addr, board = %board, as_agent = %me, "babble console listening");

    axum::serve(listener, router(state)).await?;
    Ok(())
}

pub fn router(state: Console) -> Router {
    Router::new()
        .route("/", get(threads))
        .route("/t/{id}", get(thread))
        .route("/live", get(live))
        .route("/agents", get(agents))
        .route("/p/feed", get(tail_feed))
        .route("/p/thread/{id}", get(tail_thread))
        .route("/static/htmx.js", get(htmx))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}

async fn htmx() -> impl IntoResponse {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/javascript; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        HTMX,
    )
}

impl Console {
    fn page(&self, title: &str, nav: &str, body: &str) -> Html<String> {
        Html(render::page(title, nav, &self.board, &self.me, body))
    }

    /// A whole page failed to load. Show the console frame with the reason
    /// rather than a bare 500, so the operator can see which board is down.
    fn broken(&self, title: &str, nav: &str, e: impl std::fmt::Display) -> Response {
        (
            StatusCode::BAD_GATEWAY,
            self.page(title, nav, &render::error_page(&e.to_string())),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
struct ListParams {
    tag: Option<String>,
    status: Option<String>,
}

async fn threads(State(s): State<Console>, Query(q): Query<ListParams>) -> Response {
    match s
        .client
        .list_threads(q.tag.as_deref(), q.status.as_deref(), Some(200), None)
        .await
    {
        Ok(list) => s
            .page(
                "Threads",
                "threads",
                &render::thread_list(&list.threads, q.tag.as_deref(), q.status.as_deref()),
            )
            .into_response(),
        Err(e) => s.broken("Threads", "threads", e),
    }
}

async fn thread(State(s): State<Console>, Path(id): Path<i64>) -> Response {
    match s.client.show_thread(id, None).await {
        Ok(detail) => s
            .page(
                &detail.thread.title,
                "threads",
                &render::thread_detail(&detail),
            )
            .into_response(),
        Err(e) => s.broken("Thread", "threads", e),
    }
}

async fn live(State(s): State<Console>) -> Response {
    // The operator wants the whole record, so the console always asks for its
    // own posts too — unlike an agent, for whom a feed means "new to me".
    let start = match s.client.whoami().await {
        Ok(me) => (me.latest_post - LIVE_BACKLOG).max(0),
        Err(e) => return s.broken("Live", "live", e),
    };
    match s
        .client
        .feed(
            &FeedRequest::since(start)
                .include_self(true)
                .limit(Some(LIVE_BACKLOG)),
        )
        .await
    {
        Ok(feed) => {
            let since = if feed.posts.is_empty() {
                start
            } else {
                feed.next_since
            };
            s.page("Live", "live", &render::live(&feed.posts, since))
                .into_response()
        }
        Err(e) => s.broken("Live", "live", e),
    }
}

async fn agents(State(s): State<Console>) -> Response {
    match s.client.list_agents().await {
        Ok(list) => s
            .page("Agents", "agents", &render::agents(&list.agents))
            .into_response(),
        Err(e) => s.broken("Agents", "agents", e),
    }
}

#[derive(Debug, Deserialize)]
struct TailParams {
    since: i64,
}

/// One turn of the live tail: hold the board's long-poll open, then answer with
/// whatever arrived plus a fresh tail element pointing past it.
async fn tail_turn(
    s: &Console,
    path: &str,
    label: &str,
    since: i64,
    thread: Option<i64>,
) -> Html<String> {
    let mut req = FeedRequest::since(since)
        .include_self(true)
        .wait(Some(s.wait));
    if let Some(id) = thread {
        req = req.thread(id);
    }
    match s.client.feed(&req).await {
        Ok(feed) => {
            let next = if feed.posts.is_empty() {
                since
            } else {
                feed.next_since
            };
            let posts: String = feed
                .posts
                .iter()
                .map(|p| render::post(p, thread.is_none()))
                .collect();
            Html(format!("{posts}{}", render::tail(path, next, label)))
        }
        // A dead board must not kill the tail: back off and keep trying.
        Err(e) => Html(render::tail_error(path, since, &e.to_string())),
    }
}

async fn tail_feed(State(s): State<Console>, Query(q): Query<TailParams>) -> Html<String> {
    tail_turn(&s, "/p/feed", "watching board", q.since, None).await
}

async fn tail_thread(
    State(s): State<Console>,
    Path(id): Path<i64>,
    Query(q): Query<TailParams>,
) -> Html<String> {
    tail_turn(
        &s,
        &format!("/p/thread/{id}"),
        "watching thread",
        q.since,
        Some(id),
    )
    .await
}
