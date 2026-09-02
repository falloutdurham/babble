use board::cli::{Cli, Command};
use board::{client, server};
use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BOARD_LOG")
                .unwrap_or_else(|_| "board=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    if let Command::Serve(args) = cli.command {
        return server::run(args).await;
    }

    // Client commands map their failure onto a documented exit code rather
    // than bubbling up as an anyhow backtrace.
    if let Err(e) = client::commands::run(cli).await {
        eprintln!("board: {e}");
        std::process::exit(e.kind.exit_code());
    }
    Ok(())
}
