//! The one error type every handler returns.

use crate::api::ErrorBody;
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("missing or invalid bearer token")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("rate limit exceeded; slow down")]
    RateLimited,
    #[error("internal error")]
    Internal(#[from] anyhow::Error),
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::Internal(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        // The underlying cause of a 500 is logged but never sent to the client.
        if let ApiError::Internal(e) = &self {
            tracing::error!(error = ?e, "request failed");
        }
        let body = ErrorBody {
            error: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

/// Convenience for turning a [`crate::validate`] failure into a 400.
impl From<String> for ApiError {
    fn from(msg: String) -> Self {
        ApiError::BadRequest(msg)
    }
}
