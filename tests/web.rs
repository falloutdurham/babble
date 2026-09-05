//! Tests for the operator console. A real board and a real console are started
//! in-process, and the console is driven over HTTP like a browser would.

use babble::cli::ServeArgs;
use babble::client::config::Resolved;
use babble::client::{Client, ShowRequest};
use babble::{api, server, web};
use std::time::{Duration, Instant};

const ADMIN: &str = "test-admin-token";

struct Harness {
    board: String,
    console: String,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn start() -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let args = ServeArgs {
            db: dir.path().join("b.sqlite").to_string_lossy().into_owned(),
            bind: "127.0.0.1:0".into(),
            admin_token: Some(ADMIN.into()),
            post_rate: 0,
        };
        let (listener, state) = server::bind(&args).await.expect("bind board");
        let board = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            let _ = server::serve(listener, state, std::future::pending()).await;
        });

        let console_client = Client::new(Resolved {
            url: board.clone(),
            token: ADMIN.into(),
        })
        .expect("console client");
        let state = web::Console::new(console_client, board.clone(), "admin".into()).wait_secs(2);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind console");
        let console = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, web::router(state)).await;
        });

        Harness {
            board,
            console,
            _dir: dir,
        }
    }

    fn client(&self, token: &str) -> Client {
        Client::new(Resolved {
            url: self.board.clone(),
            token: token.to_string(),
        })
        .expect("client")
    }

    async fn agent(&self, name: &str) -> Client {
        let created = self
            .client(ADMIN)
            .create_agent(name, false)
            .await
            .expect("agent");
        self.client(&created.token)
    }

    async fn get(&self, path: &str) -> (reqwest::StatusCode, String) {
        let r = reqwest::get(format!("{}{}", self.console, path))
            .await
            .expect("request");
        (r.status(), r.text().await.expect("body"))
    }
}

fn new_thread(title: &str, body: &str, tags: &[&str]) -> api::NewThread {
    api::NewThread {
        title: title.into(),
        body: body.into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
    }
}

#[tokio::test]
async fn the_console_lists_threads_and_links_to_them() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("Deploy plan", "rolling out", &["ops"]))
        .await
        .unwrap();

    let (status, html) = h.get("/").await;
    assert!(status.is_success());
    assert!(html.contains("Deploy plan"));
    assert!(html.contains(r#"href="/t/1""#));
    assert!(html.contains("alice"));
    assert!(html.contains("ops"));
}

#[tokio::test]
async fn filters_narrow_the_thread_list() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let open = alice
        .create_thread(&new_thread("still open", "b", &["ops"]))
        .await
        .unwrap();
    alice
        .create_thread(&new_thread("other tag", "b", &["docs"]))
        .await
        .unwrap();
    let shut = alice
        .create_thread(&new_thread("all done", "b", &["ops"]))
        .await
        .unwrap();
    alice.set_thread_status(shut.thread.id, true).await.unwrap();

    let (_, by_tag) = h.get("/?tag=ops").await;
    assert!(by_tag.contains("still open"));
    assert!(!by_tag.contains("other tag"));

    let (_, by_status) = h.get("/?status=open").await;
    assert!(by_status.contains("still open"));
    assert!(!by_status.contains("all done"));
    let _ = open;
}

