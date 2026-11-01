use tempfile::TempDir;
use vesper_protocol::Role;
use vesper_store::{NewAccount, Store, StoreError};

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

fn names(rooms: &[vesper_protocol::RoomInfo]) -> Vec<&str> {
    rooms.iter().map(|r| r.name.as_str()).collect()
}

#[tokio::test]
async fn fresh_db_has_general() {
    let (_tmp, s) = open().await;
    let rooms = s.rooms().await.unwrap();
    assert_eq!(names(&rooms), ["general", "hangout"]);
    assert!(rooms.iter().all(|r| !r.restricted && r.members.is_empty()));
    assert_eq!(rooms[0].position, 0);
}

#[tokio::test]
async fn room_names_unique_ignoring_case() {
    let (_tmp, s) = open().await;
    let r = s.create_room("Games", 5).await.unwrap();
    assert_eq!(r.position, 2);
    for name in ["games", "GENERAL", "Hangout"] {
        assert!(
            matches!(s.create_room(name, 6).await, Err(StoreError::Conflict(_))),
            "{name}"
        );
    }
    let general = s.rooms().await.unwrap()[0].id;
    assert!(matches!(
        s.patch_room(general, Some("GAMES"), None, None, 0).await,
        Err(StoreError::Conflict(_))
    ));
    // A case-only rename of the same room is fine.
    let g = s
        .patch_room(general, Some("General"), None, None, 0)
        .await
        .unwrap();
    assert_eq!(g.name, "General");
    assert!(matches!(
        s.patch_room(999, Some("x"), None, None, 0).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn reorder_requires_permutation() {
    let (_tmp, s) = open().await;
    let c = s.create_room("c", 1).await.unwrap().id;
    let rooms = s.rooms().await.unwrap();
    let (a, b) = (rooms[0].id, rooms[1].id);
    for bad in [
        vec![a, b],
        vec![a, b, c, c],
        vec![a, b, b],
        vec![a, b, c, 999],
        vec![],
    ] {
        assert!(
            matches!(s.reorder_rooms(&bad).await, Err(StoreError::Conflict(_))),
            "{bad:?}"
        );
    }
    assert_eq!(
        names(&s.rooms().await.unwrap()),
        ["general", "hangout", "c"]
    );
    s.reorder_rooms(&[c, a, b]).await.unwrap();
    let rooms = s.rooms().await.unwrap();
    assert_eq!(names(&rooms), ["c", "general", "hangout"]);
    assert_eq!(
        rooms.iter().map(|r| r.position).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    // A new room still goes last.
    s.create_room("d", 2).await.unwrap();
    assert_eq!(s.rooms().await.unwrap()[3].name, "d");
}

#[tokio::test]
async fn members_only_for_restricted() {
    let (_tmp, s) = open().await;
    let alice = user(&s, "alice").await;
    let bob = user(&s, "bob").await;
    let id = s.rooms().await.unwrap()[1].id;

    let r = s
        .set_room_members(id, &[bob.clone(), alice.clone(), bob.clone()])
        .await
        .unwrap();
    assert!(!r.restricted && r.members.is_empty(), "{r:?}");

    let r = s.patch_room(id, None, Some(true), None, 0).await.unwrap();
    let mut want = vec![alice.clone(), bob.clone()];
    want.sort();
    assert_eq!(r.members, want);
    assert_eq!(s.rooms().await.unwrap()[1].members, want);
    assert!(s.rooms().await.unwrap()[0].members.is_empty());

    // An unknown account or room changes nothing.
    assert!(matches!(
        s.set_room_members(id, &[alice.clone(), "f".repeat(32)])
            .await,
        Err(StoreError::NotFound)
    ));
    assert_eq!(s.rooms().await.unwrap()[1].members, want);
    assert!(matches!(
        s.set_room_members(999, std::slice::from_ref(&alice)).await,
        Err(StoreError::NotFound)
    ));

    let r = s
        .set_room_members(id, std::slice::from_ref(&alice))
        .await
        .unwrap();
    assert_eq!(r.members, [alice]);
    let r = s.set_room_members(id, &[]).await.unwrap();
    assert!(r.restricted && r.members.is_empty());
}

#[tokio::test]
async fn archive_keeps_the_room_and_never_the_last_one() {
    let (_tmp, s) = open().await;
    let rooms = s.rooms().await.unwrap();
    let (general, hangout) = (rooms[0].id, rooms[1].id);
    let r = s
        .patch_room(hangout, None, None, Some(true), 5)
        .await
        .unwrap();
    assert!(r.archived && r.name == "hangout");
    // Archiving again keeps the first time.
    s.patch_room(hangout, None, None, Some(true), 9)
        .await
        .unwrap();
    let at: Option<i64> = sqlx::query_scalar("SELECT archived_at FROM rooms WHERE id = ?1")
        .bind(hangout)
        .fetch_one(s.pool())
        .await
        .unwrap();
    assert_eq!(at, Some(5));
    // A rename leaves the flag alone.
    let r = s
        .patch_room(hangout, Some("old"), None, None, 9)
        .await
        .unwrap();
    assert!(r.archived);
    assert!(matches!(
        s.patch_room(general, None, None, Some(true), 9).await,
        Err(StoreError::Conflict(_))
    ));
    assert!(!s.rooms().await.unwrap()[0].archived);
    let r = s
        .patch_room(hangout, None, None, Some(false), 9)
        .await
        .unwrap();
    assert!(!r.archived);
    assert!(matches!(
        s.patch_room(999, None, None, Some(true), 9).await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn racing_archives_leave_one_room_open() {
    for _ in 0..10 {
        let (_tmp, s) = open().await;
        let rooms = s.rooms().await.unwrap();
        let (a, b) = (rooms[0].id, rooms[1].id);
        let (s1, s2) = (s.clone(), s.clone());
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { s1.patch_room(a, None, None, Some(true), 1).await }),
            tokio::spawn(async move { s2.patch_room(b, None, None, Some(true), 1).await }),
        );
        assert!(r1.unwrap().is_ok() ^ r2.unwrap().is_ok());
        assert_eq!(
            s.rooms()
                .await
                .unwrap()
                .iter()
                .filter(|r| !r.archived)
                .count(),
            1
        );
    }
}
