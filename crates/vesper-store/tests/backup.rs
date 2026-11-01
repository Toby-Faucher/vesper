use tempfile::TempDir;
use vesper_protocol::Role;
use vesper_store::{NewAccount, NewMessage, Store};

/// Test-only SQL; `table` comes from a fixed list in the tests.
async fn count(s: &Store, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(s.pool())
        .await
        .unwrap()
}

async fn populated(path: &std::path::Path) -> Store {
    let s = Store::open(path).await.unwrap();
    let user = s
        .create_account(NewAccount {
            username: "ann",
            display_name: "Ann",
            password_phc: None,
            role: Role::Member,
        })
        .await
        .unwrap()
        .user_id;
    let room = s.create_room("games", 1).await.unwrap().id;
    for i in 0..20 {
        let body = format!("hello {i}");
        s.post_message(NewMessage {
            room_id: room,
            author_id: &user,
            body: &body,
            reply_to: None,
            mentions: &[],
            attachments: &[],
            now: i,
        })
        .await
        .unwrap();
    }
    s
}

#[tokio::test]
async fn backup_opens_with_the_same_rows() {
    let tmp = TempDir::new().unwrap();
    let s = populated(&tmp.path().join("v.db")).await;
    let dest = tmp.path().join("copy.db");
    s.backup_into(&dest).await.unwrap();
    let copy = Store::open(&dest).await.unwrap();
    for t in ["accounts", "rooms", "messages", "messages_fts"] {
        assert_eq!(count(&copy, t).await, count(&s, t).await, "{t}");
    }
    assert_eq!(count(&copy, "messages").await, 20);
    assert_eq!(copy.rooms().await.unwrap(), s.rooms().await.unwrap());
    let ok = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_one(copy.pool())
        .await
        .unwrap();
    assert_eq!(ok, "ok");
    // A second backup to the same file is refused, not merged.
    assert!(s.backup_into(&dest).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_while_another_connection_writes() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("v.db");
    let s = populated(&path).await;
    let writer = Store::open(&path).await.unwrap();
    let w = tokio::spawn(async move {
        for i in 0..200 {
            writer.create_room(&format!("r{i}"), i).await.unwrap();
        }
    });
    let mut copies = Vec::new();
    for i in 0..5 {
        let dest = tmp.path().join(format!("copy{i}.db"));
        s.backup_into(&dest).await.unwrap();
        copies.push(dest);
    }
    w.await.unwrap();
    for dest in copies {
        let copy = Store::open(&dest).await.unwrap();
        let ok = sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
            .fetch_one(copy.pool())
            .await
            .unwrap();
        assert_eq!(ok, "ok");
        // A snapshot: positions are dense up to whatever the writer had committed.
        let rooms = copy.rooms().await.unwrap();
        assert!(rooms.len() >= 3, "{}", rooms.len());
        for (i, r) in rooms.iter().enumerate() {
            assert_eq!(r.position, i as i64);
        }
        assert_eq!(count(&copy, "messages").await, 20);
    }
}
