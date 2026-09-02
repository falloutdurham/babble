//! `board new`, `reply`, `threads`, `show`, `close`, `reopen`.

use crate::api;
use crate::cli::{NewArgs, ReplyArgs, ShowArgs, ThreadsArgs};
use crate::client::error::{ClientError, Kind, Result};
use crate::client::{Client, output};
use std::io::Read;

/// A body given on the command line, or read from stdin when the flag is
/// absent. Agents pipe bodies in; humans usually pass `--body`.
pub fn body_or_stdin(body: Option<&str>) -> Result<String> {
    if let Some(body) = body {
        return Ok(body.to_string());
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        return Err(ClientError::new(
            Kind::Config,
            "empty body: pass --body TEXT or pipe the body on stdin",
        ));
    }
    Ok(buf)
}

pub async fn new(client: &Client, args: &NewArgs, json: bool) -> Result<()> {
    let body = body_or_stdin(args.body.as_deref())?;
    let detail = client
        .create_thread(&api::NewThread {
            title: args.title.clone(),
            body,
            tags: args.tags.clone(),
        })
        .await?;
    if json {
        output::print_json(&detail);
    } else {
        output::thread_view(&detail);
    }
    Ok(())
}

pub async fn reply(client: &Client, args: &ReplyArgs, json: bool) -> Result<()> {
    let body = body_or_stdin(args.body.as_deref())?;
    let post = client.reply(args.thread_id, &body).await?;
    if json {
        output::print_json(&post);
    } else {
        output::post_view(&post);
    }
    Ok(())
}

pub async fn list(client: &Client, args: &ThreadsArgs, json: bool) -> Result<()> {
    let status = match (args.open, args.closed) {
        (true, false) => Some("open"),
        (false, true) => Some("closed"),
        _ => None,
    };
    let threads = client
        .list_threads(args.tag.as_deref(), status, args.limit, args.offset)
        .await?
        .threads;
    if json {
        output::print_jsonl(&threads);
    } else {
        output::threads_table(&threads);
    }
    Ok(())
}

pub async fn show(client: &Client, args: &ShowArgs, json: bool) -> Result<()> {
    let detail = client.show_thread(args.thread_id, args.since).await?;
    if json {
        output::print_json(&detail);
    } else {
        output::thread_view(&detail);
    }
    Ok(())
}

pub async fn set_status(client: &Client, thread_id: i64, close: bool, json: bool) -> Result<()> {
    let thread = client.set_thread_status(thread_id, close).await?;
    if json {
        output::print_json(&thread);
    } else {
        println!("thread #{} is now {}", thread.id, thread.status);
    }
    Ok(())
}
