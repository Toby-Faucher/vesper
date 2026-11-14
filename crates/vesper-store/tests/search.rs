use tempfile::TempDir;
use vesper_protocol::{Message, Role, SearchPage, SearchQuery};
use vesper_store::{NewAccount, NewMessage, SEARCH_SQL, Store, fts_query};

async fn open() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(&tmp.path().join("v.db")).await.unwrap();
    (tmp, store)
}

async fn user(s: &Store, username: &str) -> String {
    s.create_account(NewAccount {
        username,
        display_name: username,
        password_phc: None,
        role: Role::Member,
    })
    .await
    .unwrap()
    .user_id
}

async fn rooms(s: &Store) -> (i64, i64) {
    let r = s.rooms().await.unwrap();
    (r[0].id, r[1].id)
}

async fn post_at(s: &Store, room: i64, author: &str, body: &str, now: i64) -> Message {
    s.post_message(NewMessage {
        room_id: room,
        author_id: author,
        body,
        reply_to: None,
        mentions: &[],
        attachments: &[],
        now,
    })
    .await
    .unwrap()
}

fn q(text: &str) -> SearchQuery {
    SearchQuery {
        q: text.into(),
        room_id: None,
        author_id: None,
        from: None,
        to: None,
        before: None,
    }
}

fn ids(p: &SearchPage) -> Vec<i64> {
    let mut v: Vec<i64> = p.hits.iter().map(|h| h.message.id).collect();
    v.sort();
    v
}

