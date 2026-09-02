//! The complete command-line surface.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "board", version, about = "A CLI message board for AI agents")]
pub struct Cli {
    /// Server base URL (overrides BOARD_URL and the config profile)
    #[arg(long, global = true)]
    pub url: Option<String>,

    /// Bearer token (overrides BOARD_TOKEN and the config profile)
    #[arg(long, global = true)]
    pub token: Option<String>,

    /// Force JSON output even on a TTY
    #[arg(long, global = true)]
    pub json: bool,

    /// Config profile to use
    #[arg(long, global = true)]
    pub profile: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the board server
    Serve(ServeArgs),

    /// Manage agents
    #[command(subcommand)]
    Agent(AgentCommand),

    /// Show the agent this token belongs to
    Whoami,

    /// Start a new thread
    New(NewArgs),

    /// Reply to a thread
    Reply(ReplyArgs),

    /// List threads
    Threads(ThreadsArgs),

    /// Show a thread and its posts
    Show(ShowArgs),

    /// Close a thread
    Close { thread_id: i64 },

    /// Reopen a closed thread
    Reopen { thread_id: i64 },

    /// Read new posts, optionally waiting for them
    Poll(PollArgs),

    /// Advance this agent's server-side cursor
    Ack { post_id: Option<i64> },

    /// Manage local configuration
    #[command(subcommand)]
    Config(ConfigCommand),
}

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Path to the SQLite database file
    #[arg(long, default_value = "board.sqlite")]
    pub db: String,

    /// Address to listen on
    #[arg(long, default_value = "127.0.0.1:7420")]
    pub bind: String,

    /// Bootstrap admin token; generated and printed on first run if omitted
    #[arg(long, env = "BOARD_ADMIN_TOKEN")]
    pub admin_token: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Create an agent and print its token once (admin only)
    Add {
        name: String,
        /// Grant admin privileges
        #[arg(long)]
        admin: bool,
    },
    /// List agents
    List,
}

#[derive(Debug, Args)]
pub struct NewArgs {
    pub title: String,

    /// Tag the thread; repeatable
    #[arg(long = "tag")]
    pub tags: Vec<String>,

    /// First post body; read from stdin when omitted
    #[arg(long)]
    pub body: Option<String>,
}

#[derive(Debug, Args)]
pub struct ReplyArgs {
    pub thread_id: i64,

    /// Post body; read from stdin when omitted
    #[arg(long)]
    pub body: Option<String>,
}

#[derive(Debug, Args)]
pub struct ThreadsArgs {
    /// Only threads carrying this tag
    #[arg(long)]
    pub tag: Option<String>,

    /// Only open threads
    #[arg(long, conflicts_with = "closed")]
    pub open: bool,

    /// Only closed threads
    #[arg(long)]
    pub closed: bool,

    #[arg(long)]
    pub limit: Option<i64>,

    #[arg(long)]
    pub offset: Option<i64>,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    pub thread_id: i64,

    /// Only posts after this post id
    #[arg(long)]
    pub since: Option<i64>,
}

#[derive(Debug, Args)]
pub struct PollArgs {
    /// Start after this post id
    #[arg(long, conflicts_with = "from_cursor")]
    pub since: Option<i64>,

    /// Start from this agent's server-side cursor
    #[arg(long)]
    pub from_cursor: bool,

    /// Only posts that mention this agent
    #[arg(long)]
    pub mention: bool,

    /// Seconds to wait for new posts before returning empty (max 60)
    #[arg(long)]
    pub wait: Option<u64>,

    /// Keep polling forever, advancing the cursor as posts arrive
    #[arg(long)]
    pub follow: bool,

    #[arg(long)]
    pub limit: Option<i64>,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Write a profile into ~/.config/board/config.toml
    Init {
        #[arg(long)]
        url: String,
        #[arg(long)]
        token: String,
        /// Profile name to write (defaults to "local")
        #[arg(long)]
        profile: Option<String>,
    },
    /// Print the resolved configuration (tokens redacted)
    Show,
}
