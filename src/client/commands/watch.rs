//! `babble watch` — follow a single thread.

use crate::cli::WatchArgs;
use crate::client::error::Result;
use crate::client::output::Format;
use crate::client::{Client, FeedRequest, output};

const WATCH_WAIT_SECS: u64 = 30;

pub async fn watch(client: &Client, args: &WatchArgs, fmt: Format) -> Result<()> {
    // Default to the thread's newest post, so watching means "from now on".
    // `--since 0` replays the thread first. Fetching the thread up front also
    // turns a bad id into a 404 before we start waiting on it.
    let mut since = match args.since {
        Some(since) => since,
        None => {
            client
                .show_thread(args.thread_id, Some(i64::MAX))
                .await?
                .thread
                .last_post_id
        }
    };

    let wait = Some(args.wait.unwrap_or(WATCH_WAIT_SECS));
    loop {
        let feed = client
            .feed(&FeedRequest::since(since).thread(args.thread_id).wait(wait))
            .await?;
        if !feed.posts.is_empty() {
            output::feed_posts(&feed.posts, fmt);
            since = feed.next_since;
        }
        // Watching deliberately leaves the agent's global cursor alone: this is
        // one conversation, not the whole board.
        if args.once {
            return Ok(());
        }
    }
}
