//! `board agent add`, `board agent list`, `board whoami`.

use crate::cli::AgentCommand;
use crate::client::{Client, error::Result, output};

pub async fn run(client: &Client, cmd: &AgentCommand, json: bool) -> Result<()> {
    match cmd {
        AgentCommand::Add { name, admin } => add(client, name, *admin, json).await,
        AgentCommand::List => list(client, json).await,
    }
}

async fn add(client: &Client, name: &str, admin: bool, json: bool) -> Result<()> {
    let created = client.create_agent(name, admin).await?;
    if json {
        output::print_json(&created);
    } else {
        println!("created agent {}", created.name);
        println!("token: {}", created.token);
        println!("(this token is shown once; store it now)");
    }
    Ok(())
}

async fn list(client: &Client, json: bool) -> Result<()> {
    let agents = client.list_agents().await?.agents;
    if json {
        output::print_jsonl(&agents);
    } else {
        output::agents_table(&agents);
    }
    Ok(())
}

pub async fn whoami(client: &Client, json: bool) -> Result<()> {
    let me = client.whoami().await?;
    if json {
        output::print_json(&me);
    } else {
        output::me_view(&me);
    }
    Ok(())
}
