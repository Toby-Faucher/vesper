//! The upload speed test: a raw body is read and thrown away.

mod common;

use axum::body::{Body, Bytes};
use axum::http::{Method, Request, StatusCode, header};
use common::{ORIGIN, Session, TestServer};
use vesper_protocol::Role;

fn req(body: Body, s: Option<&Session>, origin: bool) -> Request<Body> {
    let mut b = Request::builder()
        .method(Method::POST)
        .uri("/api/speedtest");
    if origin {
        b = b.header(header::ORIGIN, ORIGIN);
    }
    if let Some(s) = s {
        b = b.header(header::COOKIE, &s.cookie);
    }
    b.body(body).unwrap()
}

fn walk(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |d| {
        d.map(|e| e.unwrap().path())
            .map(|p| if p.is_dir() { walk(&p) } else { 1 })
            .sum()
    })
}

fn mib(n: usize) -> Body {
    Body::from(vec![7u8; n << 20])
}

#[tokio::test]
async fn speedtest_accepts_and_discards() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let (status, _, _) = server.call(req(mib(4), Some(&alice), true)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    // The server keeps an empty uploads/tmp; count files only.
    assert_eq!(walk(&server.data.path().join("uploads")), 0);
}

#[tokio::test]
async fn speedtest_needs_a_session() {
    let server = TestServer::new().await;
    let (status, _, _) = server.call(req(mib(1), None, true)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn speedtest_needs_the_origin() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let (status, _, _) = server.call(req(mib(1), Some(&alice), false)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn speedtest_is_rate_limited() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    for _ in 0..3 {
        let (status, _, _) = server.call(req(mib(1), Some(&alice), true)).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let (status, _, _) = server.call(req(mib(1), Some(&alice), true)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn speedtest_refuses_over_25_mib() {
    let server = TestServer::new().await;
    let alice = server.account("alice", Role::Member).await;
    let m = Bytes::from(vec![7u8; 1 << 20]);
    let chunks: Vec<Result<Bytes, std::io::Error>> = (0..26).map(|_| Ok(m.clone())).collect();
    let body = Body::from_stream(futures_util::stream::iter(chunks));
    let (status, _, v) = server.call(req(body, Some(&alice), true)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{v}");
}
