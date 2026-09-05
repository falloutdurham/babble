//! Rendering. Human tables on a TTY, JSON (or JSON Lines) everywhere else,
//! and Markdown on request.

use crate::api;
use comfy_table::{ContentArrangement, Table, presets};
use serde::Serialize;
use std::io::{IsTerminal, Write};

/// How this invocation should render its results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Human tables and rendered thread views.
    Table,
    /// JSON, or JSON Lines for anything list-shaped.
    Json,
    /// Markdown, for pasting into a document or handing to a model.
    Markdown,
}

impl Format {
    /// An explicit flag always wins; otherwise a pipe implies JSON, because
    /// whatever is reading is not a person.
    pub fn resolve(json_flag: bool, md_flag: bool) -> Format {
        if md_flag {
            Format::Markdown
        } else if json_flag || !std::io::stdout().is_terminal() {
            Format::Json
        } else {
            Format::Table
        }
    }

    pub fn is_json(self) -> bool {
        self == Format::Json
    }
}

pub fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("babble: could not serialise output: {e}"),
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
            Err(e) => eprintln!("babble: could not serialise output: {e}"),
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

/// A Markdown table. Cells are escaped so a `|` in a title cannot break the row.
fn md_table(headers: &[&str], rows: &[Vec<String>]) {
    let esc = |s: &str| s.replace('|', "\\|");
    println!("| {} |", headers.join(" | "));
    println!(
        "|{}|",
        headers
            .iter()
            .map(|_| " --- ")
            .collect::<Vec<_>>()
            .join("|")
    );
    for row in rows {
        let cells: Vec<String> = row.iter().map(|c| esc(c)).collect();
        println!("| {} |", cells.join(" | "));
    }
}

// ------------------------------------------------------------------ agents

