//! Integration tests. Each one starts a real server on port 0 inside this
//! process and drives it through the same client the CLI uses — no external
//! processes, no fixed ports.

use babble::api;
use babble::cli::ServeArgs;
use babble::client::config::Resolved;
use babble::client::error::Kind;
use babble::client::{Client, FeedRequest};
use babble::server;
use std::time::{Duration, Instant};

/// The bootstrap admin token every test logs in with.
const ADMIN: &str = "test-admin-token";

struct Harness {
    url: String,
    /// Kept alive so the SQLite file outlives the test.
    _dir: tempfile::TempDir,
}

impl Harness {
    /// Start a server with post rate limiting disabled.
    async fn start() -> Harness {
        Harness::start_with_rate(0).await
    }

    async fn start_with_rate(post_rate: u32) -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let args = ServeArgs {
            db: dir
                .path()
                .join("babble.sqlite")
                .to_string_lossy()
                .into_owned(),
            bind: "127.0.0.1:0".into(),
            admin_token: Some(ADMIN.into()),
            post_rate,
        };
        let (listener, state) = server::bind(&args).await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            // Never shuts down on its own; the test process ends it.
            let _ = server::serve(listener, state, std::future::pending()).await;
        });
        Harness {
            url: format!("http://{addr}"),
            _dir: dir,
        }
    }

    fn client(&self, token: &str) -> Client {
        Client::new(Resolved {
            url: self.url.clone(),
            token: token.to_string(),
        })
        .expect("client")
    }

    fn admin(&self) -> Client {
        self.client(ADMIN)
    }

    /// Create an agent and return a client authenticated as it.
    async fn agent(&self, name: &str) -> Client {
        let created = self.admin().create_agent(name, false).await.expect("agent");
        self.client(&created.token)
    }
}

fn new_thread(title: &str, body: &str, tags: &[&str]) -> api::NewThread {
    api::NewThread {
        title: title.into(),
        body: body.into(),
        tags: tags.iter().map(|t| t.to_string()).collect(),
    }
}

// ------------------------------------------------------------------ health

#[tokio::test]
async fn health_needs_no_token() {
    let h = Harness::start().await;
    assert!(h.client("not-a-real-token").health().await.unwrap().ok);
}

#[tokio::test]
async fn unknown_token_is_rejected() {
    let h = Harness::start().await;
    let err = h.client("nope").whoami().await.unwrap_err();
    assert_eq!(err.kind, Kind::Auth);
    assert_eq!(err.kind.exit_code(), 2);
}

// ------------------------------------------------------------------ agents

#[tokio::test]
async fn admin_creates_agents_and_lists_them() {
    let h = Harness::start().await;
    let created = h.admin().create_agent("alice", false).await.unwrap();
    assert_eq!(created.name, "alice");
    assert!(!created.token.is_empty());
    assert!(!created.is_admin);

    let names: Vec<String> = h
        .admin()
        .list_agents()
        .await
        .unwrap()
        .agents
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, vec!["admin", "alice"]);
}

#[tokio::test]
async fn non_admins_cannot_create_agents() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let err = alice.create_agent("mallory", false).await.unwrap_err();
    assert_eq!(err.kind, Kind::Auth);
}

#[tokio::test]
async fn duplicate_and_malformed_agent_names_are_refused() {
    let h = Harness::start().await;
    h.admin().create_agent("alice", false).await.unwrap();

    let dup = h.admin().create_agent("alice", false).await.unwrap_err();
    assert_eq!(dup.kind, Kind::Config);
    assert!(dup.message.contains("already exists"));

    let bad = h.admin().create_agent("Alice!", false).await.unwrap_err();
    assert_eq!(bad.kind, Kind::Config);
}

#[tokio::test]
async fn tokens_are_never_returned_by_listings() {
    let h = Harness::start().await;
    h.admin().create_agent("alice", false).await.unwrap();
    let listed = serde_json::to_string(&h.admin().list_agents().await.unwrap()).unwrap();
    assert!(!listed.contains("token"), "listing leaked a token field");
}

#[tokio::test]
async fn whoami_reports_identity_cursor_and_high_water_mark() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    let me = alice.whoami().await.unwrap();
    assert_eq!(me.agent.name, "alice");
    assert_eq!(me.cursor, 0);
    assert_eq!(me.latest_post, 0);

    alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    assert_eq!(alice.whoami().await.unwrap().latest_post, 1);
}

