//! HTTP and the WebSocket, on one tokio thread. The UI sends `Cmd`s and gets `Event`s back; it owns all state.
//!
//! Like a browser on the server's own page: the session cookie lives in a cookie jar, and every request and the
//! socket upgrade send `Origin` equal to the server's origin, which the server checks (ADR 0007).

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{COOKIE, HeaderMap, HeaderValue, ORIGIN};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{self, Message as WsMsg, client::IntoClientRequest};
use url::Url;
use vesper_protocol::{
    ApiError, ApiErrorCode, CLOSE_DISCONNECTED, CLOSE_SESSION_ENDED, ClientMsg, LoginReq, Me,
    Message, MessagePage, ReadReq, Secret, SendMessage, ServerMsg,
};

use crate::store::{Saved, SessionStore};
use crate::voice::{AudioDevices, Call, VoiceCmd, VoiceEvent};

/// From the UI to the network thread.
#[derive(Debug)]
pub enum Cmd {
    /// Sign in with the session kept in the store, if there is one.
    Restore,
    Login {
        server: String,
        username: String,
        password: Secret,
    },
    /// `POST /api/logout`, then forget the stored session.
    Logout,
    /// The server ended the session: close the socket and forget it.
    Forget,
    LoadLatest(i64),
    LoadOlder {
        room: i64,
        before: i64,
    },
    /// The newest message id of each DM, to sort them (`hello` has no activity).
    SeedActivity(Vec<i64>),
    Send {
        room: i64,
        body: String,
    },
    MarkRead {
        room: i64,
        id: i64,
    },
    /// To the call engine. `Start` starts a call, ending the one before.
    Voice(VoiceCmd),
    /// A message on the socket. Dropped while it is down.
    Signal(ClientMsg),
}

/// From the network thread to the UI.
#[derive(Debug)]
pub enum Event {
    SignedIn {
        me: Box<Me>,
        server: String,
    },
    /// Not signed in (at start, or after sign out). `server` fills the address field; `notice` says why.
    SignedOut {
        server: Option<String>,
        notice: String,
    },
    SignInFailed(String),
    /// No keyring: signed in, but not remembered. Sent at most once per run.
    NotRemembered,
    Server(Box<ServerMsg>),
    /// The socket dropped; the next try starts after `retry_in`.
    Offline {
        retry_in: Duration,
    },
    /// A 401, or the socket closed with 4001.
    SessionEnded,
    /// A moderator disconnected you (4003). No reconnect.
    Disconnected,
    Latest {
        room: i64,
        page: MessagePage,
    },
    Older {
        room: i64,
        page: MessagePage,
    },
    LoadFailed {
        room: i64,
        error: String,
    },
    Activity(Vec<(i64, i64)>),
    Sent(Box<Message>),
    SendFailed {
        room: i64,
        body: String,
        error: String,
    },
    ReadFailed {
        room: i64,
    },
    /// From the call started with `VoiceCmd::Start { call, .. }`.
    Voice {
        call: u64,
        ev: VoiceEvent,
    },
}

/// Why a request failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fail {
    Api {
        status: u16,
        code: ApiErrorCode,
    },
    /// No answer: wrong address, server down, no network.
    Network,
    /// An answer that is not from a Vesper server.
    NotVesper,
}

impl Fail {
    pub fn unauthorized(self) -> bool {
        matches!(self, Fail::Api { status: 401, .. })
    }

    /// Plain words for the error, the same texts as `web/src/lib/api.ts`.
    pub fn text(self) -> &'static str {
        let code = match self {
            Fail::Network => {
                return "Could not reach the server. Check the address and your connection.";
            }
            Fail::NotVesper => return "That address does not answer like a Vesper server.",
            Fail::Api { code, .. } => code,
        };
        match code {
            ApiErrorCode::BadRequest => "The server did not accept that. Check what you typed.",
            ApiErrorCode::Unauthorized => "You are signed out. Sign in again.",
            ApiErrorCode::Forbidden => "You are not allowed to do that.",
            ApiErrorCode::NotFound => "That was not found.",
            ApiErrorCode::Conflict => "That name is already taken.",
            ApiErrorCode::RateLimited => "Too many tries. Wait a minute and try again.",
            ApiErrorCode::TooLarge => "That is too large.",
            ApiErrorCode::QuotaFull => "The server is out of space for uploads.",
            ApiErrorCode::InviteInvalid | ApiErrorCode::ResetInvalid => "That link does not work.",
            ApiErrorCode::Internal => "Something went wrong on the server. Try again.",
        }
    }

    /// The text for a failed sign in.
    pub fn login_text(self) -> &'static str {
        match self {
            Fail::Api { status: 401, .. } => "Wrong username or password.",
            // The Origin check: the address typed is not the one the server is set up with.
            Fail::Api { status: 403, .. } => {
                "The server refused this address. Use the address you open Vesper at in a browser."
            }
            f => f.text(),
        }
    }
}

