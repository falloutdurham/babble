//! `babble new`, `reply`, `threads`, `show`, `close`, `reopen`.

use crate::api;
use crate::cli::{NewArgs, ReplyArgs, ShowArgs, ThreadsArgs};
use crate::client::error::{ClientError, Kind, Result};
use crate::client::output::Format;
use crate::client::{Client, output};
use std::io::Read;

/// A body given on the command line, or read from stdin when the flag is
/// absent. Agents pipe bodies in; humans usually pass `--body`.
pub fn body_or_stdin(body: Option<&str>) -> Result<String> {
    // `-` is the near-universal "read stdin" sentinel; taking it literally
    // silently posts a one-character body.
    if let Some(body) = body.filter(|b| *b != "-") {
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

pub async fn new(client: &Client, args: &NewArgs, fmt: Format) -> Result<()> {
    let body = body_or_stdin(args.body.as_deref())?;
    let detail = client
        .create_thread(&api::NewThread {
            title: args.title.clone(),
            body,
            tags: args.tags.clone(),
        })
        .await?;
    output::thread_detail(&detail, fmt);
    Ok(())
}

pub async fn reply(client: &Client, args: &ReplyArgs, fmt: Format) -> Result<()> {
    let body = body_or_stdin(args.body.as_deref())?;
    let post = client.reply(args.thread_id, &body).await?;
    output::post(&post, fmt);
    Ok(())
}

pub async fn list(client: &Client, args: &ThreadsArgs, fmt: Format) -> Result<()> {
    let status = match (args.open, args.closed) {
        (true, false) => Some("open"),
        (false, true) => Some("closed"),
        _ => None,
    };
    let threads = client
        .list_threads(args.tag.as_deref(), status, args.limit, args.offset)
        .await?
        .threads;
    output::threads(&threads, fmt);
    Ok(())
}

pub async fn show(client: &Client, args: &ShowArgs, fmt: Format) -> Result<()> {
    let req = crate::client::ShowRequest::thread(args.thread_id)
        .since(args.since)
        .limit(args.limit)
        .tail(args.tail);
    let detail = client.show_thread(&req).await?;
    output::thread_detail(&detail, fmt);
    Ok(())
}

pub async fn react(client: &Client, args: &crate::cli::ReactArgs, fmt: Format) -> Result<()> {
    let post = if args.remove {
        client.unreact(args.post_id, &args.emoji).await?
    } else {
        client.react(args.post_id, &args.emoji).await?
    };
    output::post(&post, fmt);
    Ok(())
}

pub async fn set_status(client: &Client, thread_id: i64, close: bool, fmt: Format) -> Result<()> {
    let thread = client.set_thread_status(thread_id, close).await?;
    match fmt {
        Format::Json => output::print_json(&thread),
        Format::Markdown => println!("Thread **#{}** is now `{}`.", thread.id, thread.status),
        Format::Table => println!("thread #{} is now {}", thread.id, thread.status),
    }
    Ok(())
}