// ----------------------------------------------------------------- cursors

#[tokio::test]
async fn cursors_advance_but_never_rewind() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    assert_eq!(alice.set_cursor(5).await.unwrap().cursor, 5);
    assert_eq!(alice.set_cursor(9).await.unwrap().cursor, 9);
    assert_eq!(alice.set_cursor(2).await.unwrap().cursor, 9);
    assert!(alice.set_cursor(-1).await.is_err());
}

// ----------------------------------------------------------------- threads

#[tokio::test]
async fn a_thread_carries_its_first_post() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    let detail = alice
        .create_thread(&new_thread("Deploy v2", "rolling out", &["ops", "ops"]))
        .await
        .unwrap();
    assert_eq!(detail.thread.title, "Deploy v2");
    assert_eq!(detail.thread.author, "alice");
    assert_eq!(detail.thread.status, "open");
    // Duplicate tags collapse.
    assert_eq!(detail.thread.tags, vec!["ops"]);
    assert_eq!(detail.thread.post_count, 1);
    assert_eq!(detail.posts.len(), 1);
    assert_eq!(detail.posts[0].body, "rolling out");
}

#[tokio::test]
async fn thread_input_is_bounded() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    // Tags are deduplicated before they are counted, so the over-long list has
    // to be genuinely distinct.
    let too_many: Vec<String> = (0..=api::MAX_TAGS).map(|i| format!("tag{i}")).collect();
    let too_long = vec!["t".repeat(api::MAX_TAG_LEN + 1)];

    for bad in [
        new_thread("", "body", &[]),
        new_thread(&"t".repeat(api::MAX_TITLE_LEN + 1), "body", &[]),
        new_thread("title", "", &[]),
        new_thread("title", &"b".repeat(api::MAX_BODY_LEN + 1), &[]),
        api::NewThread {
            title: "title".into(),
            body: "body".into(),
            tags: too_many,
        },
        api::NewThread {
            title: "title".into(),
            body: "body".into(),
            tags: too_long,
        },
    ] {
        let err = alice.create_thread(&bad).await.unwrap_err();
        assert_eq!(err.kind, Kind::Config, "expected a 400 for {:?}", bad.title);
    }
}

#[tokio::test]
async fn threads_list_filters_by_tag_and_status_newest_first() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    let a = alice
        .create_thread(&new_thread("first", "b", &["ops"]))
        .await
        .unwrap();
    let b = alice
        .create_thread(&new_thread("second", "b", &["docs"]))
        .await
        .unwrap();

    // Most recently updated first.
    let all = alice.list_threads(None, None, None, None).await.unwrap();
    assert_eq!(
        all.threads.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![b.thread.id, a.thread.id]
    );

    let ops = alice
        .list_threads(Some("ops"), None, None, None)
        .await
        .unwrap();
    assert_eq!(ops.threads.len(), 1);
    assert_eq!(ops.threads[0].title, "first");

    alice.set_thread_status(a.thread.id, true).await.unwrap();
    let open = alice
        .list_threads(None, Some("open"), None, None)
        .await
        .unwrap();
    assert_eq!(open.threads.len(), 1);
    assert_eq!(open.threads[0].id, b.thread.id);

    let closed = alice
        .list_threads(None, Some("closed"), None, None)
        .await
        .unwrap();
    assert_eq!(closed.threads.len(), 1);

    // limit and offset page through the same ordering.
    let page = alice
        .list_threads(None, None, Some(1), Some(1))
        .await
        .unwrap();
    assert_eq!(page.threads.len(), 1);
    assert_eq!(page.threads[0].id, a.thread.id);

    assert!(
        alice
            .list_threads(None, Some("weird"), None, None)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn showing_a_thread_supports_since_and_404s() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;

    let t = alice
        .create_thread(&new_thread("chat", "one", &[]))
        .await
        .unwrap();
    bob.reply(t.thread.id, "two").await.unwrap();

    let full = bob.show_thread(t.thread.id, None).await.unwrap();
    assert_eq!(full.posts.len(), 2);
    assert_eq!(full.thread.post_count, 2);

    let tail = bob.show_thread(t.thread.id, Some(1)).await.unwrap();
    assert_eq!(tail.posts.len(), 1);
    assert_eq!(tail.posts[0].body, "two");

    let err = bob.show_thread(9999, None).await.unwrap_err();
    assert_eq!(err.kind, Kind::NotFound);
    assert_eq!(err.kind.exit_code(), 3);
}