/// The origin of what the user typed: `vesper.example.com` becomes `https://vesper.example.com`. Lowercase host,
/// no default port, no path, so it matches the server's `VESPER_ORIGIN` byte for byte. `None` if it is not one.
pub fn origin_of(input: &str) -> Option<Url> {
    let s = input.trim();
    let s = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{s}")
    };
    let u = Url::parse(&s).ok()?;
    if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none_or(str::is_empty) {
        return None;
    }
    Url::parse(&u.origin().ascii_serialization()).ok()
}

/// One server, one cookie jar.
#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    jar: Arc<Jar>,
    base: Url,
    /// `base` without the trailing slash: the `Origin` header.
    pub origin: String,
}

impl Api {
    /// `cookie` is the `name=value` pairs of a stored session.
    pub fn new(base: Url, cookie: Option<&str>) -> Api {
        // Two TLS providers are in the tree (GPUI brings ring); name one for reqwest and the socket.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let origin = base.origin().ascii_serialization();
        let jar = Arc::new(Jar::default());
        // A restored cookie gets back the flags a browser would hold: `Secure` on https, so it never goes out over
        // plain http, as the server's `__Host-` cookie requires.
        let secure = if base.scheme() == "https" {
            "; Secure"
        } else {
            ""
        };
        for pair in cookie.into_iter().flat_map(|c| c.split("; ")) {
            jar.add_cookie_str(&format!("{pair}; Path=/; HttpOnly{secure}"), &base);
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            ORIGIN,
            HeaderValue::from_str(&origin).expect("an origin is ASCII"),
        );
        let http = reqwest::Client::builder()
            .cookie_provider(jar.clone())
            .default_headers(headers)
            .user_agent(concat!("vesper-desktop/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("the HTTP client builds");
        Api {
            http,
            jar,
            base,
            origin,
        }
    }

    /// The session cookie as `name=value`, as it would be sent to the server.
    pub fn cookie(&self) -> Option<String> {
        self.jar
            .cookies(&self.base)
            .and_then(|v| v.to_str().ok().map(String::from))
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, Fail> {
        let url = self.base.join(path).map_err(|_| Fail::NotVesper)?;
        let mut req = self.http.request(method, url);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let res = req.send().await.map_err(|_| Fail::Network)?;
        let status = res.status();
        if !status.is_success() {
            let e = res.json::<ApiError>().await.map_err(|_| Fail::NotVesper)?;
            return Err(Fail::Api {
                status: status.as_u16(),
                code: e.code,
            });
        }
        if status == StatusCode::NO_CONTENT {
            return serde_json::from_value(Value::Null).map_err(|_| Fail::NotVesper);
        }
        res.json().await.map_err(|_| Fail::NotVesper)
    }

    pub async fn login(&self, username: String, password: Secret) -> Result<Me, Fail> {
        let body = serde_json::to_value(LoginReq { username, password }).expect("json");
        self.call(Method::POST, "/api/login", Some(body)).await
    }

    pub async fn me(&self) -> Result<Me, Fail> {
        self.call(Method::GET, "/api/me", None).await
    }

    pub async fn logout(&self) -> Result<(), Fail> {
        self.call(Method::POST, "/api/logout", None).await
    }

    /// `query` is empty for the newest page, or `&before=<id>`.
    pub async fn messages(&self, room: i64, limit: u32, query: &str) -> Result<MessagePage, Fail> {
        self.call(
            Method::GET,
            &format!("/api/rooms/{room}/messages?limit={limit}{query}"),
            None,
        )
        .await
    }

    pub async fn send(&self, room: i64, body: String) -> Result<Message, Fail> {
        let b = serde_json::to_value(SendMessage {
            body,
            reply_to: None,
            attachments: vec![],
        })
        .expect("json");
        self.call(
            Method::POST,
            &format!("/api/rooms/{room}/messages"),
            Some(b),
        )
        .await
    }

    pub async fn mark_read(&self, room: i64, id: i64) -> Result<(), Fail> {
        let b = serde_json::to_value(ReadReq { last_read_id: id }).expect("json");
        self.call(Method::PUT, &format!("/api/rooms/{room}/read"), Some(b))
            .await
    }

    /// Opens `/ws` with the cookie and the `Origin` header.
    pub async fn connect(
        &self,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Fail,
    > {
        let url = format!(
            "ws{}/ws",
            self.origin.strip_prefix("http").unwrap_or(&self.origin)
        );
        let mut req = url.into_client_request().map_err(|_| Fail::NotVesper)?;
        let h = req.headers_mut();
        h.insert(
            ORIGIN,
            HeaderValue::from_str(&self.origin).expect("an origin is ASCII"),
        );
        if let Some(c) = self.jar.cookies(&self.base) {
            h.insert(COOKIE, c);
        }
        match tokio_tungstenite::connect_async(req).await {
            Ok((ws, _)) => Ok(ws),
            Err(tungstenite::Error::Http(r)) => Err(Fail::Api {
                status: r.status().as_u16(),
                code: ApiErrorCode::Unauthorized,
            }),
            Err(_) => Err(Fail::Network),
        }
    }
}

/// Messages per history page, as in the web app.
pub const PAGE: u32 = 50;
/// The retry count starts over only after a socket stayed up this long.
const STABLE: Duration = Duration::from_secs(10);

/// Retry `n` waits a random 50 to 100% of min(30 s, 2 s * 2^n), like the web app.
pub fn backoff(n: u32) -> Duration {
    use std::hash::{BuildHasher, Hasher};
    let base = 2000u64.saturating_mul(1 << n.min(5)).min(30_000);
    let r = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
        % 1000;
    Duration::from_millis(base / 2 + base * r / 2000)
}

/// Starts the network thread. It stops when every `Cmd` sender is dropped. Calls open their audio from `devices`.
pub fn start(
    store: Arc<dyn SessionStore>,
    devices: Arc<dyn AudioDevices>,
) -> (UnboundedSender<Cmd>, UnboundedReceiver<Event>) {
    let (cmd_tx, cmd_rx) = unbounded_channel();
    let (tx, ev_rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("vesper-net".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(
                Net {
                    store,
                    devices,
                    tx,
                    conn: None,
                    call: None,
                    warned: false,
                }
                .run(cmd_rx),
            );
        })
        .expect("the network thread starts");
    (cmd_tx, ev_rx)
}

struct Conn {
    api: Api,
    socket: JoinHandle<()>,
    /// Messages for the socket.
    out: UnboundedSender<ClientMsg>,
}

struct Net {
    store: Arc<dyn SessionStore>,
    devices: Arc<dyn AudioDevices>,
    tx: UnboundedSender<Event>,
    conn: Option<Conn>,
    call: Option<Call>,
    /// NotRemembered was sent.
    warned: bool,
}

const SESSION_ENDED: &str = "Your session ended. Sign in again.";

impl Net {
    async fn run(mut self, mut rx: UnboundedReceiver<Cmd>) {
        while let Some(cmd) = rx.recv().await {
            self.handle(cmd).await;
        }
        self.drop_conn();
    }

    fn emit(&self, ev: Event) {
        let _ = self.tx.send(ev);
    }

    /// Ends the socket and any call.
    fn drop_conn(&mut self) -> Option<Api> {
        self.call = None;
        let c = self.conn.take()?;
        c.socket.abort();
        Some(c.api)
    }

    /// Runs a store call off the runtime thread: the keyring may block on D-Bus.
    async fn store<T: Send + 'static>(
        &mut self,
        f: impl FnOnce(&dyn SessionStore) -> Result<T, String> + Send + 'static,
    ) -> Option<T> {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || f(&*store)).await {
            Ok(Ok(v)) => Some(v),
            _ => {
                if !std::mem::replace(&mut self.warned, true) {
                    self.emit(Event::NotRemembered);
                }
                None
            }
        }
    }

    fn signed_in(&mut self, api: Api, me: Me) {
        self.drop_conn();
        let (out, out_rx) = unbounded_channel();
        let socket = tokio::spawn(socket(api.clone(), self.tx.clone(), out_rx));
        let server = api.origin.clone();
        self.conn = Some(Conn { api, socket, out });
        self.emit(Event::SignedIn {
            me: Box::new(me),
            server,
        });
    }

    async fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Restore => self.restore().await,
            Cmd::Login {
                server,
                username,
                password,
            } => {
                let Some(base) = origin_of(&server) else {
                    let e = "Enter the server address, like https://vesper.example.com.";
                    return self.emit(Event::SignInFailed(e.into()));
                };
                let api = Api::new(base, None);
                match api.login(username, password).await {
                    Ok(me) => {
                        if let Some(cookie) = api.cookie() {
                            let saved = Saved {
                                server: api.origin.clone(),
                                cookie: cookie.into(),
                            };
                            self.store(move |s| s.save(&saved)).await;
                        }
                        self.signed_in(api, me);
                    }
                    Err(f) => self.emit(Event::SignInFailed(f.login_text().into())),
                }
            }
            Cmd::Logout => {
                // The socket first, so the server's 4001 for this logout is not read as a session ended elsewhere.
                let api = self.drop_conn();
                if let Some(api) = &api {
                    let _ = api.logout().await; // signed out here either way
                }
                self.store(|s| s.clear()).await;
                self.emit(Event::SignedOut {
                    server: api.map(|a| a.origin),
                    notice: String::new(),
                });
            }
            Cmd::Forget => {
                self.drop_conn();
                self.store(|s| s.clear()).await;
            }
            Cmd::Signal(m) => {
                if let Some(c) = &self.conn {
                    let _ = c.out.send(m);
                }
            }
            Cmd::Voice(VoiceCmd::Start {
                call,
                bitrate_kbps,
                volumes,
            }) => {
                self.call = None; // the old call's thread ends first
                let tx = self.tx.clone();
                let emit = move |ev| drop(tx.send(Event::Voice { call, ev }));
                self.call = Some(Call::start(
                    self.devices.clone(),
                    bitrate_kbps,
                    volumes,
                    emit,
                ));
            }
            Cmd::Voice(VoiceCmd::Stop) => self.call = None,
            Cmd::Voice(c) => {
                if let Some(call) = &self.call {
                    call.send(c);
                }
            }
            cmd => {
                if let Some(c) = &self.conn {
                    tokio::spawn(request(c.api.clone(), self.tx.clone(), cmd));
                }
            }
        }
    }

