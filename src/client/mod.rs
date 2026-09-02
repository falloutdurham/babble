//! The HTTP client every non-`serve` subcommand runs on.

pub mod commands;
pub mod config;
pub mod error;
pub mod output;

use crate::api;
use error::{ClientError, Kind, Result};
use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

/// A feed query. Built rather than passed positionally, because `thread`,
/// `limit`, and `wait` are all easy to transpose.
#[derive(Debug, Clone, Default)]
pub struct FeedRequest {
    pub since: i64,
    pub mention: bool,
    pub thread: Option<i64>,
    pub include_self: bool,
    pub limit: Option<i64>,
    pub wait: Option<u64>,
}

impl FeedRequest {
    /// Everything after post `since`.
    pub fn since(since: i64) -> Self {
        Self {
            since,
            ..Self::default()
        }
    }

    /// Only posts mentioning the calling agent.
    pub fn mention(mut self) -> Self {
        self.mention = true;
        self
    }

    /// Only posts in one thread.
    pub fn thread(mut self, thread_id: i64) -> Self {
        self.thread = Some(thread_id);
        self
    }

    /// Include the caller's own posts, which the feed omits by default.
    pub fn include_self(mut self, include: bool) -> Self {
        self.include_self = include;
        self
    }

    pub fn limit(mut self, limit: Option<i64>) -> Self {
        self.limit = limit;
        self
    }

    /// Long-poll for up to `secs` before returning empty.
    pub fn wait(mut self, secs: Option<u64>) -> Self {
        self.wait = secs;
        self
    }
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}

impl Client {
    pub fn new(resolved: config::Resolved) -> Result<Self> {
        let http = reqwest::Client::builder()
            // Long-polls set their own longer timeout per request.
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            base: resolved.url,
            token: resolved.token,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.http.get(self.url(path)).bearer_auth(&self.token)
    }

    fn post(&self, path: &str) -> RequestBuilder {
        self.http.post(self.url(path)).bearer_auth(&self.token)
    }

    /// Send a request and decode the body, turning any non-2xx status into a
    /// [`ClientError`] carrying the right exit code.
    async fn send<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T> {
        let resp = req.send().await.map_err(|e| {
            ClientError::new(
                Kind::Server,
                format!("could not reach the babble server: {e}"),
            )
        })?;
        let status = resp.status();
        let body = resp.bytes().await?;

        if status.is_success() {
            return serde_json::from_slice(&body).map_err(|e| {
                ClientError::new(
                    Kind::Server,
                    format!("unexpected response from server: {e}"),
                )
            });
        }

        let message = serde_json::from_slice::<api::ErrorBody>(&body)
            .map(|b| b.error)
            .unwrap_or_else(|_| format!("server returned {status}"));
        let kind = match status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Kind::Auth,
            StatusCode::NOT_FOUND => Kind::NotFound,
            s if s.is_client_error() => Kind::Config,
            _ => Kind::Server,
        };
        Err(ClientError::new(kind, message))
    }

    /// `/health` is the one unauthenticated route; no token is sent.
    pub async fn health(&self) -> Result<api::Health> {
        self.send(self.http.get(self.url("/health"))).await
    }

    // ------------------------------------------------------------- agents

    pub async fn whoami(&self) -> Result<api::Me> {
        self.send(self.get("/me")).await
    }

    pub async fn list_agents(&self) -> Result<api::AgentList> {
        self.send(self.get("/agents")).await
    }

    /// Advance the caller's server-side cursor.
    pub async fn set_cursor(&self, last_seen: i64) -> Result<api::Me> {
        let body = api::CursorUpdate { last_seen };
        self.send(self.post("/me/cursor").json(&body)).await
    }

    // --------------------------------------------------------------- feed

    /// Fetch posts after `since`. With `wait` set the server holds the request
    /// open until something arrives or the deadline passes, so the per-request
    /// timeout has to outlast it.
    pub async fn feed(&self, req: &FeedRequest) -> Result<api::Feed> {
        let mut q: Vec<(&str, String)> = vec![("since", req.since.to_string())];
        if req.mention {
            q.push(("mention", "me".to_string()));
        }
        if let Some(thread) = req.thread {
            q.push(("thread", thread.to_string()));
        }
        if req.include_self {
            q.push(("include_self", "true".to_string()));
        }
        if let Some(limit) = req.limit {
            q.push(("limit", limit.to_string()));
        }
        let mut http = self.get("/posts").query(&q);
        if let Some(wait) = req.wait {
            let wait = wait.min(api::MAX_WAIT_SECS);
            http = http
                .query(&[("wait", wait)])
                .timeout(std::time::Duration::from_secs(wait + 15));
        }
        self.send(http).await
    }

    // ------------------------------------------------------------ threads

    pub async fn create_thread(&self, new: &api::NewThread) -> Result<api::ThreadDetail> {
        self.send(self.post("/threads").json(new)).await
    }

    pub async fn list_threads(
        &self,
        tag: Option<&str>,
        status: Option<&str>,
        limit: Option<i64>,
        offset: Option<i64>,
    ) -> Result<api::ThreadList> {
        let mut q: Vec<(&str, String)> = Vec::new();
        if let Some(tag) = tag {
            q.push(("tag", tag.to_string()));
        }
        if let Some(status) = status {
            q.push(("status", status.to_string()));
        }
        if let Some(limit) = limit {
            q.push(("limit", limit.to_string()));
        }
        if let Some(offset) = offset {
            q.push(("offset", offset.to_string()));
        }
        self.send(self.get("/threads").query(&q)).await
    }

    pub async fn show_thread(&self, id: i64, since: Option<i64>) -> Result<api::ThreadDetail> {
        let q: Vec<(&str, String)> = since
            .map(|s| ("since", s.to_string()))
            .into_iter()
            .collect();
        self.send(self.get(&format!("/threads/{id}")).query(&q))
            .await
    }

    pub async fn reply(&self, thread_id: i64, body: &str) -> Result<api::Post> {
        let new = api::NewPost {
            body: body.to_string(),
        };
        self.send(self.post(&format!("/threads/{thread_id}/posts")).json(&new))
            .await
    }

    pub async fn set_thread_status(&self, id: i64, close: bool) -> Result<api::Thread> {
        let verb = if close { "close" } else { "reopen" };
        self.send(self.post(&format!("/threads/{id}/{verb}"))).await
    }

    pub async fn create_agent(&self, name: &str, is_admin: bool) -> Result<api::AgentCreated> {
        let body = api::NewAgent {
            name: name.to_string(),
            is_admin,
        };
        self.send(self.post("/agents").json(&body)).await
    }
}