#[tokio::test]
async fn replying_bumps_the_thread() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;

    let older = alice
        .create_thread(&new_thread("older", "b", &[]))
        .await
        .unwrap();
    alice
        .create_thread(&new_thread("newer", "b", &[]))
        .await
        .unwrap();
    bob.reply(older.thread.id, "up you go").await.unwrap();

    let listed = alice.list_threads(None, None, None, None).await.unwrap();
    assert_eq!(listed.threads[0].title, "older");
    assert!(listed.threads[0].updated_at >= older.thread.updated_at);
}

#[tokio::test]
async fn only_the_author_or_an_admin_changes_thread_status() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    let t = alice
        .create_thread(&new_thread("mine", "b", &[]))
        .await
        .unwrap();

    let err = bob.set_thread_status(t.thread.id, true).await.unwrap_err();
    assert_eq!(err.kind, Kind::Auth);

    assert_eq!(
        alice
            .set_thread_status(t.thread.id, true)
            .await
            .unwrap()
            .status,
        "closed"
    );
    // Replies to a closed thread are refused.
    let err = bob.reply(t.thread.id, "hello?").await.unwrap_err();
    assert_eq!(err.kind, Kind::Config);
    assert!(err.message.contains("closed"));

    // An admin can reopen someone else's thread.
    assert_eq!(
        h.admin()
            .set_thread_status(t.thread.id, false)
            .await
            .unwrap()
            .status,
        "open"
    );
    bob.reply(t.thread.id, "hello again").await.unwrap();

    assert_eq!(
        bob.set_thread_status(9999, true).await.unwrap_err().kind,
        Kind::NotFound
    );
}

#[tokio::test]
async fn posting_to_a_missing_thread_is_a_404() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    assert_eq!(
        alice.reply(4242, "into the void").await.unwrap_err().kind,
        Kind::NotFound
    );
}

#[tokio::test]
async fn post_bodies_are_bounded() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();

    assert!(alice.reply(t.thread.id, "   ").await.is_err());
    assert!(
        alice
            .reply(t.thread.id, &"x".repeat(api::MAX_BODY_LEN + 1))
            .await
            .is_err()
    );
}

// ---------------------------------------------------------------- mentions

#[tokio::test]
async fn mentions_resolve_known_agents_and_ignore_the_rest() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    h.agent("bob").await;

    let t = alice
        .create_thread(&new_thread("hi", "ping @bob and @ghost", &[]))
        .await
        .unwrap();
    assert_eq!(t.posts[0].mentions, vec!["bob"]);
}

// -------------------------------------------------------------------- feed

#[tokio::test]
async fn the_feed_pages_forward_from_since() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    let t = alice
        .create_thread(&new_thread("t", "one", &[]))
        .await
        .unwrap();
    alice.reply(t.thread.id, "two").await.unwrap();
    alice.reply(t.thread.id, "three").await.unwrap();

    let first = bob
        .feed(&FeedRequest::since(0).limit(Some(2)))
        .await
        .unwrap();
    assert_eq!(first.posts.len(), 2);
    assert_eq!(first.next_since, 2);
    assert_eq!(first.posts[0].thread_title, "t");

    let rest = bob
        .feed(&FeedRequest::since(first.next_since).limit(Some(2)))
        .await
        .unwrap();
    assert_eq!(rest.posts.len(), 1);
    assert_eq!(rest.posts[0].body, "three");

    // Nothing left: next_since holds its position.
    let empty = bob
        .feed(&FeedRequest::since(rest.next_since))
        .await
        .unwrap();
    assert!(empty.posts.is_empty());
    assert_eq!(empty.next_since, rest.next_since);
}

#[tokio::test]
async fn mention_me_filters_the_feed() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;

    let t = alice
        .create_thread(&new_thread("t", "hello everyone", &[]))
        .await
        .unwrap();
    alice.reply(t.thread.id, "over to you @bob").await.unwrap();

    let mine = bob.feed(&FeedRequest::since(0).mention()).await.unwrap();
    assert_eq!(mine.posts.len(), 1);
    assert_eq!(mine.posts[0].body, "over to you @bob");

    assert!(
        alice
            .feed(&FeedRequest::since(0).mention())
            .await
            .unwrap()
            .posts
            .is_empty()
    );
}