#[tokio::test]
async fn a_thread_page_shows_every_post_and_a_live_tail() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    let t = alice
        .create_thread(&new_thread("chat", "first post", &[]))
        .await
        .unwrap();
    bob.reply(t.thread.id, "second post").await.unwrap();

    let (status, html) = h.get("/t/1").await;
    assert!(status.is_success());
    assert!(html.contains("first post"));
    assert!(html.contains("second post"));
    // The tail resumes from the newest post, not from zero.
    assert!(html.contains(r#"hx-get="/p/thread/1?since=2""#), "{html}");
}

#[tokio::test]
async fn the_live_page_shows_posts_from_every_thread() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("one", "in thread one", &[]))
        .await
        .unwrap();
    alice
        .create_thread(&new_thread("two", "in thread two", &[]))
        .await
        .unwrap();

    let (status, html) = h.get("/live").await;
    assert!(status.is_success());
    assert!(html.contains("in thread one"));
    assert!(html.contains("in thread two"));
    assert!(html.contains(r#"hx-get="/p/feed?since=2""#), "{html}");
}

#[tokio::test]
async fn the_console_shows_the_operators_own_posts() {
    // An agent's feed hides its own posts; a console is a record and must not.
    let h = Harness::start().await;
    let admin = h.client(ADMIN);
    admin
        .create_thread(&new_thread("by the console's own identity", "mine", &[]))
        .await
        .unwrap();

    let (_, html) = h.get("/live").await;
    assert!(html.contains("mine"), "console hid the operator's own post");
}

#[tokio::test]
async fn the_tail_returns_the_moment_a_post_lands() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("t", "first", &[]))
        .await
        .unwrap();
    let thread_id = t.thread.id;

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        alice.reply(thread_id, "it arrived").await.expect("reply");
    });

    let started = Instant::now();
    let (status, html) = h.get("/p/thread/1?since=1").await;
    assert!(status.is_success());
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "tail did not wake"
    );
    assert!(html.contains("it arrived"));
    // ...and hands back a tail pointing past what it just delivered.
    assert!(html.contains(r#"hx-get="/p/thread/1?since=2""#), "{html}");
}

#[tokio::test]
async fn the_tail_holds_its_position_when_nothing_arrives() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("quiet", "nothing more", &[]))
        .await
        .unwrap();

    // The board caps `wait`, so this returns on its own; assert it does not
    // silently rewind the cursor when it times out.
    let (status, html) = h.get("/p/feed?since=1").await;
    assert!(status.is_success());
    assert!(html.contains(r#"hx-get="/p/feed?since=1""#), "{html}");
}

#[tokio::test]
async fn post_bodies_cannot_inject_markup_into_the_console() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread(
            "<script>alert('title')</script>",
            "<img src=x onerror=alert('body')> hello @bob",
            &[],
        ))
        .await
        .unwrap();

    for path in ["/", "/t/1", "/live"] {
        let (_, html) = h.get(path).await;
        assert!(!html.contains("<script>alert"), "unescaped title on {path}");
        assert!(!html.contains("<img src=x"), "unescaped body on {path}");
        assert!(html.contains("&lt;"), "nothing was escaped on {path}");
    }
}

#[tokio::test]
async fn agents_are_listed_without_tokens() {
    let h = Harness::start().await;
    h.agent("alice").await;

    let (status, html) = h.get("/agents").await;
    assert!(status.is_success());
    assert!(html.contains("alice"));
    assert!(html.contains("admin"));
    assert!(!html.contains("token"), "console leaked a token field");
}

#[tokio::test]
async fn htmx_is_served_locally_not_from_a_cdn() {
    let h = Harness::start().await;
    let (status, js) = h.get("/static/htmx.js").await;
    assert!(status.is_success());
    assert!(js.contains("htmx"));

    let (_, html) = h.get("/").await;
    assert!(html.contains(r#"src="/static/htmx.js""#));
    assert!(
        !html.contains("cdnjs"),
        "console pulls htmx from the network"
    );
}

#[tokio::test]
async fn every_page_is_readable_without_javascript() {
    // htmx only drives the tail; navigation is plain links.
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("plain", "body text", &[]))
        .await
        .unwrap();

    for path in ["/", "/t/1", "/live", "/agents"] {
        let (status, html) = h.get(path).await;
        assert!(status.is_success(), "{path} failed");
        assert!(html.starts_with("<!doctype html>"), "{path} is not a page");
        assert!(html.contains("</html>"), "{path} was truncated");
    }
}

#[tokio::test]
async fn a_dead_board_degrades_instead_of_five_hundreding() {
    let dir = tempfile::tempdir().expect("temp dir");
    let _ = &dir;
    // Point a console at a port with nothing on it.
    let client = Client::new(Resolved {
        url: "http://127.0.0.1:1".into(),
        token: ADMIN.into(),
    })
    .expect("client");
    let state = web::Console::new(client, "http://127.0.0.1:1".into(), "admin".into()).wait_secs(2);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, web::router(state)).await;
    });

    let r = reqwest::get(format!("{base}/")).await.expect("request");
    assert_eq!(r.status(), reqwest::StatusCode::BAD_GATEWAY);
    let html = r.text().await.unwrap();
    assert!(html.contains("Cannot reach the board"));

    // The tail keeps retrying rather than dying.
    let r = reqwest::get(format!("{base}/p/feed?since=7"))
        .await
        .expect("request");
    let html = r.text().await.unwrap();
    assert!(html.contains("retrying"));
    assert!(html.contains("since=7"), "tail lost its position on error");
}

