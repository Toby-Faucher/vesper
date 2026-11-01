use tempfile::TempDir;
use vesper_protocol::Role;
use vesper_store::{NewAccount, NewSession, SESSION_TTL_MS, Store, StoreError, now_ms, random_hex};

const MIN: i64 = 60 * 1000;

async fn open() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(&tmp.path().join("v.db")).await.unwrap();
    (tmp, store)
}

fn acct(username: &str) -> NewAccount<'_> {
    NewAccount {
        username,
        display_name: username,
        password_phc: None,
        role: Role::Member,
    }
}

fn sess(n: u8, now: i64) -> NewSession<'static> {
    NewSession {
        token_hash: [n; 32],
        label: "Firefox on Linux",
        now,
    }
}

fn is_hex32(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[tokio::test]
async fn fts5_is_compiled_in() {
    let (_tmp, s) = open().await;
    let used: i64 = sqlx::query_scalar("SELECT sqlite_compileoption_used('ENABLE_FTS5')")
        .fetch_one(s.pool())
        .await
        .unwrap();
    assert_eq!(used, 1, "SQLite was built without FTS5");
    sqlx::query("CREATE VIRTUAL TABLE t USING fts5(x)")
        .execute(s.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO t (x) VALUES ('hello quiet world'), ('something else')")
        .execute(s.pool())
        .await
        .unwrap();
    let hits: Vec<String> = sqlx::query_scalar("SELECT x FROM t WHERE t MATCH 'quiet'")
        .fetch_all(s.pool())
        .await
        .unwrap();
    assert_eq!(hits, ["hello quiet world"]);
}

#[tokio::test]
async fn wal_mode_is_on() {
    let (_tmp, s) = open().await;
    let mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(s.pool())
        .await
        .unwrap();
    assert_eq!(mode, "wal");
    let fk: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(s.pool())
        .await
        .unwrap();
    assert_eq!(fk, 1);
}

#[test]
fn random_hex_is_lowercase_and_sized() {
    let a = random_hex(16);
    assert!(is_hex32(&a), "{a}");
    assert_ne!(a, random_hex(16));
}

#[tokio::test]
async fn create_account_rejects_duplicate_username() {
    let (_tmp, s) = open().await;
    let u = s.create_account(acct("toby")).await.unwrap();
    assert!(is_hex32(&u.user_id), "{}", u.user_id);
    assert_eq!(u.accent, "#7c8cff");
    assert!(matches!(
        s.create_account(acct("toby")).await,
        Err(StoreError::Conflict(_))
    ));
}

#[tokio::test]
async fn invite_is_single_use_under_race() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    s.create_invite(&[1; 32], Role::Member, 1, now + 60 * MIN, None)
        .await
        .unwrap();
    let mut set = tokio::task::JoinSet::new();
    for i in 0..8u8 {
        let s = s.clone();
        set.spawn(async move {
            let name = format!("user{i}");
            s.signup(&[1; 32], acct(&name), sess(10 + i, now)).await
        });
    }
    let (mut ok, mut invalid) = (0, 0);
    while let Some(r) = set.join_next().await {
        match r.unwrap() {
            Ok(_) => ok += 1,
            Err(StoreError::InviteInvalid) => invalid += 1,
            Err(e) => panic!("unexpected {e}"),
        }
    }
    assert_eq!((ok, invalid), (1, 7));
    assert_eq!(s.users().await.unwrap().len(), 1);
}

#[tokio::test]
async fn expired_and_revoked_invites_are_invalid() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    s.create_invite(&[1; 32], Role::Member, 5, now - 1, None)
        .await
        .unwrap();
    let revoked = s
        .create_invite(&[2; 32], Role::Member, 5, now + 60 * MIN, None)
        .await
        .unwrap();
    s.revoke_invite(revoked.id, now).await.unwrap();
    assert!(s.invites().await.unwrap().iter().any(|i| i.revoked));
    for (n, h) in [[1u8; 32], [2; 32]].iter().enumerate() {
        assert_eq!(s.invite_role(h, now).await.unwrap(), None);
        let name = format!("user{n}");
        assert!(matches!(
            s.signup(h, acct(&name), sess(10 + n as u8, now)).await,
            Err(StoreError::InviteInvalid)
        ));
    }
    assert!(s.users().await.unwrap().is_empty());
}