#[tokio::test]
async fn a_long_poll_returns_as_soon_as_a_post_lands() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    let t = alice
        .create_thread(&new_thread("t", "first", &[]))
        .await
        .unwrap();
    let thread_id = t.thread.id;

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        alice.reply(thread_id, "woken up").await.expect("reply");
    });

    let started = Instant::now();
    let feed = bob
        .feed(&FeedRequest::since(1).wait(Some(10)))
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(feed.posts.len(), 1);
    assert_eq!(feed.posts[0].body, "woken up");
    assert!(elapsed < Duration::from_secs(1), "took {elapsed:?}");
}

#[tokio::test]
async fn a_long_poll_times_out_empty_and_successful() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    let started = Instant::now();
    let feed = alice
        .feed(&FeedRequest::since(0).wait(Some(1)))
        .await
        .unwrap();
    assert!(feed.posts.is_empty());
    assert_eq!(feed.next_since, 0);
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[tokio::test]
async fn a_waiting_poll_still_returns_existing_posts_immediately() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    alice
        .create_thread(&new_thread("t", "already here", &[]))
        .await
        .unwrap();

    let started = Instant::now();
    let feed = bob
        .feed(&FeedRequest::since(0).wait(Some(30)))
        .await
        .unwrap();
    assert_eq!(feed.posts.len(), 1);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[tokio::test]
async fn the_mention_filter_only_accepts_me() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    // The client only ever sends `me`, so this exercises the raw query.
    let resp = reqwest::Client::new()
        .get(format!("{}/posts?mention=bob", h.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert!(alice.feed(&FeedRequest::since(0).mention()).await.is_ok());
}

// ------------------------------------------------------------- rate limits

#[tokio::test]
async fn posting_faster_than_the_budget_is_refused() {
    let h = Harness::start_with_rate(2).await;
    let alice = h.agent("alice").await;

    // The thread itself spends one token, leaving exactly one reply.
    let t = alice
        .create_thread(&new_thread("t", "one", &[]))
        .await
        .unwrap();
    alice.reply(t.thread.id, "two").await.unwrap();

    let err = alice.reply(t.thread.id, "three").await.unwrap_err();
    assert_eq!(err.kind, Kind::Config);
    assert!(err.message.contains("rate limit"), "got: {}", err.message);

    // Budgets are per agent, so bob is unaffected.
    let bob = h.agent("bob").await;
    bob.reply(t.thread.id, "mine").await.unwrap();
}

// ------------------------------------------------------------- concurrency

#[tokio::test]
async fn ten_agents_posting_at_once_lose_nothing() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("stampede", "go", &[]))
        .await
        .unwrap();
    let thread_id = t.thread.id;

    const AGENTS: usize = 10;
    const EACH: usize = 10;

    let mut clients = Vec::new();
    for i in 0..AGENTS {
        clients.push(h.agent(&format!("agent-{i}")).await);
    }

    let mut tasks = Vec::new();
    for (i, client) in clients.into_iter().enumerate() {
        tasks.push(tokio::spawn(async move {
            for j in 0..EACH {
                client
                    .reply(thread_id, &format!("agent {i} post {j}"))
                    .await
                    .expect("reply");
            }
        }));
    }
    for task in tasks {
        task.await.expect("task");
    }

    let detail = alice.show_thread(thread_id, None).await.unwrap();
    // The opening post plus every concurrent reply.
    assert_eq!(detail.posts.len(), AGENTS * EACH + 1);
    assert_eq!(detail.thread.post_count as usize, AGENTS * EACH + 1);

    // Ids are strictly increasing and never reused.
    let ids: Vec<i64> = detail.posts.iter().map(|p| p.id).collect();
    assert!(
        ids.windows(2).all(|w| w[1] > w[0]),
        "post ids are not strictly increasing"
    );

    // Every message survived exactly once.
    let mut bodies: Vec<&str> = detail.posts.iter().map(|p| p.body.as_str()).collect();
    bodies.sort_unstable();
    bodies.dedup();
    assert_eq!(bodies.len(), AGENTS * EACH + 1);

    // And the feed sees the same set.
    let feed = alice
        .feed(
            &FeedRequest::since(0)
                .include_self(true)
                .limit(Some(api::MAX_LIMIT)),
        )
        .await
        .unwrap();
    assert_eq!(feed.posts.len(), AGENTS * EACH + 1);
    assert_eq!(feed.next_since, *ids.last().unwrap());
}

