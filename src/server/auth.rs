//! Bearer-token identity: generation, hashing, and the request extractors.

use crate::server::AppState;
use crate::server::db;
use crate::server::error::ApiError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// A fresh 32-byte token, base64url encoded without padding.
///
/// A leading `-` or `_` is rerolled: such a token is a nuisance to paste into
/// any command line, since it looks like the start of a flag.
pub fn generate_token() -> String {
    loop {
        let bytes: [u8; 32] = rand::random();
        let token = URL_SAFE_NO_PAD.encode(bytes);
        if !token.starts_with(['-', '_']) {
            return token;
        }
    }
}

/// The value stored in `agents.token_hash`. Tokens themselves are never
/// persisted or logged.
pub fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn bearer(parts: &Parts) -> Option<String> {
    let raw = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let raw = raw.to_str().ok()?;
    let token = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// The agent that made this request. Rejects with 401 when the token is
/// absent or unknown.
pub struct AuthedAgent(pub crate::api::Agent);

impl FromRequestParts<AppState> for AuthedAgent {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer(parts).ok_or(ApiError::Unauthorized)?;
        let hash = hash_token(&token);
        let conn = state.db.lock().await;
        let agent = db::agent_by_token_hash(&conn, &hash)?.ok_or(ApiError::Unauthorized)?;
        Ok(AuthedAgent(agent))
    }
}

/// Like [`AuthedAgent`], but additionally requires the admin flag.
pub struct AdminAgent(pub crate::api::Agent);

impl FromRequestParts<AppState> for AdminAgent {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let AuthedAgent(agent) = AuthedAgent::from_request_parts(parts, state).await?;
        if !agent.is_admin {
            return Err(ApiError::Forbidden("admin privileges required".into()));
        }
        Ok(AdminAgent(agent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_url_safe() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    #[test]
    fn hashing_is_stable_and_hex() {
        let h = hash_token("hunter2");
        assert_eq!(h, hash_token("hunter2"));
        assert_eq!(h.len(), 64);
        assert_ne!(h, hash_token("hunter3"));
    }
}