#[tokio::test]
async fn signup_takes_role_from_invite() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let owner = s
        .create_account(NewAccount {
            role: Role::Owner,
            ..acct("owner")
        })
        .await
        .unwrap();
    assert!(s.owner_exists().await.unwrap());
    let inv = s
        .create_invite(
            &[1; 32],
            Role::Moderator,
            1,
            now + 60 * MIN,
            Some(&owner.user_id),
        )
        .await
        .unwrap();
    assert_eq!(inv.created_by.as_deref(), Some(owner.user_id.as_str()));
    assert_eq!(
        s.invite_role(&[1; 32], now).await.unwrap(),
        Some(Role::Moderator)
    );
    let (u, sid) = s.signup(&[1; 32], acct("mod"), sess(9, now)).await.unwrap();
    assert_eq!(u.role, Role::Moderator);
    let (got, user) = s.session_user(&[9; 32], now).await.unwrap().unwrap();
    assert_eq!((got, user), (sid, u));
    assert_eq!(s.invites().await.unwrap()[0].uses, 1);
}

#[tokio::test]
async fn session_expires_after_ttl_and_touch_is_hourly() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let u = s.create_account(acct("toby")).await.unwrap();
    let sid = s.create_session(&u.user_id, sess(1, now)).await.unwrap();
    let seen = |devs: Vec<vesper_protocol::Device>| devs[0].last_seen_at;

    assert!(
        s.session_user(&[1; 32], now + 30 * MIN)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(seen(s.sessions(&u.user_id, sid).await.unwrap()), now);

    let later = now + 61 * MIN;
    assert!(s.session_user(&[1; 32], later).await.unwrap().is_some());
    let devs = s.sessions(&u.user_id, sid).await.unwrap();
    assert!(devs[0].current);
    assert_eq!(seen(devs), later);

    assert!(
        s.session_user(&[1; 32], later + SESSION_TTL_MS + 1)
            .await
            .unwrap()
            .is_none()
    );
    assert!(s.session_user(&[2; 32], now).await.unwrap().is_none());
}

#[tokio::test]
async fn ban_and_password_change_drop_sessions() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let a = s.create_account(acct("alice")).await.unwrap();
    let keep = s.create_session(&a.user_id, sess(1, now)).await.unwrap();
    s.create_session(&a.user_id, sess(2, now)).await.unwrap();
    s.set_password(&a.user_id, "$argon2id$fake", keep)
        .await
        .unwrap();
    let left: Vec<i64> = s
        .sessions(&a.user_id, keep)
        .await
        .unwrap()
        .iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(left, [keep]);
    let row = s.login_row("alice").await.unwrap().unwrap();
    assert_eq!(row.password_phc.as_deref(), Some("$argon2id$fake"));

    let b = s.create_account(acct("bob")).await.unwrap();
    s.create_session(&b.user_id, sess(3, now)).await.unwrap();
    assert!(s.set_banned(&b.user_id, true, now).await.unwrap().banned);
    assert!(s.sessions(&b.user_id, 0).await.unwrap().is_empty());
    assert!(s.session_user(&[3; 32], now).await.unwrap().is_none());
    assert!(s.login_row("bob").await.unwrap().unwrap().banned);
    assert!(!s.set_banned(&b.user_id, false, now).await.unwrap().banned);
}

#[tokio::test]
async fn delete_session_of_someone_else_is_not_found() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let a = s.create_account(acct("alice")).await.unwrap();
    let b = s.create_account(acct("bob")).await.unwrap();
    let sid = s.create_session(&a.user_id, sess(1, now)).await.unwrap();
    assert!(matches!(
        s.delete_session(&b.user_id, sid).await,
        Err(StoreError::NotFound)
    ));
    s.delete_session(&a.user_id, sid).await.unwrap();
    assert!(s.session_user(&[1; 32], now).await.unwrap().is_none());
}