// ------------------------------------------------------------------- watch

#[tokio::test]
async fn a_thread_reports_its_newest_post_id() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    let t = alice
        .create_thread(&new_thread("t", "one", &[]))
        .await
        .unwrap();
    assert_eq!(t.thread.last_post_id, t.posts[0].id);

    let reply = alice.reply(t.thread.id, "two").await.unwrap();
    let listed = alice.list_threads(None, None, None, None).await.unwrap();
    assert_eq!(listed.threads[0].last_post_id, reply.id);
}

#[tokio::test]
async fn the_feed_can_be_scoped_to_one_thread() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;

    let a = alice
        .create_thread(&new_thread("a", "in a", &[]))
        .await
        .unwrap();
    let b = alice
        .create_thread(&new_thread("b", "in b", &[]))
        .await
        .unwrap();
    alice.reply(a.thread.id, "also in a").await.unwrap();

    let scoped = bob
        .feed(&FeedRequest::since(0).thread(a.thread.id))
        .await
        .unwrap();
    assert_eq!(scoped.posts.len(), 2);
    assert!(scoped.posts.iter().all(|p| p.thread_id == a.thread.id));

    let other = bob
        .feed(&FeedRequest::since(0).thread(b.thread.id))
        .await
        .unwrap();
    assert_eq!(other.posts.len(), 1);
    assert_eq!(other.posts[0].body, "in b");
}

#[tokio::test]
async fn watching_a_missing_thread_is_a_404_not_a_hang() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    let started = Instant::now();
    let err = alice
        .feed(&FeedRequest::since(0).thread(9999).wait(Some(30)))
        .await
        .unwrap_err();
    assert_eq!(err.kind, Kind::NotFound);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn a_scoped_long_poll_ignores_posts_in_other_threads() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;

    let watched = alice
        .create_thread(&new_thread("watched", "start", &[]))
        .await
        .unwrap();
    let other = alice
        .create_thread(&new_thread("other", "start", &[]))
        .await
        .unwrap();
    let watched_id = watched.thread.id;
    let other_id = other.thread.id;

    tokio::spawn(async move {
        // A post in the wrong thread wakes the notifier but must not satisfy
        // the query; the right one, later, must.
        tokio::time::sleep(Duration::from_millis(150)).await;
        alice.reply(other_id, "noise").await.expect("reply");
        tokio::time::sleep(Duration::from_millis(150)).await;
        alice.reply(watched_id, "signal").await.expect("reply");
    });

    let feed = bob
        .feed(
            &FeedRequest::since(watched.thread.last_post_id)
                .thread(watched_id)
                .wait(Some(10)),
        )
        .await
        .unwrap();
    assert_eq!(feed.posts.len(), 1);
    assert_eq!(feed.posts[0].body, "signal");
}

#[tokio::test]
async fn watching_does_not_move_the_global_cursor() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;

    let t = alice
        .create_thread(&new_thread("t", "one", &[]))
        .await
        .unwrap();
    alice.reply(t.thread.id, "two").await.unwrap();

    bob.feed(&FeedRequest::since(0).thread(t.thread.id))
        .await
        .unwrap();
    assert_eq!(bob.whoami().await.unwrap().cursor, 0);
}

#[tokio::test]
async fn the_feed_leaves_out_your_own_posts_unless_asked() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;

    let t = alice
        .create_thread(&new_thread("t", "mine", &[]))
        .await
        .unwrap();
    bob.reply(t.thread.id, "theirs").await.unwrap();

    // Alice sees only bob's reply, not her own opening post.
    let mine = alice.feed(&FeedRequest::since(0)).await.unwrap();
    assert_eq!(mine.posts.len(), 1);
    assert_eq!(mine.posts[0].author, "bob");

    // ...and can ask for the full record.
    let all = alice
        .feed(&FeedRequest::since(0).include_self(true))
        .await
        .unwrap();
    assert_eq!(all.posts.len(), 2);

    // The thread view is a record, not a feed: it always shows everything.
    let detail = alice.show_thread(t.thread.id, None).await.unwrap();
    assert_eq!(detail.posts.len(), 2);
}

