use tempfile::TempDir;
use vesper_protocol::{Message, Role};
use vesper_store::{Cursor, NewAccount, NewMessage, Store, StoreError};

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

async fn general(s: &Store) -> i64 {
    s.rooms().await.unwrap()[0].id
}

fn msg<'a>(room_id: i64, author_id: &'a str, body: &'a str) -> NewMessage<'a> {
    NewMessage {
        room_id,
        author_id,
        body,
        reply_to: None,
        mentions: &[],
        attachments: &[],
        now: 1000,
    }
}

async fn post(s: &Store, room: i64, author: &str, body: &str) -> Message {
    s.post_message(msg(room, author, body)).await.unwrap()
}

async fn fts_count(s: &Store, word: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM messages_fts WHERE messages_fts MATCH ?1")
        .bind(word)
        .fetch_one(s.pool())
        .await
        .unwrap()
}

fn ids(page: &vesper_protocol::MessagePage) -> Vec<i64> {
    page.messages.iter().map(|m| m.id).collect()
}

#[tokio::test]
async fn history_pages_backwards_without_gaps() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let room = general(&s).await;
    let mut posted = Vec::new();
    for i in 0..250 {
        posted.push(post(&s, room, &a, &format!("message {i}")).await.id);
    }
    let mut seen = Vec::new();
    let mut page = s.history(room, Cursor::Latest, 100).await.unwrap();
    assert!(!page.more_after);
    loop {
        let p = ids(&page);
        assert!(p.windows(2).all(|w| w[0] < w[1]), "ascending within a page");
        seen.splice(0..0, p.iter().copied());
        if !page.more_before {
            break;
        }
        assert!(page.more_before);
        page = s.history(room, Cursor::Before(p[0]), 100).await.unwrap();
        assert!(page.more_after);
    }
    assert_eq!(seen, posted);
    // A message posted while paging does not shift an older cursor.
    post(&s, room, &a, "late").await;
    let p = s
        .history(room, Cursor::Before(posted[150]), 100)
        .await
        .unwrap();
    assert_eq!(ids(&p), posted[50..150]);
    let p = s
        .history(room, Cursor::After(posted[100]), 100)
        .await
        .unwrap();
    assert_eq!(p.messages.len(), 100);
    assert_eq!(p.messages[0].id, posted[101]);
    assert!(p.more_before && p.more_after);
    // Clamp.
    assert_eq!(
        s.history(room, Cursor::Latest, 0)
            .await
            .unwrap()
            .messages
            .len(),
        1
    );
    assert_eq!(
        s.history(room, Cursor::Latest, 500)
            .await
            .unwrap()
            .messages
            .len(),
        100
    );
}

