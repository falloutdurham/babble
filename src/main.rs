use babble::cli::{Cli, Command};
use babble::{client, server, web};
use clap::Parser;

/// Snapshot a database file. Like `serve`, this works on a path rather than
/// through the API, so it needs no token and can run from a cron job.
fn backup(args: babble::cli::BackupArgs, json: bool) -> anyhow::Result<()> {
    let to = args.to.unwrap_or_else(|| {
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        format!("{}.{stamp}.backup", args.db)
    });
    let bytes = babble::server::db::backup(&args.db, &to)?;
    if json {
        println!(
            "{}",
            serde_json::json!({ "from": args.db, "to": to, "bytes": bytes })
        );
    } else {
        println!("wrote {to} ({bytes} bytes)");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("BABBLE_LOG")
                .unwrap_or_else(|_| "babble=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();
    // Resolve before dispatch: `web` is a server, but it reaches the board as
    // a client and so needs the same url/token precedence every command uses.
    let overrides = client::commands::overrides(&cli);
    match cli.command {
        Command::Serve(args) => return server::run(args).await,
        Command::Backup(args) => return backup(args, cli.json),
        Command::Web(args) => {
            return web::run(args, client::config::resolve(&overrides)?).await;
        }
        _ => {}
    }

    // Client commands map their failure onto a documented exit code rather
    // than bubbling up as an anyhow backtrace.
    if let Err(e) = client::commands::run(cli).await {
        eprintln!("babble: {e}");
        std::process::exit(e.kind.exit_code());
    }
    Ok(())
}
