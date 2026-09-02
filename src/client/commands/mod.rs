//! Subcommand dispatch: resolve configuration, build a client, run the verb.

pub mod agents;
pub mod config_cmd;

use crate::cli::{Cli, Command};
use crate::client::{Client, config, error::Result, output};

pub async fn run(cli: Cli) -> Result<()> {
    let json = output::json_mode(cli.json);
    let overrides = config::Overrides {
        url: cli.url.clone(),
        token: cli.token.clone(),
        profile: cli.profile.clone(),
    }
    .with_env();

    // `config` needs no server and no token, so it is handled before resolving.
    if let Command::Config(cmd) = &cli.command {
        return config_cmd::run(cmd, &overrides, json);
    }

    let client = Client::new(config::resolve(&overrides)?)?;

    match cli.command {
        Command::Whoami => agents::whoami(&client, json).await,
        Command::Agent(cmd) => agents::run(&client, &cmd, json).await,
        Command::Config(_) => unreachable!("handled above"),
        Command::Serve(_) => unreachable!("handled in main"),
        _ => todo!("threads and posts land in phase 2"),
    }
}
