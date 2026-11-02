use tempfile::TempDir;
use vesper_protocol::{CallMode, CallSummary, Role};
use vesper_store::{CALLS_KEPT, CALLS_TTL_MS, NewAccount, Store};

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

fn call(duration_s: u32) -> CallSummary {
    CallSummary {
        duration_s,
        mode: CallMode::Browser,
        fallback_reason: "denoiser stalled".into(),
        loss_pct: Some(1.5),
        jitter_ms: Some(12.0),
        concealed_pct: None,
        rtt_ms: Some(40.0),
        browser_ns_loading: true,
    }
}

#[tokio::test]
async fn each_account_reads_only_its_own_calls_newest_first() {
    let (_t, s) = open().await;
    let a = account(&s, "ann").await;
    let b = account(&s, "ben").await;
    s.add_call(&a, &call(60), 1_000_000).await.unwrap();
    s.add_call(&a, &call(120), 2_000_000).await.unwrap();
    s.add_call(&b, &call(5), 3_000_000).await.unwrap();
    let mine = s.calls_of(&a).await.unwrap();
    assert_eq!(mine.len(), 2);
    assert_eq!(mine[0].summary, call(120));
    assert_eq!(
        (mine[0].started_at, mine[0].ended_at),
        (1_880_000, 2_000_000)
    );
    assert_eq!(s.calls_of(&b).await.unwrap().len(), 1);
    assert!(
        s.calls_of("0".repeat(32).as_str())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn keeps_the_newest_20_per_account() {
    let (_t, s) = open().await;
    let a = account(&s, "ann").await;
    let b = account(&s, "ben").await;
    s.add_call(&b, &call(1), 1).await.unwrap();
    for i in 0..CALLS_KEPT + 5 {
        s.add_call(&a, &call(i as u32), 1_000_000 + i)
            .await
            .unwrap();
    }
    let mine = s.calls_of(&a).await.unwrap();
    assert_eq!(mine.len() as i64, CALLS_KEPT);
    assert_eq!(mine[0].summary.duration_s as i64, CALLS_KEPT + 4);
    assert_eq!(mine.last().unwrap().summary.duration_s, 5);
    assert_eq!(
        s.calls_of(&b).await.unwrap().len(),
        1,
        "another account's trim took my rows"
    );
}

#[tokio::test]
async fn prune_drops_calls_older_than_30_days_only() {
    let (_t, s) = open().await;
    let a = account(&s, "ann").await;
    let now = 100 * CALLS_TTL_MS;
    s.add_call(&a, &call(10), now - CALLS_TTL_MS - 1)
        .await
        .unwrap();
    s.add_call(&a, &call(20), now - CALLS_TTL_MS + 1)
        .await
        .unwrap();
    assert_eq!(s.prune_calls(now - CALLS_TTL_MS).await.unwrap(), 1);
    let left = s.calls_of(&a).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].summary.duration_s, 20);
}

#[tokio::test]
async fn the_table_refuses_values_out_of_range() {
    let (_t, s) = open().await;
    let a = account(&s, "ann").await;
    let bad = CallSummary {
        loss_pct: Some(101.0),
        ..call(1)
    };
    assert!(s.add_call(&a, &bad, 1).await.is_err());
    let long = CallSummary {
        fallback_reason: "x".repeat(121),
        ..call(1)
    };
    assert!(s.add_call(&a, &long, 1).await.is_err());
    assert!(s.calls_of(&a).await.unwrap().is_empty());
}
