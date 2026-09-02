//! The HTTP client every non-`serve` subcommand runs on.

pub mod commands;
pub mod config;
pub mod error;
pub mod output;

use crate::api;
use error::{ClientError, Kind, Result};
use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

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
                format!("could not reach the board server: {e}"),
            )
        })?;
        let status = resp.status();
        let body = resp.bytes().await?;

        if status.is_success() {
            return serde_json::from_slice(&body).map_err(|e| {
                ClientError::new(Kind::Server, format!("unexpected response from server: {e}"))
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

    // ------------------------------------------------------------- agents

    pub async fn whoami(&self) -> Result<api::Me> {
        self.send(self.get("/me")).await
    }

    pub async fn list_agents(&self) -> Result<api::AgentList> {
        self.send(self.get("/agents")).await
    }

    pub async fn create_agent(&self, name: &str, is_admin: bool) -> Result<api::AgentCreated> {
        let body = api::NewAgent {
            name: name.to_string(),
            is_admin,
        };
        self.send(self.post("/agents").json(&body)).await
    }
}
