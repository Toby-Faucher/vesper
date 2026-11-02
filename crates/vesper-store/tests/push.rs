use tempfile::TempDir;
use vesper_protocol::Role;
use vesper_store::{
    MAX_PUSH_DEVICES, NewAccount, NewPushSub, NewSession, PushKind, SESSION_TTL_MS, Store,
    StoreError, now_ms,
};

async fn open() -> (TempDir, Store) {
    let tmp = TempDir::new().unwrap();
    let store = Store::open(&tmp.path().join("v.db")).await.unwrap();
    (tmp, store)
}

async fn account(s: &Store, name: &str) -> String {
    s.create_account(NewAccount {
        username: name,
        display_name: name,
        password_phc: None,
        role: Role::Member,
    })
    .await
    .unwrap()
    .user_id
}

async fn session(s: &Store, user: &str, n: u8, now: i64) -> i64 {
    s.create_session(
        user,
        NewSession {
            token_hash: [n; 32],
            label: "Chrome on Linux",
            now,
        },
    )
    .await
    .unwrap()
}

fn sub<'a>(account_id: &'a str, session_id: i64, endpoint: &'a str) -> NewPushSub<'a> {
    NewPushSub {
        account_id,
        session_id,
        endpoint,
        p256dh: &[4; 65],
        auth: &[1; 16],
        mentions: true,
        voice: false,
        user_agent_hint: "Chrome on Linux",
        now: now_ms(),
    }
}

#[tokio::test]
async fn secret_is_made_once() {
    let (_tmp, s) = open().await;
    let a = s
        .secret_or_insert("vapid", || vec![1, 2, 3], 1)
        .await
        .unwrap();
    let b = s.secret_or_insert("vapid", || vec![9], 2).await.unwrap();
    assert_eq!(a, vec![1, 2, 3]);
    assert_eq!(b, a);
}

#[tokio::test]
async fn one_subscription_per_session_and_endpoint() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let alice = account(&s, "alice").await;
    let bob = account(&s, "bob").await;
    let sa = session(&s, &alice, 1, now).await;
    let sb = session(&s, &bob, 2, now).await;
    s.put_push_subscription(sub(&alice, sa, "https://a/1"))
        .await
        .unwrap();
    // Same session again: replaced, not added.
    s.put_push_subscription(sub(&alice, sa, "https://a/2"))
        .await
        .unwrap();
    assert_eq!(s.push_devices(&alice, sa).await.unwrap().len(), 1);
    // The same browser endpoint signed in as bob moves to bob.
    s.put_push_subscription(sub(&bob, sb, "https://a/2"))
        .await
        .unwrap();
    assert!(s.push_devices(&alice, sa).await.unwrap().is_empty());
    let d = s.push_devices(&bob, sb).await.unwrap();
    assert_eq!(d.len(), 1);
    assert!(d[0].current && d[0].mentions && !d[0].voice);
    // A session that is not the account's is refused.
    assert!(matches!(
        s.put_push_subscription(sub(&alice, sb, "https://a/3"))
            .await,
        Err(StoreError::NotFound)
    ));
}

#[tokio::test]
async fn devices_are_capped() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let alice = account(&s, "alice").await;
    for n in 0..MAX_PUSH_DEVICES as u8 {
        let sid = session(&s, &alice, n, now).await;
        s.put_push_subscription(sub(&alice, sid, &format!("https://a/{n}")))
            .await
            .unwrap();
    }
    let extra = session(&s, &alice, 99, now).await;
    assert!(matches!(
        s.put_push_subscription(sub(&alice, extra, "https://a/x"))
            .await,
        Err(StoreError::Conflict(_))
    ));
}

#[tokio::test]
async fn logout_ban_and_password_change_drop_subscriptions() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let alice = account(&s, "alice").await;
    let s1 = session(&s, &alice, 1, now).await;
    let s2 = session(&s, &alice, 2, now).await;
    let s3 = session(&s, &alice, 3, now).await;
    for (sid, e) in [
        (s1, "https://a/1"),
        (s2, "https://a/2"),
        (s3, "https://a/3"),
    ] {
        s.put_push_subscription(sub(&alice, sid, e)).await.unwrap();
    }
    s.delete_session(&alice, s1).await.unwrap();
    assert_eq!(s.push_devices(&alice, 0).await.unwrap().len(), 2);
    s.set_password(&alice, "phc", s2).await.unwrap();
    let left = s.push_devices(&alice, s2).await.unwrap();
    assert_eq!(left.len(), 1);
    assert!(left[0].current);
    s.set_banned(&alice, true, now).await.unwrap();
    assert!(s.push_devices(&alice, 0).await.unwrap().is_empty());
}

#[tokio::test]
async fn targets_follow_switches_bans_and_session_age() {
    let (_tmp, s) = open().await;
    let now = now_ms();
    let alice = account(&s, "alice").await;
    let bob = account(&s, "bob").await;
    let carol = account(&s, "carol").await;
    let sa = session(&s, &alice, 1, now).await;
    let sb = session(&s, &bob, 2, now).await;
    // Carol's session was last used past the session lifetime: it is dead.
    let sc = session(&s, &carol, 3, now - SESSION_TTL_MS - 1).await;
    s.put_push_subscription(sub(&alice, sa, "https://a/a"))
        .await
        .unwrap();
    s.put_push_subscription(NewPushSub {
        mentions: false,
        voice: true,
        ..sub(&bob, sb, "https://a/b")
    })
    .await
    .unwrap();
    s.put_push_subscription(sub(&carol, sc, "https://a/c"))
        .await
        .unwrap();
    let all = [alice.clone(), bob.clone(), carol.clone()];
    let who = |t: Vec<vesper_store::PushTarget>| -> Vec<String> {
        t.into_iter().map(|t| t.account_id).collect()
    };
    assert_eq!(
        who(s.push_targets(&all, PushKind::Mentions, now).await.unwrap()),
        std::slice::from_ref(&alice)
    );
    assert_eq!(
        who(s.push_targets(&all, PushKind::Voice, now).await.unwrap()),
        std::slice::from_ref(&bob)
    );
    assert!(
        s.push_targets(&[], PushKind::Voice, now)
            .await
            .unwrap()
            .is_empty()
    );
    let t = s.push_target_of_session(sa).await.unwrap().unwrap();
    assert_eq!(t.endpoint, "https://a/a");
    assert_eq!((t.p256dh.len(), t.auth.len()), (65, 16));
    assert!(!format!("{t:?}").contains("https://a/a"));
    // Removal: only by the owner; 410 removal by id; delivery stamps last_ok_at.
    assert!(matches!(
        s.delete_push_device(&bob, t.id).await,
        Err(StoreError::NotFound)
    ));
    s.push_delivered(t.id, 42).await.unwrap();
    assert_eq!(
        s.push_devices(&alice, sa).await.unwrap()[0].last_ok_at,
        Some(42)
    );
    s.delete_push_subscription(t.id).await.unwrap();
    assert!(s.push_target_of_session(sa).await.unwrap().is_none());
    s.delete_push_of_session(sb).await.unwrap();
    assert!(s.push_devices(&bob, sb).await.unwrap().is_empty());
}