/// `application/x-www-form-urlencoded` by hand, so the shipped client does not
/// have to carry reqwest's `form` feature just for these tests.
fn urlencoded(pairs: &[(&str, &str)]) -> String {
    fn enc(s: &str) -> String {
        let mut out = String::new();
        for b in s.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(*b as char)
                }
                b' ' => out.push('+'),
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", enc(k), enc(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// POST a form to the console, optionally as htmx would.
async fn post_form(url: &str, htmx: bool, pairs: &[(&str, &str)]) -> (reqwest::StatusCode, String) {
    let mut req = reqwest::Client::new()
        .post(url)
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(urlencoded(pairs));
    if htmx {
        req = req.header("HX-Request", "true");
    }
    let r = req.send().await.expect("post");
    (r.status(), r.text().await.expect("body"))
}

// ------------------------------------------------------------- posting

/// A console that will accept writes, plus a peer agent to watch the board.
async fn writable() -> Harness {
    Harness::start().await
}

#[tokio::test]
async fn the_operator_can_reply_from_the_console() {
    let h = writable().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("ops", "anyone there?", &[]))
        .await
        .unwrap();

    let (status, html) = post_form(
        &format!("{}/t/1/reply", h.console),
        true,
        &[("body", "on it — @alice taking this")],
    )
    .await;
    assert!(status.is_success());

    // The board really has it, authored by the console's own agent.
    let detail = alice.show_thread(&ShowRequest::thread(1)).await.unwrap();
    assert_eq!(detail.posts.len(), 2);
    assert_eq!(detail.posts[1].author, "admin");
    assert_eq!(detail.posts[1].body, "on it — @alice taking this");
    assert_eq!(detail.posts[1].mentions, vec!["alice"]);

    // The response is an empty box, not the post: the live tail delivers that,
    // so the operator never sees their reply twice.
    assert!(html.contains(r#"class="compose""#));
    assert!(
        !html.contains("taking this"),
        "reply was echoed as well as tailed"
    );
}

#[tokio::test]
async fn a_reply_reaches_a_waiting_agent_immediately() {
    let h = writable().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("ops", "waiting", &[]))
        .await
        .unwrap();

    let waiter = tokio::spawn(async move {
        alice
            .feed(&babble::client::FeedRequest::since(1).wait(Some(10)))
            .await
            .expect("feed")
    });
    tokio::time::sleep(Duration::from_millis(150)).await;

    post_form(
        &format!("{}/t/1/reply", h.console),
        true,
        &[("body", "from the console")],
    )
    .await;

    let started = Instant::now();
    let feed = waiter.await.expect("task");
    assert_eq!(feed.posts.len(), 1);
    assert_eq!(feed.posts[0].body, "from the console");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn an_empty_reply_is_refused_without_touching_the_board() {
    let h = writable().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();

    let (_, html) = post_form(
        &format!("{}/t/1/reply", h.console),
        true,
        &[("body", "   \n  ")],
    )
    .await;
    assert!(html.contains("Write something first"));
    assert_eq!(
        alice
            .show_thread(&ShowRequest::thread(1))
            .await
            .unwrap()
            .posts
            .len(),
        1
    );
}

#[tokio::test]
async fn replying_to_a_closed_thread_says_so() {
    let h = writable().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    alice.set_thread_status(t.thread.id, true).await.unwrap();

    // The page offers no box...
    let (_, page) = h.get("/t/1").await;
    assert!(page.contains("This thread is closed"));
    assert!(!page.contains(r#"class="compose""#));

    // ...and a hand-rolled POST is refused with the board's own reason.
    let (_, html) = post_form(
        &format!("{}/t/1/reply", h.console),
        true,
        &[("body", "let me in")],
    )
    .await;
    assert!(html.contains("closed"), "{html}");
    assert_eq!(
        alice
            .show_thread(&ShowRequest::thread(1))
            .await
            .unwrap()
            .posts
            .len(),
        1
    );
}

#[tokio::test]
async fn a_cross_origin_form_post_cannot_write_to_the_board() {
    // A hostile page on the network can make a browser send a simple form POST,
    // but it cannot set HX-Request without a preflight the console never grants.
    let h = writable().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();

    let (status, _) = post_form(
        &format!("{}/t/1/reply", h.console),
        false,
        &[("body", "posted by a drive-by")],
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        alice
            .show_thread(&ShowRequest::thread(1))
            .await
            .unwrap()
            .posts
            .len(),
        1
    );
}

#[tokio::test]
async fn read_only_mode_removes_the_box_and_refuses_writes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let args = ServeArgs {
        db: dir.path().join("b.sqlite").to_string_lossy().into_owned(),
        bind: "127.0.0.1:0".into(),
        admin_token: Some(ADMIN.into()),
        post_rate: 0,
    };
    let (listener, state) = server::bind(&args).await.expect("bind");
    let board = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = server::serve(listener, state, std::future::pending()).await;
    });

    let api = Client::new(Resolved {
        url: board.clone(),
        token: ADMIN.into(),
    })
    .unwrap();
    api.create_thread(&new_thread("t", "b", &[])).await.unwrap();

    let console_state = web::Console::new(
        Client::new(Resolved {
            url: board.clone(),
            token: ADMIN.into(),
        })
        .unwrap(),
        board.clone(),
        "admin".into(),
    )
    .wait_secs(2)
    .read_only(true);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let console = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, web::router(console_state)).await;
    });

    let page = reqwest::get(format!("{console}/t/1"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("read-only"));
    assert!(!page.contains(r#"class="compose""#));

    let (status, _) = post_form(
        &format!("{console}/t/1/reply"),
        true,
        &[("body", "should not land")],
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        api.show_thread(&ShowRequest::thread(1))
            .await
            .unwrap()
            .posts
            .len(),
        1
    );
}

// ------------------------------------------------------------- reactions

async fn post_query(url: &str, htmx: bool) -> (reqwest::StatusCode, String) {
    let mut req = reqwest::Client::new().post(url);
    if htmx {
        req = req.header("HX-Request", "true");
    }
    let r = req.send().await.expect("post");
    (r.status(), r.text().await.expect("body"))
}

#[tokio::test]
async fn clicking_a_reaction_toggles_it() {
    let h = writable().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("t", "react to me", &[]))
        .await
        .unwrap();

    // The picker is offered, and nothing is on the post yet.
    let (_, page) = h.get("/t/1").await;
    assert!(page.contains(r#"id="rx-1""#));
    assert!(page.contains(r#"class="rx__add""#));

    let url = format!("{}/p/react/1?emoji=%F0%9F%91%80", h.console);
    let (status, bar) = post_query(&url, true).await;
    assert!(status.is_success());
    assert!(bar.contains("\u{1f440}"), "{bar}");
    assert!(bar.contains("mine"), "own reaction is not marked: {bar}");
    assert_eq!(
        alice
            .show_thread(&ShowRequest::thread(1))
            .await
            .unwrap()
            .posts[0]
            .reactions[0]
            .by,
        vec!["admin"]
    );

    // Clicking the same one again takes it off.
    let (_, bar) = post_query(&url, true).await;
    assert!(!bar.contains("mine"), "{bar}");
    assert!(
        alice
            .show_thread(&ShowRequest::thread(1))
            .await
            .unwrap()
            .posts[0]
            .reactions
            .is_empty()
    );
}

#[tokio::test]
async fn another_agents_reaction_shows_but_is_not_marked_mine() {
    let h = writable().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    alice.react(t.posts[0].id, "\u{1f680}").await.unwrap();

    let (_, page) = h.get("/t/1").await;
    assert!(page.contains("\u{1f680}"));
    assert!(page.contains("alice reacted"));
    assert!(!page.contains(r#"class="rx__b mine""#));
}

#[tokio::test]
async fn a_read_only_console_shows_counts_but_no_controls() {
    let dir = tempfile::tempdir().expect("temp dir");
    let args = ServeArgs {
        db: dir.path().join("b.sqlite").to_string_lossy().into_owned(),
        bind: "127.0.0.1:0".into(),
        admin_token: Some(ADMIN.into()),
        post_rate: 0,
    };
    let (listener, state) = server::bind(&args).await.expect("bind");
    let board = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = server::serve(listener, state, std::future::pending()).await;
    });
    let api = Client::new(Resolved {
        url: board.clone(),
        token: ADMIN.into(),
    })
    .unwrap();
    let t = api.create_thread(&new_thread("t", "b", &[])).await.unwrap();
    api.react(t.posts[0].id, "\u{2705}").await.unwrap();

    let console_state = web::Console::new(
        Client::new(Resolved {
            url: board.clone(),
            token: ADMIN.into(),
        })
        .unwrap(),
        board,
        "admin".into(),
    )
    .wait_secs(2)
    .read_only(true);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let console = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, web::router(console_state)).await;
    });

    let page = reqwest::get(format!("{console}/t/1"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("\u{2705}"), "count is missing");
    assert!(
        !page.contains(r#"class="rx__add""#),
        "picker offered on a read-only console"
    );
    assert!(
        !page.contains("hx-post=\"/p/react"),
        "reactions are clickable"
    );

    let (status, _) = post_query(&format!("{console}/p/react/1?emoji=%F0%9F%91%80"), true).await;
    assert_eq!(status, reqwest::StatusCode::FORBIDDEN);
    assert!(
        api.show_thread(&ShowRequest::thread(1))
            .await
            .unwrap()
            .posts[0]
            .reactions
            .iter()
            .all(|r| r.emoji != "\u{1f440}")
    );
}

#[tokio::test]
async fn a_cross_origin_click_cannot_react() {
    let h = writable().await;
    let alice = h.agent("alice").await;
    alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();

    let (status, _) = post_query(
        &format!("{}/p/react/1?emoji=%F0%9F%91%80", h.console),
        false,
    )
    .await;
    assert_eq!(status, reqwest::StatusCode::BAD_REQUEST);
    assert!(
        alice
            .show_thread(&ShowRequest::thread(1))
            .await
            .unwrap()
            .posts[0]
            .reactions
            .is_empty()
    );
}

#[tokio::test]
async fn the_console_shows_the_end_of_a_long_thread_and_offers_the_rest() {
    let h = writable().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("long", "post 0", &[]))
        .await
        .unwrap();
    for i in 1..60 {
        alice
            .reply(t.thread.id, &format!("post {i}"))
            .await
            .unwrap();
    }

    // Default view is the tail, and says so rather than silently dropping posts.
    let (status, page) = h.get("/t/1").await;
    assert!(status.is_success());
    assert!(page.contains("post 59"), "newest post missing");
    assert!(
        !page.contains(">post 0<"),
        "whole thread rendered by default"
    );
    assert!(
        page.contains("Showing the last 50 of 60 posts"),
        "no notice"
    );
    assert!(page.contains(r#"href="?all=1""#));

    // ...and the escape hatch really does return everything.
    let (_, whole) = h.get("/t/1?all=1").await;
    assert!(whole.contains("post 0"));
    assert!(whole.contains("post 59"));
    assert!(!whole.contains("Showing the last"));
}
