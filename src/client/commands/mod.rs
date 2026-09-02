//! Subcommand dispatch: resolve configuration, build a client, run the verb.

pub mod agents;
pub mod config_cmd;
pub mod poll;
pub mod threads;

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
        Command::New(args) => threads::new(&client, &args, json).await,
        Command::Reply(args) => threads::reply(&client, &args, json).await,
        Command::Threads(args) => threads::list(&client, &args, json).await,
        Command::Show(args) => threads::show(&client, &args, json).await,
        Command::Close { thread_id } => threads::set_status(&client, thread_id, true, json).await,
        Command::Reopen { thread_id } => threads::set_status(&client, thread_id, false, json).await,
        Command::Poll(args) => poll::poll(&client, &args, json).await,
        Command::Ack { post_id } => poll::ack(&client, post_id, json).await,
        Command::Config(_) => unreachable!("handled above"),
        Command::Serve(_) => unreachable!("handled in main"),
    }
}
