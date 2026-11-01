//! Test server for the `/api` routes: the router driven in process with `oneshot`, plus a real
//! listener on `addr` for socket tests.
#![allow(dead_code)] // each test binary uses a different part

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use vesper_protocol::UserInfo;
use vesper_server::config::Config;
use vesper_server::limits::Limiter;
use vesper_server::{AppState, cli, router};
use vesper_store::Store;

pub const ORIGIN: &str = "http://localhost:8080";
pub const PASSWORD: &str = "test-password-1";

pub struct TestServer {
    pub addr: SocketAddr,
    pub state: AppState,
    pub data: TempDir,
}

pub struct Session {
    /// `name=value`, ready for a `Cookie` header.
    pub cookie: String,
    pub user: UserInfo,
}

pub fn config(data: &TempDir) -> Config {
    Config {
        http_bind: "127.0.0.1:0".parse().unwrap(),
        udp_bind: "127.0.0.1:0".parse().unwrap(),
        public_host: None,
        udp_port_public: 0,
        dev_key: "k".into(),
        max_room_size: 12,
        join_deadline: Duration::from_secs(30),
        ping_interval: Duration::from_secs(30),
        max_sockets: 16,
        web_dir: "does-not-exist".into(),
        data_dir: data.path().to_path_buf(),
        origin: ORIGIN.into(),
        secure_cookies: false,
        upload_quota_bytes: 0,
    }
}

impl TestServer {
    pub async fn new() -> TestServer {
        let data = TempDir::new().unwrap();
        let cfg = config(&data);
        let store = Store::open(&cfg.data_dir.join("vesper.db")).await.unwrap();
        let state = AppState {
            store,
            cfg: Arc::new(cfg),
            limits: Limiter::default(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
        tokio::spawn(async move { axum::serve(listener, app).await });
        TestServer { addr, state, data }
    }

    /// One request through the router. The peer is loopback, so `X-Forwarded-For` is trusted.
    /// The body is `Value::Null` when empty.
    pub async fn call(&self, mut req: Request<Body>) -> (StatusCode, HeaderMap, Value) {
        if req.extensions().get::<ConnectInfo<SocketAddr>>().is_none() {
            req.extensions_mut()
                .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        }
        let res = router(self.state.clone()).oneshot(req).await.unwrap();
        let (parts, body) = res.into_parts();
        let bytes = body.collect().await.unwrap().to_bytes();
        let v = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
        };
        (parts.status, parts.headers, v)
    }

    /// A fresh invite code from the CLI (`args` after `invite`, or `create-owner`).
    pub async fn invite(&self, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let link = cli::run(&args, &self.state.cfg).await.unwrap().unwrap();
        link.split_once('#').unwrap().1.to_string()
    }

    /// Signs up `username` with `PASSWORD`; panics unless it is 201.
    pub async fn signup(&self, code: &str, username: &str) -> Session {
        let body = json!({
            "invite": code, "username": username, "password": PASSWORD, "display_name": username,
        });
        let (status, h, v) = self
            .call(request(Method::POST, "/api/signup", Some(&body), None))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{v}");
        session(&h, &v["user"])
    }

    pub async fn owner(&self) -> Session {
        let code = self.invite(&["create-owner"]).await;
        self.signup(&code, "owner").await
    }

    pub async fn member(&self, name: &str) -> Session {
        let code = self.invite(&["invite"]).await;
        self.signup(&code, name).await
    }

    /// Logs in with `PASSWORD`; panics unless it is 200.
    pub async fn login(&self, username: &str) -> Session {
        let body = json!({ "username": username, "password": PASSWORD });
        let (status, h, v) = self
            .call(request(Method::POST, "/api/login", Some(&body), None))
            .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        session(&h, &v["user"])
    }
}

/// The `name=value` part of the response's `Set-Cookie`.
pub fn set_cookie(h: &HeaderMap) -> String {
    let v = h
        .get(header::SET_COOKIE)
        .expect("Set-Cookie")
        .to_str()
        .unwrap();
    v.split(';').next().unwrap().to_string()
}

fn session(h: &HeaderMap, user: &Value) -> Session {
    Session {
        cookie: set_cookie(h),
        user: serde_json::from_value(user.clone()).unwrap(),
    }
}

/// A request from our own origin, with an optional JSON body and cookie.
pub fn request(
    method: Method,
    path: &str,
    body: Option<&Value>,
    cookie: Option<&str>,
) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::ORIGIN, ORIGIN);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap(),
        None => b.body(Body::empty()).unwrap(),
    }
}

pub fn post(path: &str, json: Value, s: &Session) -> Request<Body> {
    request(Method::POST, path, Some(&json), Some(&s.cookie))
}

pub fn get(path: &str, s: &Session) -> Request<Body> {
    request(Method::GET, path, None, Some(&s.cookie))
}
