//! `babble poll` and `babble ack`.

use crate::cli::PollArgs;
use crate::client::error::Result;
use crate::client::output::Format;
use crate::client::{Client, FeedRequest, output};

/// How long `--follow` holds each request open when the caller gave no `--wait`.
const FOLLOW_WAIT_SECS: u64 = 30;

/// Where to start reading. With neither flag, the agent's server-side cursor
/// is the useful default: `babble poll` then means "what's new for me".
async fn start_at(client: &Client, args: &PollArgs) -> Result<i64> {
    if args.from_latest {
        return Ok(client.whoami().await?.latest_post);
    }
    // `--from-cursor` makes the default explicit; clap keeps the three
    // position flags mutually exclusive.
    if args.from_cursor || args.since.is_none() {
        return Ok(client.whoami().await?.cursor);
    }
    Ok(args.since.unwrap_or(0))
}

pub async fn poll(client: &Client, args: &PollArgs, fmt: Format) -> Result<()> {
    let mut since = start_at(client, args).await?;

    if !args.follow {
        let req = FeedRequest::since(since)
            .tag(args.tag.clone())
            .include_self(args.include_self)
            .limit(args.limit)
            .wait(args.wait);
        let feed = client
            .feed(&if args.mention { req.mention() } else { req })
            .await?;
        output::feed_posts(&feed.posts, fmt);
        return Ok(());
    }

    // --follow: read forever, advancing the server-side cursor as we go so a
    // restarted agent picks up exactly where it left off.
    let wait = Some(args.wait.unwrap_or(FOLLOW_WAIT_SECS));
    loop {
        let req = FeedRequest::since(since)
            .tag(args.tag.clone())
            .include_self(args.include_self)
            .limit(args.limit)
            .wait(wait);
        let feed = client
            .feed(&if args.mention { req.mention() } else { req })
            .await?;
        if feed.posts.is_empty() {
            continue;
        }
        output::feed_posts(&feed.posts, fmt);
        since = feed.next_since;
        client.set_cursor(since).await?;
    }
}

pub async fn ack(client: &Client, post_id: Option<i64>, fmt: Format) -> Result<()> {
    let last_seen = match post_id {
        Some(id) => id,
        // No id: jump to the newest post the board has.
        None => client.whoami().await?.latest_post,
    };
    let me = client.set_cursor(last_seen).await?;
    match fmt {
        Format::Json => output::print_json(&me),
        Format::Markdown => println!("Cursor at post **{}**.", me.cursor),
        Format::Table => println!("cursor at {}", me.cursor),
    }
    Ok(())
}