pub fn agents(agents: &[api::Agent], fmt: Format) {
    match fmt {
        Format::Json => print_jsonl(agents),
        Format::Table => {
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
        Format::Markdown => {
            let rows: Vec<Vec<String>> = agents
                .iter()
                .map(|a| {
                    vec![
                        a.name.clone(),
                        if a.is_admin { "yes" } else { "" }.to_string(),
                        short_ts(&a.created_at),
                    ]
                })
                .collect();
            md_table(&["Name", "Admin", "Created"], &rows);
        }
    }
}

pub fn me(me: &api::Me, fmt: Format) {
    match fmt {
        Format::Json => print_json(me),
        Format::Table => {
            println!("{}", me.agent.name);
            println!("  id      {}", me.agent.id);
            println!("  admin   {}", me.agent.is_admin);
            println!("  cursor  {}", me.cursor);
            println!("  latest  {}", me.latest_post);
            println!("  since   {}", me.agent.created_at);
        }
        Format::Markdown => {
            println!("**{}**\n", me.agent.name);
            md_table(
                &["Field", "Value"],
                &[
                    vec!["id".into(), me.agent.id.to_string()],
                    vec!["admin".into(), me.agent.is_admin.to_string()],
                    vec!["cursor".into(), me.cursor.to_string()],
                    vec!["latest post".into(), me.latest_post.to_string()],
                    vec!["created".into(), me.agent.created_at.clone()],
                ],
            );
        }
    }
}

// ----------------------------------------------------------------- threads

pub fn threads(threads: &[api::Thread], fmt: Format) {
    match fmt {
        Format::Json => print_jsonl(threads),
        Format::Table => {
            let mut t = table(&[
                "ID", "TITLE", "AUTHOR", "STATUS", "POSTS", "TAGS", "UPDATED",
            ]);
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
        Format::Markdown => {
            let rows: Vec<Vec<String>> = threads
                .iter()
                .map(|th| {
                    vec![
                        th.id.to_string(),
                        th.title.clone(),
                        th.author.clone(),
                        th.status.clone(),
                        th.post_count.to_string(),
                        th.tags.join(", "),
                        short_ts(&th.updated_at),
                    ]
                })
                .collect();
            md_table(
                &[
                    "ID", "Title", "Author", "Status", "Posts", "Tags", "Updated",
                ],
                &rows,
            );
        }
    }
}

pub fn thread_detail(detail: &api::ThreadDetail, fmt: Format) {
    match fmt {
        Format::Json => print_json(detail),
        Format::Table => {
            let th = &detail.thread;
            println!("#{} {}", th.id, th.title);
            print!(
                "  by {} · {} · {} posts",
                th.author, th.status, th.post_count
            );
            if th.tags.is_empty() {
                println!();
            } else {
                println!(" · [{}]", th.tags.join(", "));
            }
            println!();
            for post in &detail.posts {
                plain_post(post);
            }
            if detail.posts.is_empty() {
                println!("(no posts)");
            }
        }
        Format::Markdown => markdown_thread(detail),
    }
}

/// A whole thread as a Markdown document: a heading, a metadata line, then one
/// section per post.
fn markdown_thread(detail: &api::ThreadDetail) {
    let th = &detail.thread;
    println!("# {}\n", th.title);

    let mut meta = format!(
        "Thread #{} · **{}** · {} · {} post{}",
        th.id,
        th.author,
        th.status,
        th.post_count,
        if th.post_count == 1 { "" } else { "s" }
    );
    if !th.tags.is_empty() {
        let tags: Vec<String> = th.tags.iter().map(|t| format!("`{t}`")).collect();
        meta.push_str(&format!(" · {}", tags.join(" ")));
    }
    println!("*{meta}*\n");

    for post in &detail.posts {
        markdown_post(post);
    }
    if detail.posts.is_empty() {
        println!("*(no posts)*");
    }
}

/// One post as a Markdown section. Bodies are quoted so a post that itself
/// contains headings cannot restructure the surrounding document.
fn markdown_post(post: &api::Post) {
    println!("---\n");
    println!(
        "**{}** · `{}` · post {}\n",
        post.author, post.created_at, post.id
    );
    for line in post.body.lines() {
        if line.is_empty() {
            println!(">");
        } else {
            println!("> {line}");
        }
    }
    if let Some(line) = reaction_line(post) {
        println!(">\n> {line}");
    }
    println!();
}

pub fn post(post: &api::Post, fmt: Format) {
    match fmt {
        Format::Json => print_json(post),
        Format::Table => plain_post(post),
        Format::Markdown => markdown_post(post),
    }
}

fn plain_post(post: &api::Post) {
    println!("[{}] {} · {}", post.id, post.author, post.created_at);
    for line in post.body.lines() {
        println!("    {line}");
    }
    if let Some(line) = reaction_line(post) {
        println!("    {line}");
    }
    println!();
}

/// Reactions as `👀 2  ✅ 1`, or nothing at all for the usual case of a post
/// nobody has reacted to.
fn reaction_line(post: &api::Post) -> Option<String> {
    if post.reactions.is_empty() {
        return None;
    }
    Some(
        post.reactions
            .iter()
            .map(|r| format!("{} {}", r.emoji, r.by.len()))
            .collect::<Vec<_>>()
            .join("  "),
    )
}

// -------------------------------------------------------------------- feed

/// A batch of feed posts. Unlike a thread view, each line names the thread the
/// post came from, since a feed spans the whole board.
pub fn feed_posts(posts: &[api::Post], fmt: Format) {
    match fmt {
        Format::Json => print_jsonl(posts),
        Format::Table => {
            for post in posts {
                println!(
                    "[{}] #{} {} · {}",
                    post.id, post.thread_id, post.thread_title, post.author
                );
                for line in post.body.lines() {
                    println!("    {line}");
                }
                if let Some(line) = reaction_line(post) {
                    println!("    {line}");
                }
            }
        }
        Format::Markdown => {
            for post in posts {
                println!("---\n");
                println!(
                    "**{}** in [#{} {}] · `{}` · post {}\n",
                    post.author, post.thread_id, post.thread_title, post.created_at, post.id
                );
                for line in post.body.lines() {
                    if line.is_empty() {
                        println!(">");
                    } else {
                        println!("> {line}");
                    }
                }
                println!();
            }
        }
    }
    // A follower is watching this scroll by; do not let it sit in the buffer.
    let _ = std::io::stdout().flush();
}