    async fn restore(&mut self) {
        let signed_out = |server: Option<String>, notice: &str| Event::SignedOut {
            server,
            notice: notice.into(),
        };
        let Some(Some(saved)) = self.store(|s| s.load()).await else {
            return self.emit(signed_out(None, ""));
        };
        let Some(base) = origin_of(&saved.server) else {
            self.store(|s| s.clear()).await;
            return self.emit(signed_out(None, ""));
        };
        let api = Api::new(base, Some(&saved.cookie));
        match api.me().await {
            Ok(me) => self.signed_in(api, me),
            Err(f) if f.unauthorized() => {
                self.store(|s| s.clear()).await;
                self.emit(signed_out(Some(saved.server), SESSION_ENDED));
            }
            // Kept: the server may be back later. Signing in again replaces it.
            Err(f) => self.emit(signed_out(Some(saved.server), f.text())),
        }
    }
}

/// One REST call; a 401 ends the session.
async fn request(api: Api, tx: UnboundedSender<Event>, cmd: Cmd) {
    let (res, ev) = match cmd {
        Cmd::LoadLatest(room) => match api.messages(room, PAGE, "").await {
            Ok(page) => (Ok(()), Event::Latest { room, page }),
            Err(f) => (
                Err(f),
                Event::LoadFailed {
                    room,
                    error: f.text().into(),
                },
            ),
        },
        Cmd::LoadOlder { room, before } => {
            match api.messages(room, PAGE, &format!("&before={before}")).await {
                Ok(page) => (Ok(()), Event::Older { room, page }),
                Err(f) => (
                    Err(f),
                    Event::LoadFailed {
                        room,
                        error: f.text().into(),
                    },
                ),
            }
        }
        Cmd::SeedActivity(rooms) => {
            let mut got = Vec::new();
            // A failed fetch leaves that DM unranked.
            for room in rooms {
                if let Ok(p) = api.messages(room, 1, "").await {
                    got.push((room, p.messages.last().map_or(0, |m| m.id)));
                }
            }
            (Ok(()), Event::Activity(got))
        }
        Cmd::Send { room, body } => match api.send(room, body.clone()).await {
            Ok(m) => (Ok(()), Event::Sent(Box::new(m))),
            Err(f) => (
                Err(f),
                Event::SendFailed {
                    room,
                    body,
                    error: f.text().into(),
                },
            ),
        },
        Cmd::MarkRead { room, id } => match api.mark_read(room, id).await {
            Ok(()) => return,
            Err(f) => (Err(f), Event::ReadFailed { room }),
        },
        _ => return,
    };
    let ev = if res.is_err_and(Fail::unauthorized) {
        Event::SessionEnded
    } else {
        ev
    };
    let _ = tx.send(ev);
}

