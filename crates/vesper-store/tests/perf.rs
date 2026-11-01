//! Run by hand, in release:
//! `scripts/capped.sh cargo test --release -p vesper-store --test perf -- --ignored --nocapture`

use std::time::{Duration, Instant};

use tempfile::TempDir;
use vesper_protocol::Role;
use vesper_store::{Cursor, NewAccount, Store};

const WORDS: &[&str] = &[
    "hello", "voice", "game", "tonight", "café", "build", "server", "music", "later", "where",
    "what", "the", "and", "lag", "mic", "stream", "link", "photo", "lunch", "weekend", "deploy",
    "bug",
];

#[tokio::test(flavor = "multi_thread")]
#[ignore]
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
    println!("search: skipped until Task 15 adds it");
    println!(
        "db size after checkpoint: {:.1} MiB",
        size as f64 / 1_048_576.0
    );
    for (what, d) in [("latest", latest), ("before", before), ("unread", unread)] {
        assert!(d < Duration::from_millis(50), "{what} took {d:?}");
    }
}
