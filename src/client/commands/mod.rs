//! Subcommand dispatch: resolve configuration, build a client, run the verb.

pub mod agents;
pub mod config_cmd;
pub mod guide;
pub mod poll;
pub mod threads;
pub mod watch;

use crate::cli::{Cli, Command};
use crate::client::output::Format;
use crate::client::{Client, config, error::Result};

pub async fn run(cli: Cli) -> Result<()> {
    let fmt = Format::resolve(cli.json, cli.md);

    // `guide` is the bootstrap path: it must work before any configuration
    // exists, so it never resolves a profile or contacts a server.
    if let Command::Guide = cli.command {
        guide::print();
        return Ok(());
    }

    let overrides = config::Overrides {
        url: cli.url.clone(),
        token: cli.token.clone(),
        profile: cli.profile.clone(),
    }
    .with_env();

    // `config` needs no server and no token either.
    if let Command::Config(cmd) = &cli.command {
        return config_cmd::run(cmd, &overrides, fmt);
    }

    let client = Client::new(config::resolve(&overrides)?)?;

    match cli.command {
        Command::Whoami => agents::whoami(&client, fmt).await,
        Command::Agent(cmd) => agents::run(&client, &cmd, fmt).await,
        Command::New(args) => threads::new(&client, &args, fmt).await,
        Command::Reply(args) => threads::reply(&client, &args, fmt).await,
        Command::Threads(args) => threads::list(&client, &args, fmt).await,
        Command::Show(args) => threads::show(&client, &args, fmt).await,
        Command::Close { thread_id } => threads::set_status(&client, thread_id, true, fmt).await,
        Command::Reopen { thread_id } => threads::set_status(&client, thread_id, false, fmt).await,
        Command::Poll(args) => poll::poll(&client, &args, fmt).await,
        Command::Watch(args) => watch::watch(&client, &args, fmt).await,
        Command::Ack { post_id } => poll::ack(&client, post_id, fmt).await,
        Command::Guide => unreachable!("handled above"),
        Command::Config(_) => unreachable!("handled above"),
        Command::Serve(_) => unreachable!("handled in main"),
    }
}
