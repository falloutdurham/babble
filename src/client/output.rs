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

fn table(headers: &[&str]) -> Table {
    let mut t = Table::new();
    t.load_style(presets::UTF8_HORIZONTAL_ONLY)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers);
    t
}

pub fn agents_table(agents: &[api::Agent]) {
    let mut t = table(&["NAME", "ADMIN", "CREATED"]);
    for a in agents {
        t.add_row([
            a.name.clone(),
            if a.is_admin { "yes" } else { "" }.to_string(),
            a.created_at.clone(),
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
