//! Test server: the `/api` router driven in process with `oneshot`, plus the whole app (with a
//! real SFU) on `addr` for socket tests. The data dir is removed when the server is dropped.
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
use vesper_protocol::{Role, UserInfo};
use vesper_server::config::Config;
use vesper_server::hub::Hub;
use vesper_server::limits::Limiter;
use vesper_server::voice::Voice;
use vesper_server::{AppState, cli, router};
use vesper_sfu::{Sfu, SfuConfig, SfuHandle};
use vesper_store::{NewAccount, NewSession, Store, now_ms};

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
        max_room_size: 12,
        ping_interval: Duration::from_secs(30),
        max_sockets: 16,
        web_dir: "does-not-exist".into(),
        data_dir: data.path().to_path_buf(),
        origin: ORIGIN.into(),
        secure_cookies: false,
        upload_quota_bytes: 1 << 30,
    }
}

impl TestServer {
    pub async fn new() -> TestServer {
        TestServer::with(|_| {}, spawn_sfu()).await
    }

    /// A server with `edit` applied to the test config, on an SFU the caller spawned.
    pub async fn with(edit: impl FnOnce(&mut Config), sfu: SfuHandle) -> TestServer {
        let data = TempDir::new().unwrap();
        let mut cfg = config(&data);
        edit(&mut cfg);
        let store = Store::open(&cfg.data_dir.join("vesper.db")).await.unwrap();
        let hub = Hub::default();
        let state = AppState {
            store,
            voice: Voice::new(sfu.clone(), hub.clone(), cfg.max_room_size),
            cfg: Arc::new(cfg),
            limits: Limiter::default(),
            hub,
        };
        let (addr, _serving) = vesper_server::serve(state.clone(), sfu).await.unwrap();
        TestServer { addr, state, data }
    }

    /// An account with a session, made in the store directly: no invite and no argon2, so tests
    /// that need many users stay fast. It has no password, so it cannot log in.
    pub async fn account(&self, username: &str, role: Role) -> Session {
        let user = self
            .state
            .store
            .create_account(NewAccount {
                username,
                display_name: username,
                password_phc: None,
                role,
            })
            .await
            .unwrap();
        self.session(&user).await
    }

    /// Another session (device) for `user`, made in the store directly.
    pub async fn session(&self, user: &UserInfo) -> Session {
        let (token, token_hash) = vesper_server::auth::new_token();
        let s = NewSession {
            token_hash,
            label: "test",
            now: now_ms(),
        };
        self.state
            .store
            .create_session(&user.user_id, s)
            .await
            .unwrap();
        Session {
            cookie: format!("vesper_session={token}"),
            user: user.clone(),
        }
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

pub fn spawn_sfu() -> SfuHandle {
    Sfu::spawn(SfuConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        ..Default::default()
    })
    .unwrap()
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