#[tokio::test]
async fn signup_username_conflict_rolls_back_invite_use() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    s.create_account(acct("toby")).await.unwrap();
    s.create_invite(&[1; 32], Role::Member, 2, now + 60 * MIN, None)
        .await
        .unwrap();
    assert!(matches!(
        s.signup(&[1; 32], acct("toby"), sess(9, now)).await,
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(s.invites().await.unwrap()[0].uses, 0);
    assert_eq!(s.users().await.unwrap().len(), 1);
}

#[tokio::test]
async fn session_of_banned_user_is_none() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let u = s.create_account(acct("bob")).await.unwrap();
    s.set_banned(&u.user_id, true, now).await.unwrap();
    // set_banned drops sessions; one made after the ban must still not resolve.
    s.create_session(&u.user_id, sess(1, now)).await.unwrap();
    assert!(s.session_user(&[1; 32], now).await.unwrap().is_none());
}

#[tokio::test]
async fn ui_accent_starts_null_and_persists() {
    let (_tmp, s) = open().await;
    let a = s.create_account(acct("alice")).await.unwrap();
    assert_eq!(a.ui_accent, None);
    let a2 = s
        .update_profile(&a.user_id, None, None, None, Some("teal"))
        .await
        .unwrap();
    assert_eq!(a2.ui_accent.as_deref(), Some("teal"));
    // A patch without it keeps it; the avatar colour is a different column.
    let a3 = s
        .update_profile(&a.user_id, Some("Al"), None, None, None)
        .await
        .unwrap();
    assert_eq!(
        (a3.ui_accent.as_deref(), a3.accent.as_str()),
        (Some("teal"), "#7c8cff")
    );
    // The column refuses anything longer than #rrggbb even if the server check were skipped.
    let long = s
        .update_profile(&a.user_id, None, None, None, Some("#7c8cff0"))
        .await;
    assert!(long.is_err());
}

#[tokio::test]
async fn bio_and_avatar_round_trip_and_go_with_the_account() {
    let (_tmp, s) = open().await;
    let a = s.create_account(acct("alice")).await.unwrap();
    assert_eq!((a.bio.as_str(), a.avatar.as_deref()), ("", None));
    let a2 = s
        .update_profile(&a.user_id, None, None, Some("hi there"), None)
        .await
        .unwrap();
    assert_eq!(
        (a2.bio.as_str(), a2.display_name.as_str()),
        ("hi there", "alice")
    );
    // The column refuses an over-long bio even if the server check were skipped.
    let long = "é".repeat(161);
    assert!(
        s.update_profile(&a.user_id, None, None, Some(&long), None)
            .await
            .is_err()
    );
    assert!(matches!(
        s.avatar(&a.user_id).await,
        Err(StoreError::NotFound)
    ));

    let u = s
        .set_avatar(&a.user_id, "image/png", b"one", "v1")
        .await
        .unwrap();
    assert_eq!(u.avatar.as_deref(), Some("v1"));
    let u = s
        .set_avatar(&a.user_id, "image/webp", b"two", "v2")
        .await
        .unwrap();
    assert_eq!(u.avatar.as_deref(), Some("v2"));
    let got = s.avatar(&a.user_id).await.unwrap();
    assert_eq!(got, ("image/webp".into(), b"two".to_vec(), "v2".into()));
    // Only the three picture types, and at most 512 KiB, reach the table.
    assert!(
        s.set_avatar(&a.user_id, "image/svg+xml", b"x", "v3")
            .await
            .is_err()
    );
    assert!(
        s.set_avatar(&a.user_id, "image/png", &vec![0; 524_289], "v3")
            .await
            .is_err()
    );
    assert_eq!(
        s.user(&a.user_id).await.unwrap().avatar.as_deref(),
        Some("v2")
    );
    assert!(matches!(
        s.set_avatar(&"0".repeat(32), "image/png", b"x", "v").await,
        Err(StoreError::NotFound)
    ));

    let u = s.clear_avatar(&a.user_id).await.unwrap();
    assert_eq!(u.avatar, None);
    assert!(matches!(
        s.avatar(&a.user_id).await,
        Err(StoreError::NotFound)
    ));

    // A deleted account takes its picture with it.
    s.set_avatar(&a.user_id, "image/png", b"one", "v1")
        .await
        .unwrap();
    sqlx::query("DELETE FROM sessions WHERE account_id = ?1")
        .bind(&a.user_id)
        .execute(s.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM accounts WHERE id = ?1")
        .bind(&a.user_id)
        .execute(s.pool())
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM avatars")
        .fetch_one(s.pool())
        .await
        .unwrap();
    assert_eq!(left, 0);
}
