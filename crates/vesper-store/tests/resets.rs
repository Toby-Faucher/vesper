use tempfile::TempDir;
use vesper_protocol::{ResetNotice, Role};
use vesper_store::{NewAccount, NewSession, RESET_TTL_MS, Store, StoreError, now_ms};

async fn open() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(&tmp.path().join("v.db")).await.unwrap();
    (tmp, store)
}

async fn user(s: &Store, name: &str) -> String {
    staff(s, name, Role::Member).await
}

/// An account with `role`: links only work while their maker may make them.
async fn staff(s: &Store, name: &str, role: Role) -> String {
    s.create_account(NewAccount {
        username: name,
        display_name: name,
        password_phc: Some("old"),
        role,
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

async fn sess(s: &Store, id: &str, n: u8) -> i64 {
    let ns = NewSession {
        token_hash: [n; 32],
        label: "test",
        now: now_ms(),
    };
    s.create_session(id, ns).await.unwrap()
}

#[tokio::test]
async fn reset_sets_password_and_ends_every_session_once() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let alice = staff(&s, "alice", Role::Owner).await;
    let now = now_ms();
    sess(&s, &bob, 1).await;
    sess(&s, &bob, 2).await;
    sess(&s, &alice, 3).await;
    s.create_reset(&bob, &[9; 32], now + RESET_TTL_MS, Some(&alice))
        .await
        .unwrap();
    assert!(s.reset_valid(&[9; 32], now).await.unwrap());
    assert_eq!(
        s.use_reset(&[9; 32], "new", now, "lan").await.unwrap(),
        Some((bob.clone(), Some(alice.clone())))
    );
    assert_eq!(phc(&s, "bob").await.as_deref(), Some("new"));
    assert_eq!(session_count(&s, &bob).await, 0);
    // Other accounts keep their sessions and passwords.
    assert_eq!(session_count(&s, &alice).await, 1);
    assert_eq!(phc(&s, "alice").await.as_deref(), Some("old"));
    // Single use.
    assert!(!s.reset_valid(&[9; 32], now).await.unwrap());
    assert_eq!(
        s.use_reset(&[9; 32], "again", now, "lan").await.unwrap(),
        None
    );
    assert_eq!(phc(&s, "bob").await.as_deref(), Some("new"));
    // The row stays as the audit: who made it, when it was used and from where.
    let r = s.resets(now_ms()).await.unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].user_id, bob);
    assert_eq!(r[0].created_by.as_deref(), Some(alice.as_str()));
    assert_eq!(r[0].used_at, Some(now));
    assert_eq!(r[0].used_from.as_deref(), Some("lan"));
    assert!(!r[0].cancelled);
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
        assert_eq!(s.use_reset(&hash, "new", at, "lan").await.unwrap(), None);
    }
    assert_eq!(phc(&s, "bob").await.as_deref(), Some("old"));
    // Still live just before it expires.
    assert!(s.reset_valid(&[9; 32], now + 999).await.unwrap());
}

#[tokio::test]
async fn a_new_code_cancels_the_old_one() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let alice = staff(&s, "alice", Role::Owner).await;
    let now = now_ms();
    let later = now + RESET_TTL_MS;
    s.create_reset(&bob, &[1; 32], later, None).await.unwrap();
    s.create_reset(&alice, &[3; 32], later, None).await.unwrap();
    s.create_reset(&bob, &[2; 32], later, Some(&alice))
        .await
        .unwrap();
    assert!(!s.reset_valid(&[1; 32], now).await.unwrap());
    assert_eq!(
        s.use_reset(&[1; 32], "new", now, "lan").await.unwrap(),
        None
    );
    // Another account's code is untouched.
    assert!(s.reset_valid(&[3; 32], now).await.unwrap());
    assert!(
        s.use_reset(&[2; 32], "new", now, "internet")
            .await
            .unwrap()
            .is_some()
    );
    assert!(s.reset_valid(&[3; 32], now).await.unwrap());
    // The latest per account: bob's used one, alice's open one.
    let r = s.resets(now_ms()).await.unwrap();
    assert_eq!(r.len(), 2);
    let bobs = r.iter().find(|x| x.user_id == bob).unwrap();
    assert_eq!(bobs.created_by.as_deref(), Some(alice.as_str()));
    assert!(bobs.used_at.is_some());
}

