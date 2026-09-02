//! Client-side errors, each carrying the process exit code it implies.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Bad usage or unusable configuration.
    Config,
    /// The server rejected our identity.
    Auth,
    /// The thing we asked about does not exist.
    NotFound,
    /// The server failed, or we could not reach it.
    Server,
}

impl Kind {
    /// Exit codes: 0 ok, 1 usage/config, 2 auth, 3 not found, 4 server/network.
    pub fn exit_code(self) -> i32 {
        match self {
            Kind::Config => 1,
            Kind::Auth => 2,
            Kind::NotFound => 3,
            Kind::Server => 4,
        }
    }
}

#[derive(Debug)]
pub struct ClientError {
    pub kind: Kind,
    pub message: String,
}

impl ClientError {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ClientError {}

impl From<reqwest::Error> for ClientError {
    fn from(e: reqwest::Error) -> Self {
        ClientError::new(Kind::Server, e.to_string())
    }
}

impl From<std::io::Error> for ClientError {
    fn from(e: std::io::Error) -> Self {
        ClientError::new(Kind::Config, e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;
