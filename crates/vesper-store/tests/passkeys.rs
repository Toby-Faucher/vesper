use tempfile::TempDir;
use vesper_protocol::Role;
use vesper_store::{Conflict, MAX_PASSKEYS, NewAccount, Store, StoreError};

async fn open() -> (TempDir, Store, String) {
    let tmp = TempDir::new().unwrap();
    let s = Store::open(&tmp.path().join("v.db")).await.unwrap();
    let id = s
        .create_account(NewAccount {
            username: "alice",
            display_name: "Alice",
            password_phc: None,
            role: Role::Member,
        })
        .await
        .unwrap()
        .user_id;
    (tmp, s, id)
}

#[tokio::test]
async fn add_list_rename_delete_and_caps() {
    let (_t, s, id) = open().await;
    let pk = s
        .add_passkey(&id, b"cred-1", "{}", "Laptop", 10)
        .await
        .unwrap();
    assert!(matches!(
        s.add_passkey(&id, b"cred-1", "{}", "Again", 11).await,
        Err(StoreError::Conflict(Conflict::PasskeyExists))
    ));
    let list = s.passkeys(&id).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Laptop");
    assert_eq!(list[0].last_used_at, None);

    s.rename_passkey(&id, pk, "Phone").await.unwrap();
    s.passkey_used(pk, "{\"n\":2}", 20).await.unwrap();
    let list = s.passkeys(&id).await.unwrap();
    assert_eq!(
        (list[0].name.as_str(), list[0].last_used_at),
        ("Phone", Some(20))
    );
    let found = s.passkey_by_credential(b"cred-1").await.unwrap().unwrap();
    assert_eq!(
        (found.account_id.as_str(), found.banned),
        (id.as_str(), false)
    );
    assert_eq!(found.credential, "{\"n\":2}");

    // Another account cannot touch it.
    assert!(matches!(
        s.delete_passkey("0".repeat(32).as_str(), pk).await,
        Err(StoreError::NotFound)
    ));
    s.delete_passkey(&id, pk).await.unwrap();
    assert!(s.passkey_by_credential(b"cred-1").await.unwrap().is_none());

    for i in 0..MAX_PASSKEYS {
        s.add_passkey(&id, format!("c{i}").as_bytes(), "{}", "k", 1)
            .await
            .unwrap();
    }
    assert!(matches!(
        s.add_passkey(&id, b"one-too-many", "{}", "k", 1).await,
        Err(StoreError::Conflict(Conflict::TooManyPasskeys))
    ));
}

#[tokio::test]
async fn ban_shows_on_the_row() {
    let (_t, s, id) = open().await;
    s.add_passkey(&id, b"c", "{}", "k", 1).await.unwrap();
    s.set_banned(&id, true, 5).await.unwrap();
    assert!(s.passkey_by_credential(b"c").await.unwrap().unwrap().banned);
}
