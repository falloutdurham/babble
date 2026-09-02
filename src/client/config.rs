//! `~/.config/board/config.toml`, plus the flag > env > profile precedence
//! rules that turn it into a URL and token.

use crate::client::error::{ClientError, Kind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DEFAULT_URL: &str = "http://127.0.0.1:7420";
pub const DEFAULT_PROFILE: &str = "local";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Where the config lives. `BOARD_CONFIG` overrides it, which keeps tests off
/// the real user config.
pub fn config_path() -> Result<PathBuf, ClientError> {
    if let Ok(p) = std::env::var("BOARD_CONFIG") {
        return Ok(PathBuf::from(p));
    }
    let dirs = directories::ProjectDirs::from("", "", "board")
        .ok_or_else(|| ClientError::new(Kind::Config, "cannot determine a config directory"))?;
    Ok(dirs.config_dir().join("config.toml"))
}

pub fn load() -> Result<ConfigFile, ClientError> {
    let path = config_path()?;
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ConfigFile::default()),
        Err(e) => {
            return Err(ClientError::new(
                Kind::Config,
                format!("reading {}: {e}", path.display()),
            ));
        }
    };
    toml::from_str(&raw).map_err(|e| {
        ClientError::new(Kind::Config, format!("parsing {}: {e}", path.display()))
    })
}

pub fn save(cfg: &ConfigFile) -> Result<PathBuf, ClientError> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            ClientError::new(Kind::Config, format!("creating {}: {e}", parent.display()))
        })?;
    }
    let raw = toml::to_string_pretty(cfg)
        .map_err(|e| ClientError::new(Kind::Config, format!("serialising config: {e}")))?;
    std::fs::write(&path, raw)
        .map_err(|e| ClientError::new(Kind::Config, format!("writing {}: {e}", path.display())))?;
    Ok(path)
}

/// A fully resolved connection: where to talk, and as whom.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub url: String,
    pub token: String,
}

/// The overrides that outrank the config file, in order: CLI flags, then
/// environment variables.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub url: Option<String>,
    pub token: Option<String>,
    pub profile: Option<String>,
}

impl Overrides {
    /// Fold in `BOARD_URL` / `BOARD_TOKEN` / `BOARD_PROFILE` wherever a flag
    /// did not already supply a value.
    pub fn with_env(mut self) -> Self {
        fn env(key: &str) -> Option<String> {
            std::env::var(key).ok().filter(|v| !v.is_empty())
        }
        self.url = self.url.or_else(|| env("BOARD_URL"));
        self.token = self.token.or_else(|| env("BOARD_TOKEN"));
        self.profile = self.profile.or_else(|| env("BOARD_PROFILE"));
        self
    }
}

/// Resolve against an already-loaded config, so precedence can be unit tested
/// without touching the filesystem.
pub fn resolve_with(cfg: &ConfigFile, ov: &Overrides) -> Result<Resolved, ClientError> {
    let profile_name = ov
        .profile
        .clone()
        .or_else(|| cfg.default_profile.clone())
        .unwrap_or_else(|| DEFAULT_PROFILE.to_string());

    // An explicitly requested profile that does not exist is a config error;
    // falling back to the default profile silently would hide the typo.
    let profile = match cfg.profiles.get(&profile_name) {
        Some(p) => Some(p),
        None if ov.profile.is_some() => {
            return Err(ClientError::new(
                Kind::Config,
                format!("no profile named '{profile_name}' in the config file"),
            ));
        }
        None => None,
    };

    let url = ov
        .url
        .clone()
        .or_else(|| profile.map(|p| p.url.clone()))
        .unwrap_or_else(|| DEFAULT_URL.to_string());

    let token = ov
        .token
        .clone()
        .or_else(|| profile.and_then(|p| p.token.clone()))
        .ok_or_else(|| {
            ClientError::new(
                Kind::Config,
                "no token: pass --token, set BOARD_TOKEN, or run `board config init`",
            )
        })?;

    Ok(Resolved {
        url: url.trim_end_matches('/').to_string(),
        token,
    })
}

pub fn resolve(ov: &Overrides) -> Result<Resolved, ClientError> {
    resolve_with(&load()?, ov)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ConfigFile {
        let mut profiles = BTreeMap::new();
        profiles.insert(
            "local".into(),
            Profile {
                url: "http://local".into(),
                token: Some("local-token".into()),
            },
        );
        profiles.insert(
            "prod".into(),
            Profile {
                url: "https://prod/".into(),
                token: Some("prod-token".into()),
            },
        );
        ConfigFile {
            default_profile: Some("local".into()),
            profiles,
        }
    }

    #[test]
    fn falls_back_to_the_default_profile() {
        let r = resolve_with(&cfg(), &Overrides::default()).unwrap();
        assert_eq!(r.url, "http://local");
        assert_eq!(r.token, "local-token");
    }

    #[test]
    fn named_profile_wins_over_default_and_loses_to_flags() {
        let ov = Overrides {
            profile: Some("prod".into()),
            ..Default::default()
        };
        let r = resolve_with(&cfg(), &ov).unwrap();
        assert_eq!(r.url, "https://prod");
        assert_eq!(r.token, "prod-token");

        let ov = Overrides {
            profile: Some("prod".into()),
            token: Some("flag-token".into()),
            url: Some("http://flag".into()),
        };
        let r = resolve_with(&cfg(), &ov).unwrap();
        assert_eq!(r.url, "http://flag");
        assert_eq!(r.token, "flag-token");
    }

    #[test]
    fn unknown_named_profile_is_an_error() {
        let ov = Overrides {
            profile: Some("nope".into()),
            ..Default::default()
        };
        assert!(resolve_with(&cfg(), &ov).is_err());
    }

    #[test]
    fn missing_token_is_a_config_error() {
        let ov = Overrides {
            url: Some("http://x".into()),
            ..Default::default()
        };
        assert!(resolve_with(&ConfigFile::default(), &ov).is_err());
    }
}