#[test]
fn fts_query_quotes_operators() {
    assert_eq!(fts_query("cat OR dog").unwrap(), r#""cat" "OR" "dog"*"#);
    assert_eq!(fts_query("a\"bc").unwrap(), r#""a""bc"*"#);
    assert_eq!(fts_query("  "), None);
    assert_eq!(fts_query("***"), None);
    assert_eq!(fts_query("- ^ * ( ) :"), None);
    assert_eq!(
        fts_query("body:foo NEAR(xy").unwrap(),
        r#""body:foo" "NEAR(xy"*"#
    );
    assert_eq!(fts_query("a\0bc").unwrap(), r#""a" "bc"*"#);
    // A one-character last term is a whole word, never the slow `x*` prefix.
    assert_eq!(fts_query("a\0b").unwrap(), r#""a" "b""#);
    assert_eq!(fts_query("é").unwrap(), r#""é""#);
    // Only letters and digits count: these all end in a one-letter token.
    assert_eq!(fts_query("a.").unwrap(), r#""a.""#);
    assert_eq!(fts_query("a-").unwrap(), r#""a-""#);
    assert_eq!(fts_query("xy-a").unwrap(), r#""xy-a""#);
    assert_eq!(fts_query("e\u{301}").unwrap(), "\"e\u{301}\"");
    assert_eq!(fts_query("a-bc").unwrap(), r#""a-bc"*"#);
    assert_eq!(fts_query("ab").unwrap(), r#""ab"*"#);
    let twenty: Vec<String> = (0..20).map(|i| format!("w{i}")).collect();
    let out = fts_query(&twenty.join(" ")).unwrap();
    assert_eq!(out.matches('"').count(), 32);
    assert!(out.ends_with(r#""w15"*"#), "{out}");
    let long = fts_query(&"x".repeat(100)).unwrap();
    assert_eq!(long, format!("\"{}\"*", "x".repeat(64)));
}

#[tokio::test]
async fn search_finds_prefix_and_diacritics() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let (room, _) = rooms(&s).await;
    let m = post_at(&s, room, &a, "Café au lait", 1).await;
    post_at(&s, room, &a, "tea please", 2).await;
    for text in ["cafe", "CAFÉ", "lai", "au lai"] {
        let p = s.search(&q(text), &[room], 20).await.unwrap();
        assert_eq!(ids(&p), vec![m.id], "{text}");
    }
    let p = s.search(&q("cafe"), &[room], 20).await.unwrap();
    assert_eq!(p.hits[0].snippet, "\u{2}Café\u{3} au lait");
    assert_eq!(p.hits[0].message.author_id, a);
    assert_eq!(p.hits[0].message.room_id, room);
    assert_eq!(p.hits[0].message.created_at, 1);
    // A prefix only applies to the last term.
    assert!(
        s.search(&q("ca lait"), &[room], 20)
            .await
            .unwrap()
            .hits
            .is_empty()
    );
}

#[tokio::test]
async fn search_filters_room_author_date() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let b = user(&s, "bobby").await;
    let (r1, r2) = rooms(&s).await;
    let m1 = post_at(&s, r1, &a, "pizza night", 100).await.id;
    let m2 = post_at(&s, r1, &b, "pizza again", 200).await.id;
    let m3 = post_at(&s, r2, &a, "pizza here", 300).await.id;
    let m4 = post_at(&s, r2, &b, "pizza there", 400).await.id;
    let both = [r1, r2];
    let run = |f: &dyn Fn(&mut SearchQuery)| {
        let mut sq = q("pizza");
        f(&mut sq);
        let s = s.clone();
        async move { ids(&s.search(&sq, &both, 20).await.unwrap()) }
    };
    assert_eq!(run(&|_| {}).await, vec![m1, m2, m3, m4]);
    assert_eq!(run(&|q| q.room_id = Some(r2)).await, vec![m3, m4]);
    assert_eq!(run(&|q| q.author_id = Some(a.clone())).await, vec![m1, m3]);
    // Both ends inclusive.
    assert_eq!(run(&|q| q.from = Some(200)).await, vec![m2, m3, m4]);
    assert_eq!(run(&|q| q.to = Some(300)).await, vec![m1, m2, m3]);
    assert_eq!(
        run(&|q| {
            q.from = Some(200);
            q.to = Some(300);
        })
        .await,
        vec![m2, m3]
    );
    assert_eq!(
        run(&|q| {
            q.room_id = Some(r2);
            q.author_id = Some(b.clone());
            q.from = Some(400);
        })
        .await,
        vec![m4]
    );
    assert_eq!(
        run(&|q| q.author_id = Some("nobody".into())).await,
        Vec::<i64>::new()
    );
}

#[tokio::test]
async fn search_skips_invisible_rooms_and_deleted() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let (open_room, hidden) = rooms(&s).await;
    // Many better and newer matches in the hidden room must not shorten the visible page.
    let mut visible = Vec::new();
    for i in 0..5 {
        visible.push(
            post_at(&s, open_room, &a, "secret plan with many other words", i)
                .await
                .id,
        );
    }
    for i in 0..30 {
        post_at(&s, hidden, &a, "secret", 100 + i).await;
    }
    let gone = post_at(&s, open_room, &a, "secret gone", 200).await.id;
    s.delete_message(gone, 201, &a).await.unwrap();
    let edited = post_at(&s, open_room, &a, "old words", 300).await.id;
    s.edit_message(edited, &a, "fresh words", &[], 301)
        .await
        .unwrap();

    let p = s.search(&q("secret"), &[open_room], 5).await.unwrap();
    assert_eq!(ids(&p), visible);
    assert_eq!(p.next_before, None, "no hidden row may leak as a next page");
    assert!(
        s.search(&q("gone"), &[open_room, hidden], 50)
            .await
            .unwrap()
            .hits
            .is_empty()
    );
    // Asking for a hidden room by id gives nothing either.
    let mut sq = q("secret");
    sq.room_id = Some(hidden);
    assert!(
        s.search(&sq, &[open_room], 50)
            .await
            .unwrap()
            .hits
            .is_empty()
    );
    assert!(
        s.search(&q("secret"), &[], 50)
            .await
            .unwrap()
            .hits
            .is_empty()
    );
    // Edited: found by the new text only.
    assert!(
        s.search(&q("old"), &[open_room], 50)
            .await
            .unwrap()
            .hits
            .is_empty()
    );
    assert_eq!(
        ids(&s.search(&q("fresh"), &[open_room], 50).await.unwrap()),
        vec![edited]
    );
    // A banned author's messages stay searchable.
    s.set_banned(&a, true, 400).await.unwrap();
    assert_eq!(
        s.search(&q("secret"), &[open_room], 50)
            .await
            .unwrap()
            .hits
            .len(),
        5
    );
}

#[tokio::test]
async fn search_pages_with_next_before() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let (room, other) = rooms(&s).await;
    let mut all = Vec::new();
    for i in 0..23 {
        all.push(post_at(&s, room, &a, "apple pie", i).await.id);
        post_at(&s, other, &a, "apple", i).await;
    }
    let mut sq = q("apple");
    let mut seen = Vec::new();
    loop {
        let p = s.search(&sq, &[room], 5).await.unwrap();
        assert!(p.hits.len() == 5 || p.next_before.is_none());
        seen.extend(p.hits.iter().map(|h| h.message.id));
        // A message posted between pages never shifts the next page.
        post_at(&s, room, &a, "apple late", 99).await;
        match p.next_before {
            Some(b) => sq.before = Some(b),
            None => break,
        }
    }
    all.reverse();
    assert_eq!(seen, all, "newest first, every hit once");
}

#[tokio::test]
async fn hostile_query_never_errors() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let (room, _) = rooms(&s).await;
    post_at(&s, room, &a, "near body col x", 1).await;
    let big = "a\"(* ".repeat(2048);
    for text in [
        "NEAR(",
        "\"",
        "col:x",
        "body:x",
        "^",
        "a AND",
        "OR",
        "NOT x",
        "-x",
        "x*",
        "(a",
        "\"\"",
        "\u{903}",
        "a\0b",
        "{body}: x",
        "\u{2}x\u{3}",
        &big,
    ] {
        s.search(&q(text), &[room], 20)
            .await
            .unwrap_or_else(|e| panic!("{text:?}: {e}"));
    }
    // Column filter text is just words (here the phrase "near body").
    let p = s.search(&q("near:body"), &[room], 20).await.unwrap();
    assert_eq!(p.hits.len(), 1);
}

#[tokio::test]
async fn markers_in_a_body_cannot_fake_a_highlight() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let (room, _) = rooms(&s).await;
    // The server refuses control characters; the store must stay safe if one gets in anyway.
    post_at(&s, room, &a, "match \u{2}fake\u{3} here", 1).await;
    let p = s.search(&q("match"), &[room], 20).await.unwrap();
    assert_eq!(p.hits[0].snippet, "match fake here");
}

#[tokio::test]
async fn search_plan_uses_fts_and_primary_key() {
    let (_tmp, s) = open().await;
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "EXPLAIN QUERY PLAN {SEARCH_SQL}"
    )))
    .bind("\"x\"*")
    .bind("[1]")
    .bind(None::<i64>)
    .bind(None::<String>)
    .bind(None::<i64>)
    .bind(None::<i64>)
    .bind(None::<i64>)
    .bind(10)
    .fetch_all(s.pool())
    .await
    .unwrap();
    let plan: Vec<&str> = rows.iter().map(|r| r.3.as_str()).collect();
    assert!(
        plan.iter()
            .any(|l| l.starts_with("SCAN messages_fts VIRTUAL TABLE INDEX")),
        "{plan:?}"
    );
    assert!(
        plan.iter()
            .any(|l| l.starts_with("SEARCH m USING INTEGER PRIMARY KEY")),
        "{plan:?}"
    );
    assert!(
        !plan
            .iter()
            .any(|l| *l == "SCAN m" || l.starts_with("SCAN m ")),
        "{plan:?}"
    );
    // FTS5 hands rows over newest first, so the page stops early instead of sorting every match.
    assert!(!plan.iter().any(|l| l.contains("TEMP B-TREE")), "{plan:?}");
}