/// Who may make a link depends on the role, so a role change, a ban or unban and a password
/// change each cancel a live link.
#[tokio::test]
async fn role_ban_and_password_change_cancel_a_live_code() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let now = now_ms();
    let later = now + RESET_TTL_MS;
    let keep = sess(&s, &bob, 1).await;
    let steps: [(u8, &str); 4] = [(1, "role"), (2, "ban"), (3, "unban"), (4, "password")];
    for (n, what) in steps {
        s.create_reset(&bob, &[n; 32], later, None).await.unwrap();
        match what {
            "role" => drop(s.set_role(&bob, Role::Moderator).await.unwrap()),
            "ban" => drop(s.set_banned(&bob, true, now).await.unwrap()),
            "unban" => drop(s.set_banned(&bob, false, now).await.unwrap()),
            _ => s.set_password(&bob, "mine", keep).await.unwrap(),
        }
        assert!(!s.reset_valid(&[n; 32], now).await.unwrap(), "{what}");
        assert_eq!(
            s.use_reset(&[n; 32], "new", now, "lan").await.unwrap(),
            None,
            "{what}"
        );
        assert!(s.resets(now_ms()).await.unwrap()[0].cancelled, "{what}");
    }
    // An expired link is left as expired, not cancelled.
    s.create_reset(&bob, &[5; 32], now - 1, None).await.unwrap();
    s.set_role(&bob, Role::Member).await.unwrap();
    assert!(!s.resets(now_ms()).await.unwrap()[0].cancelled);
}

#[tokio::test]
async fn reset_notices_are_shown_once() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let alice = staff(&s, "alice", Role::Owner).await;
    let now = now_ms();
    s.create_reset(&bob, &[1; 32], now + RESET_TTL_MS, Some(&alice))
        .await
        .unwrap();
    // Not used yet: nothing to tell.
    assert!(s.take_reset_notices(&bob, now).await.unwrap().is_empty());
    s.use_reset(&[1; 32], "new", now, "lan").await.unwrap();
    s.create_reset(&bob, &[2; 32], now + RESET_TTL_MS, None)
        .await
        .unwrap();
    s.use_reset(&[2; 32], "new2", now + 1, "internet")
        .await
        .unwrap();
    assert!(s.take_reset_notices(&alice, now).await.unwrap().is_empty());
    assert_eq!(
        s.take_reset_notices(&bob, now).await.unwrap(),
        [
            ResetNotice {
                by: Some("alice".into()),
                by_username: Some("alice".into()),
                at: now
            },
            ResetNotice {
                by: None,
                by_username: None,
                at: now + 1
            },
        ]
    );
    assert!(s.take_reset_notices(&bob, now).await.unwrap().is_empty());
}

/// A demoted or banned maker's live links stop working; links made by others stay.
#[tokio::test]
async fn demoting_or_banning_the_maker_cancels_their_links() {
    let (_tmp, s) = open().await;
    let m = staff(&s, "mod", Role::Moderator).await;
    let owner = staff(&s, "owner", Role::Owner).await;
    let (a, b, c) = (
        user(&s, "a").await,
        user(&s, "b").await,
        user(&s, "c").await,
    );
    let now = now_ms();
    let later = now + RESET_TTL_MS;
    s.create_reset(&a, &[1; 32], later, Some(&m)).await.unwrap();
    s.create_reset(&c, &[3; 32], later, Some(&owner))
        .await
        .unwrap();
    s.set_role(&m, Role::Member).await.unwrap();
    assert!(!s.reset_valid(&[1; 32], now).await.unwrap());
    assert!(s.reset_valid(&[3; 32], now).await.unwrap());
    s.create_reset(&b, &[2; 32], later, Some(&m)).await.unwrap();
    s.set_banned(&m, true, now).await.unwrap();
    assert!(!s.reset_valid(&[2; 32], now).await.unwrap());
    assert_eq!(
        s.use_reset(&[2; 32], "new", now, "lan").await.unwrap(),
        None
    );
    assert!(s.reset_valid(&[3; 32], now).await.unwrap());
}

/// A newer link does not hide a used one from the owner.
#[tokio::test]
async fn used_resets_stay_listed_after_newer_links() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let m = staff(&s, "mod", Role::Moderator).await;
    let now = now_ms();
    let later = now + RESET_TTL_MS;
    s.create_reset(&bob, &[1; 32], later, Some(&m))
        .await
        .unwrap();
    s.use_reset(&[1; 32], "new", now, "internet").await.unwrap();
    s.create_reset(&bob, &[2; 32], later, Some(&m))
        .await
        .unwrap();
    let r = s.resets(now_ms()).await.unwrap();
    assert_eq!(r.len(), 2);
    assert!(r[0].used_at.is_none(), "newest first");
    assert_eq!(r[1].used_at, Some(now));
    assert_eq!(r[1].created_by.as_deref(), Some(m.as_str()));
    // Once the newer one is used too, both stay listed for 30 days, then only the latest.
    s.use_reset(&[2; 32], "new2", now + 1, "lan").await.unwrap();
    let r = s.resets(now + 1).await.unwrap();
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].used_at, Some(now + 1));
    let month = 31 * 24 * 3600 * 1000;
    assert_eq!(s.resets(now + month).await.unwrap().len(), 1);
}

