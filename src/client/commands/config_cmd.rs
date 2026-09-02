//! `babble config init` and `babble config show`.

use crate::cli::ConfigCommand;
use crate::client::config::{self, Overrides, Profile};
use crate::client::error::Result;
use crate::client::output;
use crate::client::output::Format;

pub fn run(cmd: &ConfigCommand, overrides: &Overrides, fmt: Format) -> Result<()> {
    match cmd {
        ConfigCommand::Init {
            url,
            token,
            profile,
        } => init(url, token, profile.as_deref(), fmt),
        ConfigCommand::Show => show(overrides, fmt),
    }
}

fn init(url: &str, token: &str, profile: Option<&str>, fmt: Format) -> Result<()> {
    let name = profile.unwrap_or(config::DEFAULT_PROFILE).to_string();
    let mut cfg = config::load()?;
    cfg.profiles.insert(
        name.clone(),
        Profile {
            url: url.trim_end_matches('/').to_string(),
            token: Some(token.to_string()),
        },
    );
    // The first profile written becomes the default.
    if cfg.default_profile.is_none() {
        cfg.default_profile = Some(name.clone());
    }
    let path = config::save(&cfg)?;

    if fmt.is_json() {
        output::print_json(&serde_json::json!({
            "profile": name,
            "path": path.to_string_lossy(),
        }));
    } else {
        println!("wrote profile '{name}' to {}", path.display());
    }
    Ok(())
}

fn show(overrides: &Overrides, fmt: Format) -> Result<()> {
    let resolved = config::resolve(overrides)?;
    let path = config::config_path()?;
    // Tokens are never printed back out, only their presence.
    let view = serde_json::json!({
        "config_path": path.to_string_lossy(),
        "url": resolved.url,
        "token": "<redacted>",
    });
    if fmt.is_json() {
        output::print_json(&view);
    } else {
        println!("config  {}", path.display());
        println!("url     {}", resolved.url);
        println!("token   <redacted>");
    }
    Ok(())
}
