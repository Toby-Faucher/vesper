//! Run by hand, in release:
//! `scripts/capped.sh cargo test --release -p vesper-store --test perf -- --ignored --nocapture`

use std::time::{Duration, Instant};

use tempfile::TempDir;
use vesper_protocol::{Role, SearchQuery};
use vesper_store::{Cursor, NewAccount, Store};

const WORDS: &[&str] = &[
    "hello", "voice", "game", "tonight", "café", "build", "server", "music", "later", "where",
    "what", "the", "and", "lag", "mic", "stream", "link", "photo", "lunch", "weekend", "deploy",
    "bug",
];

#[tokio::test(flavor = "multi_thread")]
#[ignore = "a timing run, by hand in release, see the file header"]
async fn perf_100k_messages() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("v.db");
    let s = Store::open(&path).await.unwrap();
    let mut authors = Vec::new();
    for i in 0..10 {
        let a = s
            .create_account(NewAccount {
                username: &format!("user{i}"),
                display_name: "u",
                password_phc: None,
                role: Role::Member,
            })
            .await
            .unwrap();
        authors.push(a.user_id);
    }
    let mut rooms: Vec<i64> = s.rooms().await.unwrap().iter().map(|r| r.id).collect();
    for i in rooms.len()..10 {
        rooms.push(s.create_room(&format!("room{i}"), 0).await.unwrap().id);
    }

    // Deterministic bodies, 10 to 300 chars.
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        (seed >> 33) as usize
    };
    let t = Instant::now();
    let mut tx = s.pool().begin().await.unwrap();
    for i in 0..100_000i64 {
        let len = 10 + next() % 291;
        let mut body = String::new();
        while body.chars().count() < len {
            body.push_str(WORDS[next() % WORDS.len()]);
            body.push(' ');
        }
        let body: String = body
            .chars()
            .take(len)
            .collect::<String>()
            .trim_end()
            .to_owned()
            + ".";
        sqlx::query(
            "INSERT INTO messages (room_id, author_id, body, created_at) VALUES (?1, ?2, ?3, ?4)",
        )
        .bind(rooms[(i % 10) as usize])
        .bind(&authors[next() % 10])
        .bind(&body)
        .bind(i)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let insert = t.elapsed();

    let reader = &authors[0];
    let room = rooms[0];
    // Half the rooms read up to the middle, half never read.
    let mid: i64 = 50_000;
    for r in &rooms[..5] {
        s.mark_read(reader, *r, mid).await.unwrap();
    }
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(s.pool())
        .await
        .unwrap();
    let size = std::fs::metadata(&path).unwrap().len();

    async fn time<T, F: std::future::Future<Output = T>>(f: impl Fn() -> F) -> Duration {
        f().await; // warm
        let t = Instant::now();
        for _ in 0..10 {
            f().await;
        }
        t.elapsed() / 10
    }
    let latest = time(|| s.history(room, Cursor::Latest, 50)).await;
    let before = time(|| s.history(room, Cursor::Before(mid), 50)).await;
    let unread = time(|| s.unread(reader, &rooms)).await;
    let page = s.history(room, Cursor::Latest, 50).await.unwrap();
    assert_eq!(page.messages.len(), 50);
    let u = s.unread(reader, &rooms).await.unwrap();

    println!("insert 100000 messages in one transaction: {insert:?}");
    println!("history Latest 50: {latest:?}");
    println!("history Before(mid) 50: {before:?}");
    println!(
        "unread for 10 rooms: {unread:?} (counts {:?})",
        u.iter().map(|x| x.unread).collect::<Vec<_>>()
    );
    // p50 and p95 of 50 runs per query, over all 10 rooms, page of 20.
    let mut searches = Vec::new();
    // The last one is the worst case: only the oldest matches pass the filter, so the whole
    // doclist is walked.
    for (what, text, to) in [
        ("1 word", "hello", None),
        ("3 words", "voice game tonight", None),
        ("prefix", "mu", None),
        ("1 word, oldest 1000 only", "hello", Some(1000)),
    ] {
        let sq = SearchQuery {
            q: text.into(),
            room_id: None,
            author_id: None,
            from: None,
            to,
            before: None,
        };
        let hits = s.search(&sq, &rooms, 20).await.unwrap().hits.len();
        assert_eq!(hits, 20, "{what}");
        let mut runs = Vec::new();
        for _ in 0..50 {
            let t = Instant::now();
            s.search(&sq, &rooms, 20).await.unwrap();
            runs.push(t.elapsed());
        }
        runs.sort();
        let (p50, p95) = (runs[24], runs[47]);
        println!("search {what} ({text:?}): p50 {p50:?}, p95 {p95:?}");
        searches.push((what, p95));
    }
    println!(
        "db size after checkpoint: {:.1} MiB",
        size as f64 / 1_048_576.0
    );
    for (what, d) in [("latest", latest), ("before", before), ("unread", unread)]
        .into_iter()
        .chain(searches)
    {
        assert!(d < Duration::from_millis(50), "{what} took {d:?}");
    }
}