#[tokio::test]
async fn your_own_post_does_not_satisfy_your_own_long_poll() {
    // The bug five survey agents hit: post, then `poll --wait` returns your own
    // message instantly instead of waiting for a peer.
    let h = Harness::start().await;
    let alice = h.agent("alice").await;

    alice
        .create_thread(&new_thread("t", "hello?", &[]))
        .await
        .unwrap();

    let started = Instant::now();
    let feed = alice
        .feed(&FeedRequest::since(0).wait(Some(1)))
        .await
        .unwrap();
    assert!(feed.posts.is_empty(), "own post satisfied own long-poll");
    assert!(started.elapsed() >= Duration::from_millis(900));
}

// --------------------------------------------------------------- reactions

#[tokio::test]
async fn reactions_accumulate_and_name_who_left_them() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    let t = alice
        .create_thread(&new_thread("t", "worth reacting to", &[]))
        .await
        .unwrap();
    let post_id = t.posts[0].id;

    let p = bob.react(post_id, "👀").await.unwrap();
    assert_eq!(p.reactions.len(), 1);
    assert_eq!(p.reactions[0].emoji, "👀");
    assert_eq!(p.reactions[0].by, vec!["bob"]);

    let p = alice.react(post_id, "👀").await.unwrap();
    assert_eq!(p.reactions[0].by, vec!["alice", "bob"]);

    // A second emoji sorts under the more-reacted one.
    let p = alice.react(post_id, "🚀").await.unwrap();
    assert_eq!(p.reactions.len(), 2);
    assert_eq!(p.reactions[0].emoji, "👀");
    assert_eq!(p.reactions[1].emoji, "🚀");
}

#[tokio::test]
async fn reacting_twice_the_same_way_is_a_no_op() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    let id = t.posts[0].id;

    alice.react(id, "✅").await.unwrap();
    // A retry after a dropped response must not fail or double-count.
    let p = alice.react(id, "✅").await.unwrap();
    assert_eq!(p.reactions.len(), 1);
    assert_eq!(p.reactions[0].by, vec!["alice"]);
}

#[tokio::test]
async fn a_reaction_can_be_taken_back() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    let id = t.posts[0].id;

    alice.react(id, "👍").await.unwrap();
    bob.react(id, "👍").await.unwrap();

    // Removing mine leaves theirs.
    let p = alice.unreact(id, "👍").await.unwrap();
    assert_eq!(p.reactions[0].by, vec!["bob"]);

    // The last one removed clears the emoji entirely.
    let p = bob.unreact(id, "👍").await.unwrap();
    assert!(p.reactions.is_empty());

    // Removing one that was never there is harmless.
    assert!(alice.unreact(id, "👍").await.is_ok());
}

#[tokio::test]
async fn reactions_show_up_wherever_posts_do() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    bob.react(t.posts[0].id, "🎉").await.unwrap();

    let detail = bob.show_thread(t.thread.id, None).await.unwrap();
    assert_eq!(detail.posts[0].reactions[0].emoji, "🎉");

    let feed = bob
        .feed(&FeedRequest::since(0).include_self(true))
        .await
        .unwrap();
    assert_eq!(feed.posts[0].reactions[0].emoji, "🎉");
}

#[tokio::test]
async fn a_reaction_has_to_be_an_emoji() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    let id = t.posts[0].id;

    for bad in ["lgtm", "", ":+1:", "👀 👍", "1"] {
        let err = alice.react(id, bad).await.unwrap_err();
        assert_eq!(err.kind, Kind::Config, "accepted {bad:?}");
    }
    // Multi-codepoint emoji are fine.
    for good in ["❤️", "👍🏽", "🏴󠁧󠁢󠁳󠁣󠁴󠁿"] {
        assert!(alice.unreact(id, good).await.is_ok());
    }
}

#[tokio::test]
async fn one_agent_cannot_paper_a_post_in_reactions() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    let id = t.posts[0].id;

    for e in ["👀", "✅", "👍", "👎", "🎉", "❤️", "🤔", "🚀"] {
        alice.react(id, e).await.unwrap();
    }
    let err = alice.react(id, "😀").await.unwrap_err();
    assert_eq!(err.kind, Kind::Config);

    // Another agent still has their own budget.
    let bob = h.agent("bob").await;
    assert!(bob.react(id, "😀").await.is_ok());
}

#[tokio::test]
async fn reacting_to_a_missing_post_is_a_404() {
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    assert_eq!(
        alice.react(9999, "👀").await.unwrap_err().kind,
        Kind::NotFound
    );
    assert_eq!(
        alice.unreact(9999, "👀").await.unwrap_err().kind,
        Kind::NotFound
    );
}

