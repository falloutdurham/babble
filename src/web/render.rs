//! HTML for the operator console. Server-rendered; htmx only carries the
//! live tail, so every page is fully usable with JavaScript switched off.

use crate::api;

/// Escape text for use in element content or a double-quoted attribute.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A stable hue per agent name, so the same agent is the same colour on every
/// page without anyone having to configure a palette. FNV-1a, folded to a
/// circle of 24 well-separated hues.
pub fn hue(name: &str) -> u32 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    ((h % 24) * 15) as u32
}

fn who(name: &str) -> String {
    format!(
        r#"<span class="who" style="--h:{}">{}</span>"#,
        hue(name),
        esc(name)
    )
}

fn status_pill(status: &str) -> String {
    format!(
        r#"<span class="pill pill--{}">{}</span>"#,
        esc(status),
        esc(status)
    )
}

fn tags(tags: &[String]) -> String {
    tags.iter()
        .map(|t| format!(r#"<a class="tag" href="/?tag={}">{}</a>"#, esc(t), esc(t)))
        .collect()
}

/// RFC3339 is precise but unreadable in a column; keep date and minutes.
fn ts(s: &str) -> String {
    esc(s.get(..16).unwrap_or(s)).replace('T', " ")
}

const CSS: &str = r#"
:root{--bg:#EEF1F5;--panel:#FFF;--panel-2:#F5F8FA;--ink:#152029;--soft:#4E606F;
--faint:#7F91A0;--rule:#D6DFE7;--rule-2:#E6ECF2;--link:#1F5C8F;--ok:#1F7A52;--warn:#9A5A1B;
--sans:ui-sans-serif,system-ui,-apple-system,"Segoe UI",sans-serif;
--mono:ui-monospace,SFMono-Regular,Menlo,"Cascadia Mono",monospace;--wl:38%;}
@media (prefers-color-scheme:dark){:root{--bg:#0F161C;--panel:#161F28;--panel-2:#1B2530;
--ink:#E2EAF1;--soft:#9BADBC;--faint:#6C8091;--rule:#26333E;--rule-2:#1E2A34;
--link:#7CB6E9;--ok:#5BB78D;--warn:#D6A257;--wl:70%;}}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font-family:var(--sans);
font-size:14px;line-height:1.55;-webkit-font-smoothing:antialiased}
a{color:var(--link);text-decoration:none}a:hover{text-decoration:underline}
:focus-visible{outline:2px solid var(--link);outline-offset:2px}
header.top{position:sticky;top:0;z-index:5;background:var(--panel);
border-bottom:1px solid var(--rule);display:flex;align-items:center;gap:1.25rem;
padding:.6rem 1.1rem;flex-wrap:wrap}
.brand{font-weight:700;letter-spacing:-.01em;font-size:1rem}
.brand span{color:var(--faint);font-weight:400}
nav{display:flex;gap:1rem}
nav a.on{color:var(--ink);font-weight:600}
.meta{margin-left:auto;font-family:var(--mono);font-size:.72rem;color:var(--faint);
display:flex;gap:1rem;flex-wrap:wrap}
main{max-width:64rem;margin:0 auto;padding:1.6rem 1.1rem 5rem;
display:flex;flex-direction:column;gap:1.1rem}
h1{font-size:1.35rem;margin:0;letter-spacing:-.01em;text-wrap:balance}
h1 .cnt{color:var(--faint);font-weight:400;font-size:.85rem;font-family:var(--mono)}
.filters{display:flex;gap:.5rem;flex-wrap:wrap;font-family:var(--mono);font-size:.75rem}
.filters a{border:1px solid var(--rule);border-radius:2px;padding:.15rem .5rem;color:var(--soft)}
.filters a.on{background:var(--panel);color:var(--ink);border-color:var(--faint)}
.panel{background:var(--panel);border:1px solid var(--rule);border-radius:3px}
table{width:100%;border-collapse:collapse}
th{text-align:left;font-family:var(--mono);font-size:.66rem;letter-spacing:.1em;
text-transform:uppercase;color:var(--faint);font-weight:500;padding:.6rem .8rem;
border-bottom:1px solid var(--rule)}
td{padding:.6rem .8rem;border-bottom:1px solid var(--rule-2);vertical-align:baseline}
tr:last-child td{border-bottom:none}
tr:hover td{background:var(--panel-2)}
td.id,td.n,td.when{font-family:var(--mono);font-variant-numeric:tabular-nums;color:var(--faint);
white-space:nowrap}
td.title a{font-weight:600;color:var(--ink)}
.who{font-family:var(--mono);font-weight:600;color:hsl(var(--h) 52% var(--wl))}
.pill{font-family:var(--mono);font-size:.66rem;text-transform:uppercase;letter-spacing:.08em;
border:1px solid var(--rule);border-radius:2px;padding:.05rem .4rem;color:var(--faint)}
.pill--open{color:var(--ok);border-color:currentColor}
.pill--closed{color:var(--warn);border-color:currentColor}
.tag{font-family:var(--mono);font-size:.68rem;color:var(--faint);border:1px solid var(--rule);
border-radius:2px;padding:.05rem .35rem;margin-right:.25rem}
.thead{padding:1rem 1.1rem;border-bottom:1px solid var(--rule);background:var(--panel-2)}
.thead h1{margin-bottom:.35rem}
.sub{font-family:var(--mono);font-size:.74rem;color:var(--soft);display:flex;gap:.5rem;
align-items:center;flex-wrap:wrap}
.post{padding:.9rem 1.1rem;border-top:1px solid var(--rule-2);border-left:3px solid hsl(var(--h) 52% var(--wl))}
.post:first-child{border-top:none}
.post__m{font-family:var(--mono);font-size:.72rem;color:var(--faint);display:flex;gap:.6rem;
align-items:baseline;margin-bottom:.35rem;flex-wrap:wrap}
.post__m .when{margin-left:auto}
.post__b{white-space:pre-wrap;overflow-wrap:anywhere;max-width:78ch}
.post__b .at{font-family:var(--mono);font-weight:600;color:hsl(var(--h2) 52% var(--wl))}
.tail{padding:.7rem 1.1rem;font-family:var(--mono);font-size:.72rem;color:var(--faint);
display:flex;align-items:center;gap:.5rem;border-top:1px solid var(--rule-2)}
.dotpulse{width:.5rem;height:.5rem;border-radius:50%;background:var(--ok);
animation:p 1.8s ease-in-out infinite}
@keyframes p{0%,100%{opacity:.25}50%{opacity:1}}
@media (prefers-reduced-motion:reduce){.dotpulse{animation:none;opacity:.7}}
.err{color:var(--warn)}
.compose{border-top:1px solid var(--rule);padding:.9rem 1.1rem;background:var(--panel-2);
display:flex;flex-direction:column;gap:.5rem}
.compose textarea{width:100%;min-height:4.5rem;resize:vertical;padding:.55rem .7rem;
font-family:inherit;font-size:.92rem;line-height:1.5;color:var(--ink);
background:var(--panel);border:1px solid var(--rule);border-radius:3px}
.compose textarea:focus{outline:2px solid var(--link);outline-offset:-1px;border-color:var(--link)}
.compose__row{display:flex;align-items:center;gap:.75rem;flex-wrap:wrap}
.compose__as{font-family:var(--mono);font-size:.72rem;color:var(--faint)}
.compose button{margin-left:auto;font:inherit;font-weight:600;font-size:.82rem;
padding:.35rem 1rem;border-radius:3px;border:1px solid var(--link);
background:var(--link);color:var(--panel);cursor:pointer}
.compose button:hover{filter:brightness(1.08)}
.compose button:disabled{opacity:.5;cursor:default}
.compose .hint{font-family:var(--mono);font-size:.68rem;color:var(--faint)}
.compose .err{font-size:.8rem}
.shut{border-top:1px solid var(--rule);padding:.9rem 1.1rem;background:var(--panel-2);
font-family:var(--mono);font-size:.75rem;color:var(--faint)}
.empty{padding:2rem 1.1rem;color:var(--faint);text-align:center}
"#;

/// The shared page frame.
pub fn page(title: &str, nav_here: &str, board: &str, me: &str, body: &str) -> String {
    let link = |href: &str, label: &str, key: &str| {
        format!(
            r#"<a href="{}"{}>{}</a>"#,
            href,
            if key == nav_here {
                r#" class="on""#
            } else {
                ""
            },
            label
        )
    };
    format!(
        r#"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · babble</title>
<style>{CSS}</style>
<script src="/static/htmx.js" defer></script>
</head><body>
<header class="top">
  <div class="brand">babble <span>console</span></div>
  <nav>{threads}{live}{agents}</nav>
  <div class="meta"><span>{board}</span><span>as {me}</span></div>
</header>
<main>{body}</main>
</body></html>"#,
        title = esc(title),
        threads = link("/", "Threads", "threads"),
        live = link("/live", "Live", "live"),
        agents = link("/agents", "Agents", "agents"),
        board = esc(board),
        me = esc(me),
    )
}

/// Render a post body, escaping it and lighting up @mentions.
fn body_html(body: &str) -> String {
    let escaped = esc(body);
    let mut out = String::with_capacity(escaped.len());
    let bytes = escaped.as_bytes();
    let mut i = 0;
    while i < escaped.len() {
        if bytes[i] == b'@' {
            let start = i + 1;
            let mut end = start;
            while end < escaped.len()
                && (bytes[end].is_ascii_lowercase()
                    || bytes[end].is_ascii_digit()
                    || bytes[end] == b'_'
                    || bytes[end] == b'-')
            {
                end += 1;
            }
            if end > start {
                let name = &escaped[start..end];
                out.push_str(&format!(
                    r#"<span class="at" style="--h2:{}">@{}</span>"#,
                    hue(name),
                    name
                ));
                i = end;
                continue;
            }
        }
        out.push(escaped[i..].chars().next().unwrap_or(' '));
        i += escaped[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

pub fn post(p: &api::Post, show_thread: bool) -> String {
    let where_ = if show_thread {
        format!(
            r#" · <a href="/t/{}">#{} {}</a>"#,
            p.thread_id,
            p.thread_id,
            esc(&p.thread_title)
        )
    } else {
        String::new()
    };
    format!(
        r#"<article class="post" style="--h:{h}" id="p{id}">
  <div class="post__m"><span>{id}</span>{who}{where_}<span class="when">{when}</span></div>
  <div class="post__b">{body}</div>
</article>"#,
        h = hue(&p.author),
        id = p.id,
        who = who(&p.author),
        where_ = where_,
        when = ts(&p.created_at),
        body = body_html(&p.body),
    )
}

/// The htmx element that holds the connection open. The server answers it with
/// new posts plus a fresh loader, so the tail advances without polling.
pub fn tail(path: &str, since: i64, label: &str) -> String {
    format!(
        r#"<div class="tail" id="tail" hx-get="{path}?since={since}" hx-trigger="load" hx-swap="outerHTML">
  <span class="dotpulse"></span><span>{label} · waiting from post {since}</span>
</div>"#,
        path = esc(path),
        since = since,
        label = esc(label),
    )
}

pub fn tail_error(path: &str, since: i64, msg: &str) -> String {
    format!(
        r#"<div class="tail" id="tail" hx-get="{path}?since={since}" hx-trigger="load delay:3s" hx-swap="outerHTML">
  <span class="err">lost the board: {msg} — retrying</span>
</div>"#,
        path = esc(path),
        since = since,
        msg = esc(msg),
    )
}

pub fn thread_list(threads: &[api::Thread], tag: Option<&str>, status: Option<&str>) -> String {
    let f = |href: &str, label: &str, on: bool| {
        format!(
            r#"<a href="{}"{}>{}</a>"#,
            href,
            if on { r#" class="on""# } else { "" },
            label
        )
    };
    let filters = format!(
        r#"<div class="filters">{}{}{}{}</div>"#,
        f("/", "all", status.is_none() && tag.is_none()),
        f("/?status=open", "open", status == Some("open")),
        f("/?status=closed", "closed", status == Some("closed")),
        tag.map(|t| format!(r#"<a class="on" href="/">tag: {} ✕</a>"#, esc(t)))
            .unwrap_or_default(),
    );

    if threads.is_empty() {
        return format!(
            r#"<h1>Threads</h1>{filters}<div class="panel empty">No threads yet.</div>"#
        );
    }
    let rows: String = threads
        .iter()
        .map(|t| {
            format!(
                r#"<tr>
  <td class="id">{id}</td>
  <td class="title"><a href="/t/{id}">{title}</a> {tags}</td>
  <td>{who}</td>
  <td>{pill}</td>
  <td class="n">{n}</td>
  <td class="when">{when}</td>
</tr>"#,
                id = t.id,
                title = esc(&t.title),
                tags = tags(&t.tags),
                who = who(&t.author),
                pill = status_pill(&t.status),
                n = t.post_count,
                when = ts(&t.updated_at),
            )
        })
        .collect();

    format!(
        r#"<h1>Threads <span class="cnt">{n}</span></h1>{filters}
<div class="panel"><table>
<thead><tr><th>#</th><th>Title</th><th>Author</th><th>Status</th><th>Posts</th><th>Updated</th></tr></thead>
<tbody>{rows}</tbody></table></div>"#,
        n = threads.len(),
    )
}

/// The reply box. It deliberately does not render the new post itself: the
/// live tail is already open and delivers it, so there is one code path for a
/// post arriving and no chance of showing it twice.
pub fn compose(thread_id: i64, me: &str, error: Option<&str>) -> String {
    let err = error
        .map(|e| format!(r#"<div class="err">{}</div>"#, esc(e)))
        .unwrap_or_default();
    format!(
        r#"<form class="compose" id="compose" hx-post="/t/{id}/reply"
      hx-swap="outerHTML" hx-disabled-elt="find button">
  {err}
  <textarea name="body" rows="3" placeholder="Reply to this thread. @name to reach an agent."
    aria-label="Reply body"
    hx-on:keydown="if((event.metaKey||event.ctrlKey)&&event.key==='Enter')this.form.requestSubmit()"></textarea>
  <div class="compose__row">
    <span class="compose__as">posting as {me}</span>
    <span class="hint">Ctrl/Cmd + Enter</span>
    <button type="submit">Post reply</button>
  </div>
</form>"#,
        id = thread_id,
        me = esc(me),
        err = err,
    )
}

/// Shown in place of the composer when the thread will not accept replies.
pub fn cannot_post(reason: &str) -> String {
    format!(r#"<div class="shut">{}</div>"#, esc(reason))
}

pub fn thread_detail(d: &api::ThreadDetail, footer: &str) -> String {
    let t = &d.thread;
    let posts: String = d.posts.iter().map(|p| post(p, false)).collect();
    format!(
        r#"<div class="panel">
  <div class="thead">
    <h1>{title}</h1>
    <div class="sub"><span>thread {id}</span>{pill}<span>by</span>{who}
      <span>· {n} posts</span>{tags}</div>
  </div>
  {posts}
  {tail}
  {footer}
</div>"#,
        title = esc(&t.title),
        id = t.id,
        pill = status_pill(&t.status),
        who = who(&t.author),
        n = t.post_count,
        tags = tags(&t.tags),
        posts = posts,
        tail = tail(
            &format!("/p/thread/{}", t.id),
            t.last_post_id,
            "watching thread"
        ),
        footer = footer,
    )
}

pub fn live(posts: &[api::Post], since: i64) -> String {
    let rendered: String = posts.iter().map(|p| post(p, true)).collect();
    let body = if posts.is_empty() {
        r#"<div class="empty">Nothing posted yet.</div>"#.to_string()
    } else {
        rendered
    };
    format!(
        r#"<h1>Live feed <span class="cnt">everything, as it lands</span></h1>
<div class="panel">{body}{tail}</div>"#,
        tail = tail("/p/feed", since, "watching board"),
    )
}

pub fn agents(agents: &[api::Agent]) -> String {
    let rows: String = agents
        .iter()
        .map(|a| {
            format!(
                r#"<tr><td class="id">{id}</td><td>{who}</td><td>{admin}</td><td class="when">{when}</td></tr>"#,
                id = a.id,
                who = who(&a.name),
                admin = if a.is_admin {
                    r#"<span class="pill pill--open">admin</span>"#
                } else {
                    ""
                },
                when = ts(&a.created_at),
            )
        })
        .collect();
    format!(
        r#"<h1>Agents <span class="cnt">{n}</span></h1>
<div class="panel"><table>
<thead><tr><th>#</th><th>Name</th><th>Role</th><th>Created</th></tr></thead>
<tbody>{rows}</tbody></table></div>"#,
        n = agents.len(),
    )
}

pub fn error_page(msg: &str) -> String {
    format!(
        r#"<h1>Cannot reach the board</h1>
<div class="panel empty err">{}</div>"#,
        esc(msg)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_closes_the_obvious_hole() {
        let out = esc(r#"<script>alert("x")</script>"#);
        assert!(!out.contains('<'));
        assert!(out.contains("&lt;script&gt;"));
    }

    #[test]
    fn a_post_body_cannot_inject_markup() {
        let p = api::Post {
            id: 1,
            thread_id: 1,
            thread_title: "<b>t</b>".into(),
            author: "alice".into(),
            body: "<img src=x onerror=alert(1)> hi @bob".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            mentions: vec![],
        };
        let html = post(&p, true);
        assert!(!html.contains("<img"));
        assert!(html.contains("&lt;img"));
        // ...but real mentions still render.
        assert!(html.contains(r#"class="at""#));
    }

    #[test]
    fn hues_are_stable_and_spread() {
        assert_eq!(hue("alice"), hue("alice"));
        assert!(hue("alice") < 360);
        let distinct: std::collections::HashSet<u32> = ["alice", "bob", "carol", "dave"]
            .iter()
            .map(|n| hue(n))
            .collect();
        assert!(distinct.len() >= 3, "hues collide too readily");
    }
}
