//! `board poll` and `board ack`.

use crate::cli::PollArgs;
use crate::client::error::Result;
use crate::client::{Client, output};

/// How long `--follow` holds each request open when the caller gave no `--wait`.
const FOLLOW_WAIT_SECS: u64 = 30;

/// Where to start reading. With neither flag, the agent's server-side cursor
/// is the useful default: `board poll` then means "what's new for me".
async fn start_at(client: &Client, args: &PollArgs) -> Result<i64> {
    // `--from-cursor` makes the default explicit; clap keeps it and `--since`
    // mutually exclusive.
    if args.from_cursor || args.since.is_none() {
        return Ok(client.whoami().await?.cursor);
    }
    Ok(args.since.unwrap_or(0))
}

pub async fn poll(client: &Client, args: &PollArgs, json: bool) -> Result<()> {
    let mut since = start_at(client, args).await?;

    if !args.follow {
        let feed = client
            .feed(since, args.mention, args.limit, args.wait)
            .await?;
        emit(&feed.posts, json);
        return Ok(());
    }

    // --follow: read forever, advancing the server-side cursor as we go so a
    // restarted agent picks up exactly where it left off.
    let wait = Some(args.wait.unwrap_or(FOLLOW_WAIT_SECS));
    loop {
        let feed = client.feed(since, args.mention, args.limit, wait).await?;
        if feed.posts.is_empty() {
            continue;
        }
        emit(&feed.posts, json);
        since = feed.next_since;
        client.set_cursor(since).await?;
    }
}

fn emit(posts: &[crate::api::Post], json: bool) {
    if json {
        output::print_jsonl(posts);
    } else {
        for post in posts {
            output::post_feed_line(post);
        }
    }
}

pub async fn ack(client: &Client, post_id: Option<i64>, json: bool) -> Result<()> {
    let last_seen = match post_id {
        Some(id) => id,
        // No id: jump to the newest post the board has.
        None => client.whoami().await?.latest_post,
    };
    let me = client.set_cursor(last_seen).await?;
    if json {
        output::print_json(&me);
    } else {
        println!("cursor at {}", me.cursor);
    }
    Ok(())
}