#[tokio::test]
async fn a_reaction_does_not_wake_a_long_poll() {
    // Reactions are not posts. Waking every follower for one would be noise,
    // and the feed query would return nothing anyway.
    let h = Harness::start().await;
    let alice = h.agent("alice").await;
    let bob = h.agent("bob").await;
    let t = alice
        .create_thread(&new_thread("t", "b", &[]))
        .await
        .unwrap();
    let id = t.posts[0].id;

    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        alice.react(id, "👀").await.expect("react");
    });

    let started = Instant::now();
    let feed = bob
        .feed(&FeedRequest::since(id).wait(Some(1)))
        .await
        .unwrap();
    assert!(feed.posts.is_empty());
    assert!(started.elapsed() >= Duration::from_millis(900));
}

#[tokio::test]
async fn a_v1_database_gains_reactions_without_losing_anything() {
    // The first migration this project has actually had to perform.
    let dir = tempfile::tempdir().expect("temp dir");
    let db = dir.path().join("b.sqlite").to_string_lossy().into_owned();

    // Build a v1 database by hand: schema as it shipped, with real content.
    {
        let conn = rusqlite::Connection::open(&db).expect("open");
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (1);
             CREATE TABLE agents (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE,
               token_hash TEXT NOT NULL UNIQUE, is_admin INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL);
             CREATE TABLE threads (id INTEGER PRIMARY KEY, title TEXT NOT NULL,
               author_id INTEGER NOT NULL REFERENCES agents(id),
               status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','closed')),
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE posts (id INTEGER PRIMARY KEY AUTOINCREMENT,
               thread_id INTEGER NOT NULL REFERENCES threads(id),
               author_id INTEGER NOT NULL REFERENCES agents(id),
               body TEXT NOT NULL, created_at TEXT NOT NULL);
             CREATE INDEX posts_thread ON posts(thread_id, id);
             CREATE TABLE thread_tags (thread_id INTEGER NOT NULL REFERENCES threads(id),
               tag TEXT NOT NULL, PRIMARY KEY (thread_id, tag));
             CREATE TABLE mentions (post_id INTEGER NOT NULL REFERENCES posts(id),
               agent_id INTEGER NOT NULL REFERENCES agents(id), PRIMARY KEY (post_id, agent_id));
             CREATE TABLE cursors (agent_id INTEGER PRIMARY KEY REFERENCES agents(id),
               last_seen INTEGER NOT NULL DEFAULT 0);",
        )
        .expect("v1 schema");
        let hash = babble::server::auth::hash_token(ADMIN);
        conn.execute(
            "INSERT INTO agents (id, name, token_hash, is_admin, created_at)
             VALUES (1, 'admin', ?1, 1, '2026-01-01T00:00:00Z')",
            [&hash],
        )
        .expect("agent");
        conn.execute_batch(
            "INSERT INTO threads (id,title,author_id,status,created_at,updated_at)
               VALUES (1,'from v1',1,'open','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z');
             INSERT INTO posts (id,thread_id,author_id,body,created_at)
               VALUES (1,1,1,'written before reactions existed','2026-01-01T00:00:00Z');",
        )
        .expect("content");
    }

    // Starting the server migrates it in place.
    let args = ServeArgs {
        db: db.clone(),
        bind: "127.0.0.1:0".into(),
        admin_token: Some(ADMIN.into()),
        post_rate: 0,
    };
    let (listener, state) = server::bind(&args).await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = server::serve(listener, state, std::future::pending()).await;
    });
    let client = Client::new(Resolved {
        url: format!("http://{addr}"),
        token: ADMIN.into(),
    })
    .expect("client");

    // The old content is intact...
    let detail = client.show_thread(1, None).await.unwrap();
    assert_eq!(detail.thread.title, "from v1");
    assert_eq!(detail.posts[0].body, "written before reactions existed");
    assert!(detail.posts[0].reactions.is_empty());

    // ...and the new feature works on it.
    let p = client.react(1, "🎉").await.unwrap();
    assert_eq!(p.reactions[0].emoji, "🎉");

    // Re-opening is a no-op, not a re-run.
    let version: i64 = rusqlite::Connection::open(&db)
        .unwrap()
        .query_row("SELECT version FROM schema_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
}
