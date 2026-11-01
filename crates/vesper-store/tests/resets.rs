use tempfile::TempDir;
use vesper_protocol::Role;
use vesper_store::{NewAccount, NewSession, RESET_TTL_MS, Store, StoreError, now_ms};

async fn open() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(&tmp.path().join("v.db")).await.unwrap();
    (tmp, store)
}

async fn user(s: &Store, name: &str) -> String {
    s.create_account(NewAccount {
        username: name,
        display_name: name,
        password_phc: Some("old"),
        role: Role::Member,
    })
    .await
    .unwrap()
    .user_id
}

async fn phc(s: &Store, name: &str) -> Option<String> {
    s.login_row(name).await.unwrap().unwrap().password_phc
}

async fn session_count(s: &Store, id: &str) -> usize {
    s.sessions(id, 0).await.unwrap().len()
}

#[tokio::test]
async fn reset_sets_password_and_ends_every_session_once() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let alice = user(&s, "alice").await;
    let now = now_ms();
    for (n, id) in [(1u8, &bob), (2, &bob), (3, &alice)] {
        let sess = NewSession {
            token_hash: [n; 32],
            label: "test",
            now,
        };
        s.create_session(id, sess).await.unwrap();
    }
    s.create_reset(&bob, &[9; 32], now + RESET_TTL_MS, None)
        .await
        .unwrap();
    assert!(s.reset_valid(&[9; 32], now).await.unwrap());
    assert_eq!(
        s.use_reset(&[9; 32], "new", now).await.unwrap(),
        Some(bob.clone())
    );
    assert_eq!(phc(&s, "bob").await.as_deref(), Some("new"));
    assert_eq!(session_count(&s, &bob).await, 0);
    // Other accounts keep their sessions and passwords.
    assert_eq!(session_count(&s, &alice).await, 1);
    assert_eq!(phc(&s, "alice").await.as_deref(), Some("old"));
    // Single use.
    assert!(!s.reset_valid(&[9; 32], now).await.unwrap());
    assert_eq!(s.use_reset(&[9; 32], "again", now).await.unwrap(), None);
    assert_eq!(phc(&s, "bob").await.as_deref(), Some("new"));
}

#[tokio::test]
async fn expired_and_unknown_codes_do_nothing() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let now = now_ms();
    s.create_reset(&bob, &[9; 32], now + 1000, None)
        .await
        .unwrap();
    for (hash, at) in [([9; 32], now + 1000), ([9; 32], now + 5000), ([8; 32], now)] {
        assert!(!s.reset_valid(&hash, at).await.unwrap());
        assert_eq!(s.use_reset(&hash, "new", at).await.unwrap(), None);
    }
    assert_eq!(phc(&s, "bob").await.as_deref(), Some("old"));
    // Still live just before it expires.
    assert!(s.reset_valid(&[9; 32], now + 999).await.unwrap());
}

#[tokio::test]
async fn a_new_code_replaces_the_old_one() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let alice = user(&s, "alice").await;
    let now = now_ms();
    let later = now + RESET_TTL_MS;
    s.create_reset(&bob, &[1; 32], later, None).await.unwrap();
    s.create_reset(&alice, &[3; 32], later, None).await.unwrap();
    s.create_reset(&bob, &[2; 32], later, Some(&alice))
        .await
        .unwrap();
    assert!(!s.reset_valid(&[1; 32], now).await.unwrap());
    assert_eq!(s.use_reset(&[1; 32], "new", now).await.unwrap(), None);
    // Another account's code is untouched.
    assert!(s.reset_valid(&[3; 32], now).await.unwrap());
    assert_eq!(s.use_reset(&[2; 32], "new", now).await.unwrap(), Some(bob));
    assert!(s.reset_valid(&[3; 32], now).await.unwrap());
}

#[tokio::test]
async fn reset_for_unknown_account_is_not_found() {
    let (_tmp, s) = open().await;
    let r = s
        .create_reset(&"0".repeat(32), &[1; 32], now_ms() + 1, None)
        .await;
    assert!(matches!(r, Err(StoreError::NotFound)), "{r:?}");
}