#[tokio::test]
async fn around_centres_on_target() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let room = general(&s).await;
    let mut posted = Vec::new();
    for i in 0..30 {
        posted.push(post(&s, room, &a, &format!("m {i}")).await.id);
    }
    let p = s
        .history(room, Cursor::Around(posted[15]), 10)
        .await
        .unwrap();
    assert_eq!(ids(&p), posted[10..21]);
    assert!(p.more_before && p.more_after);
    // Near the start: fewer before, no more.
    let p = s
        .history(room, Cursor::Around(posted[2]), 10)
        .await
        .unwrap();
    assert_eq!(ids(&p), posted[0..8]);
    assert!(!p.more_before && p.more_after);
    // A target in another room is NotFound.
    let other = s.rooms().await.unwrap()[1].id;
    assert!(matches!(
        s.history(other, Cursor::Around(posted[2]), 10).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn edit_only_by_author_and_not_after_delete() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let b = user(&s, "bob").await;
    let room = general(&s).await;
    let m = post(&s, room, &a, "first").await;
    assert!(matches!(
        s.edit_message(m.id, &b, "hacked", &[], 2000).await,
        Err(StoreError::NotFound)
    ));
    let e = s
        .edit_message(m.id, &a, "second", std::slice::from_ref(&b), 2000)
        .await
        .unwrap();
    assert_eq!(e.body, "second");
    assert_eq!(e.edited_at, Some(2000));
    assert_eq!(e.mentions, [b.as_str()]);
    let e = s.edit_message(m.id, &a, "third", &[], 2500).await.unwrap();
    assert!(e.mentions.is_empty(), "edit replaces mentions");
    s.delete_message(m.id, 3000, &b).await.unwrap();
    assert!(matches!(
        s.edit_message(m.id, &a, "again", &[], 4000).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.delete_message(m.id, 4000, &a).await,
        Err(StoreError::NotFound)
    ));
    assert!(matches!(
        s.edit_message(9999, &a, "x", &[], 1).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn delete_is_soft_and_removes_from_fts() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let b = user(&s, "bob").await;
    let room = general(&s).await;
    let m = post(&s, room, &a, "a secret word here").await;
    let reply = s
        .post_message(NewMessage {
            reply_to: Some(m.id),
            ..msg(room, &b, "what word?")
        })
        .await
        .unwrap();
    assert_eq!(
        reply.reply_to.as_ref().unwrap().excerpt,
        "a secret word here"
    );
    s.set_reaction(m.id, &b, "👍", true, 1).await.unwrap();
    s.set_pinned(m.id, &b, true, 1).await.unwrap();
    assert_eq!(fts_count(&s, "secret").await, 1);
    assert_eq!(fts_count(&s, "word").await, 2);

    let (d, uploads) = s.delete_message(m.id, 5000, &a).await.unwrap();
    assert!(d.deleted && d.body.is_empty());
    assert!(d.reactions.is_empty() && !d.pinned && uploads.is_empty());
    assert_eq!(s.message(m.id).await.unwrap(), d);
    let r = s.message(reply.id).await.unwrap();
    let preview = r.reply_to.unwrap();
    assert_eq!(preview.id, m.id);
    assert_eq!(preview.excerpt, "");
    assert_eq!(fts_count(&s, "secret").await, 0);
    assert_eq!(fts_count(&s, "word").await, 1);
    // The tombstone stays in history.
    let page = s.history(room, Cursor::Latest, 10).await.unwrap();
    assert_eq!(ids(&page), [m.id, reply.id]);
    assert!(page.messages[0].deleted);
    assert!(s.pins(room).await.unwrap().is_empty());
    // No 'integrity-check' here: a tombstone row has no index entry on purpose, which that check
    // reports as a mismatch with the content table.
}

#[tokio::test]
async fn insert_updates_fts_and_other_updates_do_not() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let room = general(&s).await;
    let m = post(&s, room, &a, "Café au lait").await;
    assert_eq!(fts_count(&s, "cafe").await, 1);
    s.set_pinned(m.id, &a, true, 1).await.unwrap();
    s.set_reaction(m.id, &a, "☕", true, 1).await.unwrap();
    assert_eq!(fts_count(&s, "cafe").await, 1);
    sqlx::query("INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1)")
        .execute(s.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn edit_updates_fts() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let room = general(&s).await;
    let m = post(&s, room, &a, "the old banana").await;
    assert_eq!(fts_count(&s, "banana").await, 1);
    s.edit_message(m.id, &a, "a new cherry", &[], 2)
        .await
        .unwrap();
    assert_eq!(fts_count(&s, "banana").await, 0);
    assert_eq!(fts_count(&s, "cherry").await, 1);
    sqlx::query("INSERT INTO messages_fts (messages_fts, rank) VALUES ('integrity-check', 1)")
        .execute(s.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn reply_must_be_same_room() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let rooms = s.rooms().await.unwrap();
    let m = post(&s, rooms[0].id, &a, "here").await;
    for reply_to in [m.id, 9999] {
        assert!(matches!(
            s.post_message(NewMessage {
                reply_to: Some(reply_to),
                ..msg(rooms[1].id, &a, "there")
            })
            .await,
            Err(StoreError::Conflict(_))
        ));
    }
    assert!(
        s.history(rooms[1].id, Cursor::Latest, 10)
            .await
            .unwrap()
            .messages
            .is_empty()
    );
    let ok = s
        .post_message(NewMessage {
            reply_to: Some(m.id),
            ..msg(rooms[0].id, &a, "same room")
        })
        .await
        .unwrap();
    assert_eq!(ok.reply_to.unwrap().author_id, a);
}

#[tokio::test]
async fn reactions_toggle_and_cap() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let b = user(&s, "bob").await;
    let room = general(&s).await;
    let m = post(&s, room, &a, "react").await;
    s.set_reaction(m.id, &a, "👍", true, 1).await.unwrap();
    let r = s.set_reaction(m.id, &a, "👍", true, 2).await.unwrap();
    assert_eq!(r.reactions.len(), 1);
    assert_eq!(r.reactions[0].user_ids, [a.as_str()]);
    let r = s.set_reaction(m.id, &b, "👍", true, 3).await.unwrap();
    assert_eq!(r.reactions[0].user_ids, [a.clone(), b.clone()]);
    let r = s.set_reaction(m.id, &a, "👍", false, 4).await.unwrap();
    assert_eq!(r.reactions[0].user_ids, [b.as_str()]);
    s.set_reaction(m.id, &a, "👍", false, 5).await.unwrap();
    // 19 more distinct emoji makes 20.
    for i in 0..19u32 {
        let e = char::from_u32(0x1F600 + i).unwrap().to_string();
        s.set_reaction(m.id, &a, &e, true, 10).await.unwrap();
    }
    assert!(matches!(
        s.set_reaction(m.id, &a, "🚀", true, 11).await,
        Err(StoreError::Conflict(_))
    ));
    // An emoji already there can still gain users.
    let r = s.set_reaction(m.id, &a, "👍", true, 12).await.unwrap();
    assert_eq!(r.reactions.len(), 20);
    s.delete_message(m.id, 13, &a).await.unwrap();
    assert!(matches!(
        s.set_reaction(m.id, &a, "👍", true, 14).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn pins_cap_at_50() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let rooms = s.rooms().await.unwrap();
    let room = rooms[0].id;
    let mut posted = Vec::new();
    for i in 0..51 {
        posted.push(post(&s, room, &a, &format!("pin {i}")).await.id);
    }
    for (i, id) in posted[..50].iter().enumerate() {
        assert!(s.set_pinned(*id, &a, true, i as i64).await.unwrap().pinned);
    }
    // Pinning a pinned one again is fine; the 51st is not.
    s.set_pinned(posted[0], &a, true, 100).await.unwrap();
    assert!(matches!(
        s.set_pinned(posted[50], &a, true, 100).await,
        Err(StoreError::Conflict(_))
    ));
    // Another room has its own 50.
    let other = post(&s, rooms[1].id, &a, "elsewhere").await;
    s.set_pinned(other.id, &a, true, 100).await.unwrap();
    let pins = s.pins(room).await.unwrap();
    assert_eq!(pins.len(), 50);
    assert_eq!(pins[0].id, posted[49], "newest pin first");
    s.set_pinned(posted[0], &a, false, 101).await.unwrap();
    assert!(
        s.set_pinned(posted[50], &a, true, 102)
            .await
            .unwrap()
            .pinned
    );
}

#[tokio::test]
async fn unread_counts_and_mentions() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let b = user(&s, "bob").await;
    let rooms = s.rooms().await.unwrap();
    let (room, other) = (rooms[0].id, rooms[1].id);
    let mut posted = Vec::new();
    for i in 0..5 {
        let mentions = if i == 3 { vec![b.clone()] } else { vec![] };
        let m = s
            .post_message(NewMessage {
                mentions: &mentions,
                ..msg(room, &a, &format!("hi {i}"))
            })
            .await
            .unwrap();
        posted.push(m.id);
    }
    // bob's own message is never unread for him.
    post(&s, room, &b, "mine").await;
    let u = s.unread(&b, &[room, other]).await.unwrap();
    assert_eq!(
        (u[0].room_id, u[0].unread, u[0].mentions, u[0].last_read_id),
        (room, 5, 1, 0)
    );
    assert_eq!((u[1].room_id, u[1].unread, u[1].mentions), (other, 0, 0));

    s.mark_read(&b, room, posted[1]).await.unwrap();
    let u = s.unread(&b, &[room]).await.unwrap();
    assert_eq!(
        (u[0].unread, u[0].mentions, u[0].last_read_id),
        (3, 1, posted[1])
    );
    s.mark_read(&b, room, posted[0]).await.unwrap();
    assert_eq!(
        s.unread(&b, &[room]).await.unwrap()[0].last_read_id,
        posted[1],
        "never backwards"
    );
    // A deleted message no longer counts, nor does its mention.
    s.delete_message(posted[3], 9, &a).await.unwrap();
    let u = s.unread(&b, &[room]).await.unwrap();
    assert_eq!((u[0].unread, u[0].mentions), (2, 0));
    s.mark_read(&b, room, posted[4]).await.unwrap();
    assert_eq!(s.unread(&b, &[room]).await.unwrap()[0].unread, 0);
    // The order follows the request.
    let u = s.unread(&b, &[other, room]).await.unwrap();
    assert_eq!((u[0].room_id, u[1].room_id), (other, room));
    assert!(matches!(
        s.mark_read(&b, 9999, 1).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn unread_uses_the_room_index() {
    let (_tmp, s) = open().await;
    let plan: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "EXPLAIN QUERY PLAN SELECT count(*) FROM messages m
         WHERE m.room_id = 1 AND m.id > 5 AND m.deleted_at IS NULL AND m.author_id <> 'x'",
    )
    .fetch_all(s.pool())
    .await
    .unwrap();
    let detail: Vec<&str> = plan.iter().map(|p| p.3.as_str()).collect();
    assert!(
        detail
            .iter()
            .any(|d| d.contains("COVERING INDEX messages_room")),
        "{detail:?}"
    );
}

#[tokio::test]
async fn mentions_resolve_usernames() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let b = user(&s, "bob").await;
    let got = s
        .account_ids_by_username(&["bob".into(), "nobody".into(), "alice".into(), "x\"y".into()])
        .await
        .unwrap();
    assert_eq!(got, [a, b]);
    assert!(s.account_ids_by_username(&[]).await.unwrap().is_empty());
}

async fn upload(s: &Store, id: &str, room: i64, uploader: &str, mime: &str) {
    sqlx::query(
        "INSERT INTO uploads (id, room_id, uploader_id, name, mime, size, created_at)
         VALUES (?1, ?2, ?3, 'f.png', ?4, 10, 1)",
    )
    .bind(id)
    .bind(room)
    .bind(uploader)
    .bind(mime)
    .execute(s.pool())
    .await
    .unwrap();
}

#[tokio::test]
async fn attachments_must_be_own_unattached_same_room() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let b = user(&s, "bob").await;
    let rooms = s.rooms().await.unwrap();
    let room = rooms[0].id;
    let (img, doc, bobs, elsewhere) = (
        "a".repeat(32),
        "b".repeat(32),
        "c".repeat(32),
        "d".repeat(32),
    );
    upload(&s, &img, room, &a, "image/png").await;
    upload(&s, &doc, room, &a, "application/octet-stream").await;
    upload(&s, &bobs, room, &b, "image/png").await;
    upload(&s, &elsewhere, rooms[1].id, &a, "image/png").await;
    for bad in [&bobs, &elsewhere, &"e".repeat(32)] {
        let list = [img.clone(), bad.clone()];
        assert!(matches!(
            s.post_message(NewMessage {
                attachments: &list,
                ..msg(room, &a, "files")
            })
            .await,
            Err(StoreError::Conflict(_))
        ));
    }
    assert!(
        s.history(room, Cursor::Latest, 10)
            .await
            .unwrap()
            .messages
            .is_empty(),
        "rolled back"
    );
    let list = [img.clone(), doc.clone()];
    let m = s
        .post_message(NewMessage {
            attachments: &list,
            ..msg(room, &a, "files")
        })
        .await
        .unwrap();
    let got: Vec<(&str, bool)> = m
        .attachments
        .iter()
        .map(|x| (x.id.as_str(), x.image))
        .collect();
    assert_eq!(got, [(img.as_str(), true), (doc.as_str(), false)]);
    // Already attached.
    let again = [img.clone()];
    assert!(matches!(
        s.post_message(NewMessage {
            attachments: &again,
            ..msg(room, &a, "again")
        })
        .await,
        Err(StoreError::Conflict(_))
    ));
    let (d, mut removed) = s.delete_message(m.id, 9, &a).await.unwrap();
    removed.sort();
    assert_eq!(removed, [img, doc]);
    assert!(d.attachments.is_empty());
}

#[tokio::test]
async fn upload_rows_sum_sweep_and_skip_attached() {
    let (_tmp, s) = open().await;
    let a = user(&s, "alice").await;
    let room = general(&s).await;
    let row = |id: &str, size: i64, created_at: i64| vesper_store::UploadRow {
        id: id.into(),
        room_id: room,
        uploader_id: a.clone(),
        message_id: None,
        name: "f.bin".into(),
        mime: "application/octet-stream".into(),
        size,
        created_at,
    };
    let (old, new, attached) = ("a".repeat(32), "b".repeat(32), "c".repeat(32));
    s.insert_upload(&row(&old, 10, 1)).await.unwrap();
    s.insert_upload(&row(&new, 20, 500)).await.unwrap();
    s.insert_upload(&row(&attached, 30, 1)).await.unwrap();
    assert!(matches!(
        s.insert_upload(&row(&old, 1, 1)).await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(s.upload_bytes().await.unwrap(), 60);
    assert_eq!(s.upload(&new).await.unwrap(), row(&new, 20, 500));
    assert!(matches!(
        s.upload(&"d".repeat(32)).await,
        Err(StoreError::NotFound)
    ));
    let stale = s.stale_uploads(100).await.unwrap();
    assert_eq!(stale, [old.clone(), attached.clone()]);
    // Attached between the listing and the delete: the row must stay.
    s.post_message(NewMessage {
        attachments: std::slice::from_ref(&attached),
        ..msg(room, &a, "late")
    })
    .await
    .unwrap();
    assert_eq!(s.delete_uploads(&stale).await.unwrap(), [old]);
    assert_eq!(s.upload_bytes().await.unwrap(), 50);
    assert!(s.upload(&attached).await.unwrap().message_id.is_some());
}
