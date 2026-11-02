//! Your recent calls: only ever your own, for every role; values checked; the post limited.

mod common;

use axum::http::{Method, StatusCode};
use common::{TestServer, get, post, request};
use serde_json::{Value, json};
use vesper_protocol::Role;

fn summary(duration_s: u32) -> Value {
    json!({
        "duration_s": duration_s, "mode": "deepfilter", "fallback_reason": "", "loss_pct": 0.5,
        "jitter_ms": 8.0, "concealed_pct": 0.2, "rtt_ms": 35.0, "browser_ns_loading": true,
    })
}

#[tokio::test]
async fn nobody_sees_anyone_elses_calls_whatever_the_role() {
    let server = TestServer::new().await;
    let people = [
        server.account("olive", Role::Owner).await,
        server.account("mona", Role::Moderator).await,
        server.account("mel", Role::Member).await,
        server.account("max", Role::Member).await,
    ];
    // Each posts a call of its own length (1..4 s), and one more naming another account.
    for (i, p) in people.iter().enumerate() {
        let mut s = summary(i as u32 + 1);
        let (st, _, _) = server.call(post("/api/me/calls", s.clone(), p)).await;
        assert_eq!(st, StatusCode::NO_CONTENT);
        let other = &people[(i + 1) % people.len()].user.user_id;
        s["account_id"] = json!(other);
        s["user_id"] = json!(other);
        assert_eq!(
            server.call(post("/api/me/calls", s, p)).await.0,
            StatusCode::NO_CONTENT
        );
    }
    for (i, p) in people.iter().enumerate() {
        let (st, _, v) = server.call(get("/api/me/calls", p)).await;
        assert_eq!(st, StatusCode::OK);
        let durations: Vec<u64> = v
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["duration_s"].as_u64().unwrap())
            .collect();
        assert_eq!(
            durations,
            [i as u64 + 1; 2],
            "{} sees calls that are not theirs",
            p.user.username
        );
        assert_eq!(v[0]["mode"], "deepfilter");
        assert_eq!(v[0]["browser_ns_loading"], true);
        assert!(v[0].get("account_id").is_none());
    }
    // Signed out: nothing.
    let (st, _, _) = server
        .call(request(Method::GET, "/api/me/calls", None, None))
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
    let (st, _, _) = server
        .call(request(
            Method::POST,
            "/api/me/calls",
            Some(&summary(1)),
            None,
        ))
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn values_out_of_range_are_refused_and_unknown_fields_ignored() {
    let server = TestServer::new().await;
    // The limit (10 a burst) counts refused posts too: spread them over accounts.
    let senders = [server.member("mel").await, server.member("max").await];
    let me = server.member("may").await;
    let with = |k: &str, v: Value| {
        let mut s = summary(10);
        s[k] = v;
        s
    };
    let refused = [
        with("duration_s", json!(24 * 3600 + 1)),
        with("duration_s", json!(-1)),
        with("mode", json!("rnnoise")),
        with("fallback_reason", json!("x".repeat(121))),
        with("fallback_reason", json!("line\nbreak")),
        with("loss_pct", json!(100.5)),
        with("loss_pct", json!(-0.1)),
        with("concealed_pct", json!(101)),
        with("jitter_ms", json!(10_001)),
        with("rtt_ms", json!(60_001)),
        with("rtt_ms", json!("fast")),
        json!({ "duration_s": 10 }),
    ];
    for (i, s) in refused.into_iter().enumerate() {
        let (st, _, v) = server
            .call(post("/api/me/calls", s.clone(), &senders[i % 2]))
            .await;
        assert_eq!(st, StatusCode::BAD_REQUEST, "{s} was taken: {v}");
    }
    let ok = [
        with("fallback_reason", json!("é".repeat(120))),
        with("loss_pct", Value::Null),
        with("extra", json!({ "nested": [1, 2, 3] })),
        with("rtt_ms", json!(60_000)),
    ];
    for s in ok {
        let (st, _, v) = server.call(post("/api/me/calls", s.clone(), &me)).await;
        assert_eq!(st, StatusCode::NO_CONTENT, "{s} was refused: {v}");
    }
    let (_, _, v) = server.call(get("/api/me/calls", &me)).await;
    assert_eq!(v.as_array().unwrap().len(), 4);
    assert!(v[0].get("extra").is_none());
}

#[tokio::test]
async fn the_post_is_rate_limited() {
    let server = TestServer::new().await;
    let me = server.member("mel").await;
    for _ in 0..10 {
        assert_eq!(
            server.call(post("/api/me/calls", summary(1), &me)).await.0,
            StatusCode::NO_CONTENT
        );
    }
    let (st, h, _) = server.call(post("/api/me/calls", summary(1), &me)).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    assert!(h.contains_key("retry-after"));
    // Another account has its own budget.
    let other = server.member("max").await;
    assert_eq!(
        server
            .call(post("/api/me/calls", summary(1), &other))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
}
