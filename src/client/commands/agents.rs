//! `board agent add`, `board agent list`, `board whoami`.

use crate::cli::AgentCommand;
use crate::client::error::Result;
use crate::client::output::Format;
use crate::client::{Client, output};

pub async fn run(client: &Client, cmd: &AgentCommand, fmt: Format) -> Result<()> {
    match cmd {
        AgentCommand::Add { name, admin } => add(client, name, *admin, fmt).await,
        AgentCommand::List => list(client, fmt).await,
    }
}

async fn add(client: &Client, name: &str, admin: bool, fmt: Format) -> Result<()> {
    let created = client.create_agent(name, admin).await?;
    match fmt {
        Format::Json => output::print_json(&created),
        Format::Markdown => {
            println!("Created agent **{}**.\n", created.name);
            println!("```\n{}\n```\n", created.token);
            println!("*This token is shown once; store it now.*");
        }
        Format::Table => {
            println!("created agent {}", created.name);
            println!("token: {}", created.token);
            println!("(this token is shown once; store it now)");
        }
    }
    Ok(())
}

async fn list(client: &Client, fmt: Format) -> Result<()> {
    let agents = client.list_agents().await?.agents;
    output::agents(&agents, fmt);
    Ok(())
}

pub async fn whoami(client: &Client, fmt: Format) -> Result<()> {
    let me = client.whoami().await?;
    output::me(&me, fmt);
    Ok(())
}
