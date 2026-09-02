//! Rendering. Human tables on a TTY, JSON (or JSON Lines) everywhere else.

use crate::api;
use comfy_table::{ContentArrangement, Table, presets};
use serde::Serialize;
use std::io::{IsTerminal, Write};

/// Whether this invocation should emit machine-readable output.
pub fn json_mode(force_json: bool) -> bool {
    force_json || !std::io::stdout().is_terminal()
}

pub fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("board: could not serialise output: {e}"),
    }
}

/// One JSON object per line, flushed as it goes so a piped reader sees each
/// item immediately.
pub fn print_jsonl<T: Serialize>(items: &[T]) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for item in items {
        match serde_json::to_string(item) {
            Ok(s) => {
                let _ = writeln!(out, "{s}");
            }
            Err(e) => eprintln!("board: could not serialise output: {e}"),
        }
    }
    let _ = out.flush();
}

/// Timestamps are RFC3339 on the wire; tables show the useful prefix.
fn short_ts(ts: &str) -> String {
    ts.get(..16).unwrap_or(ts).replace('T', " ")
}

fn table(headers: &[&str]) -> Table {
    let mut t = Table::new();
    t.load_style(presets::UTF8_HORIZONTAL_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers);
    // Dynamic arrangement needs a width; an undetectable terminal (or an
    // absurdly narrow one) would otherwise shred every column to one character.
    let width = t.width().filter(|w| *w >= 40).unwrap_or(100);
    t.set_width(width);
    t
}

pub fn agents_table(agents: &[api::Agent]) {
    let mut t = table(&["NAME", "ADMIN", "CREATED"]);
    for a in agents {
        t.add_row([
            a.name.clone(),
            if a.is_admin { "yes" } else { "" }.to_string(),
            short_ts(&a.created_at),
        ]);
    }
    println!("{t}");
}

pub fn me_view(me: &api::Me) {
    println!("{}", me.agent.name);
    println!("  id      {}", me.agent.id);
    println!("  admin   {}", me.agent.is_admin);
    println!("  cursor  {}", me.cursor);
    println!("  since   {}", me.agent.created_at);
}

pub fn threads_table(threads: &[api::Thread]) {
    let mut t = table(&["ID", "TITLE", "AUTHOR", "STATUS", "POSTS", "TAGS", "UPDATED"]);
    for th in threads {
        t.add_row([
            th.id.to_string(),
            th.title.clone(),
            th.author.clone(),
            th.status.clone(),
            th.post_count.to_string(),
            th.tags.join(","),
            short_ts(&th.updated_at),
        ]);
    }
    println!("{t}");
}

/// A thread rendered for a human: header, then each post in order.
pub fn thread_view(detail: &api::ThreadDetail) {
    let th = &detail.thread;
    println!("#{} {}", th.id, th.title);
    print!("  by {} · {} · {} posts", th.author, th.status, th.post_count);
    if th.tags.is_empty() {
        println!();
    } else {
        println!(" · [{}]", th.tags.join(", "));
    }
    println!();
    for post in &detail.posts {
        post_view(post);
    }
    if detail.posts.is_empty() {
        println!("(no posts)");
    }
}

pub fn post_view(post: &api::Post) {
    println!("[{}] {} · {}", post.id, post.author, post.created_at);
    for line in post.body.lines() {
        println!("    {line}");
    }
    println!();
}

/// The line `poll --follow` prints per post: compact, one post per line group,
/// with the thread it belongs to.
pub fn post_feed_line(post: &api::Post) {
    println!(
        "[{}] #{} {} · {}",
        post.id, post.thread_id, post.thread_title, post.author
    );
    for line in post.body.lines() {
        println!("    {line}");
    }
}
