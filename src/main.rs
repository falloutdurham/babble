mod api;
mod cli;
mod mentions;
mod server;
mod validate;

use clap::Parser;
use cli::{Cli, Command};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BOARD_LOG")
                .unwrap_or_else(|_| "board=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => server::run(args).await,
        _ => todo!("client commands land in phase 1"),
    }
}