/// Keeps one socket open while signed in, reconnecting with backoff, like `session.svelte.ts`. Sends what comes on `out`.
async fn socket(api: Api, tx: UnboundedSender<Event>, mut out: UnboundedReceiver<ClientMsg>) {
    let mut tries = 0;
    loop {
        let up = Instant::now();
        let code = match api.connect().await {
            Ok(mut ws) => {
                // What was asked while the socket was down belonged to the old one's call.
                while out.try_recv().is_ok() {}
                let mut code = None;
                loop {
                    let frame = tokio::select! {
                        f = ws.next() => f,
                        Some(m) = out.recv() => {
                            let text = serde_json::to_string(&m).expect("json");
                            if ws.send(WsMsg::text(text)).await.is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    let Some(frame) = frame else { break };
                    match frame {
                        Ok(WsMsg::Text(t)) => {
                            // shortcut: a message kind this build does not know is skipped; the web app instead stops
                            // on a server build change. Revisit when a protocol change lands.
                            if let Ok(m) = serde_json::from_str::<ServerMsg>(&t)
                                && tx.send(Event::Server(Box::new(m))).is_err()
                            {
                                return;
                            }
                        }
                        Ok(WsMsg::Close(f)) => {
                            code = f.map(|f| u16::from(f.code));
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                code
            }
            Err(f) if f.unauthorized() => Some(CLOSE_SESSION_ENDED),
            Err(_) => None,
        };
        match code {
            Some(CLOSE_SESSION_ENDED) => return drop(tx.send(Event::SessionEnded)),
            Some(CLOSE_DISCONNECTED) => return drop(tx.send(Event::Disconnected)),
            _ => {}
        }
        if up.elapsed() >= STABLE {
            tries = 0;
        }
        let wait = backoff(tries);
        tries += 1;
        if tx.send(Event::Offline { retry_in: wait }).is_err() {
            return;
        }
        tokio::time::sleep(wait).await;
        // A refused upgrade looks like a network drop, so ask /api/me first.
        if api.me().await.is_err_and(Fail::unauthorized) {
            return drop(tx.send(Event::SessionEnded));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_of_normalises_like_the_server() {
        let o = |s| origin_of(s).map(|u| u.origin().ascii_serialization());
        assert_eq!(
            o("vesper.example.com").as_deref(),
            Some("https://vesper.example.com")
        );
        assert_eq!(
            o(" HTTPS://Vesper.Example.com:443/r/3 ").as_deref(),
            Some("https://vesper.example.com")
        );
        assert_eq!(
            o("http://127.0.0.1:8080/").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(o("ftp://x.example"), None);
        assert_eq!(o(""), None);
    }

    #[test]
    fn a_restored_cookie_is_secure_on_https() {
        let https = Api::new(
            origin_of("https://v.example").unwrap(),
            Some("__Host-vesper_session=abc"),
        );
        assert_eq!(https.cookie().as_deref(), Some("__Host-vesper_session=abc"));
        // The jar holds it as Secure: it is never sent to the same host over plain http.
        assert!(
            https
                .jar
                .cookies(&"http://v.example/".parse().unwrap())
                .is_none()
        );
        let http = Api::new(
            origin_of("http://127.0.0.1:9").unwrap(),
            Some("vesper_session=abc"),
        );
        assert_eq!(http.cookie().as_deref(), Some("vesper_session=abc"));
    }

    #[test]
    fn errors_in_plain_words() {
        let api = |status, code| Fail::Api { status, code };
        assert_eq!(
            api(401, ApiErrorCode::Unauthorized).login_text(),
            "Wrong username or password."
        );
        assert_eq!(
            api(429, ApiErrorCode::RateLimited).login_text(),
            "Too many tries. Wait a minute and try again."
        );
        assert!(
            Fail::Network
                .login_text()
                .starts_with("Could not reach the server")
        );
        assert!(
            api(403, ApiErrorCode::Forbidden)
                .login_text()
                .contains("refused this address")
        );
        assert!(api(401, ApiErrorCode::Unauthorized).unauthorized());
    }

    #[test]
    fn backoff_doubles_to_a_cap_with_jitter() {
        for (n, base) in [(0, 2000), (1, 4000), (3, 16000), (4, 30000), (40, 30000)] {
            let ms = backoff(n).as_millis() as u64;
            assert!((base / 2..=base).contains(&ms), "{n}: {ms}");
        }
    }
}