/// The taker sees the notice first; a later reset shows every recent one to the real person.
#[tokio::test]
async fn a_later_reset_shows_earlier_notices_again() {
    let (_tmp, s) = open().await;
    let bob = user(&s, "bob").await;
    let m = staff(&s, "mod", Role::Moderator).await;
    let now = now_ms();
    let later = now + RESET_TTL_MS;
    s.create_reset(&bob, &[1; 32], later, Some(&m))
        .await
        .unwrap();
    s.use_reset(&[1; 32], "taker", now, "lan").await.unwrap();
    assert_eq!(s.take_reset_notices(&bob, now).await.unwrap().len(), 1);
    s.create_reset(&bob, &[2; 32], later, None).await.unwrap();
    s.use_reset(&[2; 32], "mine", now + 1, "lan").await.unwrap();
    let n = s.take_reset_notices(&bob, now + 1).await.unwrap();
    assert_eq!(n.len(), 2);
    assert_eq!(n[0].by.as_deref(), Some("mod"));
    assert_eq!(n[1].by, None);
    // A reset used more than 30 days before is not shown again.
    s.create_reset(&bob, &[3; 32], i64::MAX, None)
        .await
        .unwrap();
    let month = 31 * 24 * 3600 * 1000;
    s.use_reset(&[3; 32], "again", now + month, "lan")
        .await
        .unwrap();
    assert_eq!(
        s.take_reset_notices(&bob, now + month).await.unwrap().len(),
        1
    );
}

/// A link made in the moment between the maker check and the insert, while a ban or role change
/// committed (so the cancel missed it), still does not work: the maker is checked again on use.
#[tokio::test]
async fn a_link_whose_maker_lost_the_right_does_not_work() {
    let (_tmp, s) = open().await;
    let m = staff(&s, "mod", Role::Moderator).await;
    let owner = staff(&s, "owner", Role::Owner).await;
    let bob = user(&s, "bob").await;
    let sql = |q: &'static str, id: String| {
        let pool = s.pool().clone();
        async move { sqlx::query(q).bind(id).execute(&pool).await.unwrap() }
    };
    sql(
        "UPDATE accounts SET role = 'moderator' WHERE id = ?1",
        m.clone(),
    )
    .await;
    sql(
        "UPDATE accounts SET role = 'owner' WHERE id = ?1",
        owner.clone(),
    )
    .await;
    let now = now_ms();
    let later = now + RESET_TTL_MS;
    // Maker banned behind the store's back (no cancel ran).
    s.create_reset(&bob, &[1; 32], later, Some(&m))
        .await
        .unwrap();
    sql("UPDATE accounts SET banned_at = 1 WHERE id = ?1", m.clone()).await;
    assert_eq!(
        s.use_reset(&[1; 32], "new", now, "lan").await.unwrap(),
        None
    );
    sql(
        "UPDATE accounts SET banned_at = NULL WHERE id = ?1",
        m.clone(),
    )
    .await;
    // Maker demoted.
    s.create_reset(&bob, &[2; 32], later, Some(&m))
        .await
        .unwrap();
    sql(
        "UPDATE accounts SET role = 'member' WHERE id = ?1",
        m.clone(),
    )
    .await;
    assert_eq!(
        s.use_reset(&[2; 32], "new", now, "lan").await.unwrap(),
        None
    );
    sql(
        "UPDATE accounts SET role = 'moderator' WHERE id = ?1",
        m.clone(),
    )
    .await;
    // Target promoted.
    s.create_reset(&bob, &[3; 32], later, Some(&m))
        .await
        .unwrap();
    sql(
        "UPDATE accounts SET role = 'moderator' WHERE id = ?1",
        bob.clone(),
    )
    .await;
    assert_eq!(
        s.use_reset(&[3; 32], "new", now, "lan").await.unwrap(),
        None
    );
    // The owner's link for a moderator, and a command line link, still work.
    s.create_reset(&bob, &[4; 32], later, Some(&owner))
        .await
        .unwrap();
    assert!(
        s.use_reset(&[4; 32], "new", now, "lan")
            .await
            .unwrap()
            .is_some()
    );
    s.create_reset(&bob, &[5; 32], later, None).await.unwrap();
    assert!(
        s.use_reset(&[5; 32], "new", now, "lan")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(phc(&s, "bob").await.as_deref(), Some("new"));
}

#[tokio::test]
async fn reset_for_unknown_account_is_not_found() {
    let (_tmp, s) = open().await;
    let r = s
        .create_reset(&"0".repeat(32), &[1; 32], now_ms() + 1, None)
        .await;
    assert!(matches!(r, Err(StoreError::NotFound)), "{r:?}");
}
