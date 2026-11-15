//! HTTP and the WebSocket, on one tokio thread. The UI sends `Cmd`s and gets `Event`s back; it owns all state.
//!
//! Like a browser on the server's own page: the session cookie lives in a cookie jar, and every request and the
//! socket upgrade send `Origin` equal to the server's origin, which the server checks (ADR 0007).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{CONTENT_LENGTH, COOKIE, HeaderMap, HeaderValue, ORIGIN};
use reqwest::{Method, RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::{self, Message as WsMsg, client::IntoClientRequest};
use url::Url;
use vesper_protocol::{
    ApiError, ApiErrorCode, Attachment, CLOSE_DISCONNECTED, CLOSE_SESSION_ENDED, ClientMsg,
    EditMessage, LoginReq, Me, Message, MessagePage, PinReq, ReactionReq, ReadReq, Secret,
    SendMessage, ServerMsg, UploadDone,
};

use crate::files::{self, IMAGE_MAX, Picture};
use crate::settings::Denoise;
use crate::store::{Saved, SessionStore};
use crate::voice::denoise::{self, MODEL_MAX, MODEL_URL};
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
    /// `attachments` are (tray key, upload id); the keys come back with `Sent`.
    Send {
        room: i64,
        body: String,
        reply_to: Option<i64>,
        attachments: Vec<(u64, String)>,
    },
    Edit {
        id: i64,
        body: String,
    },
    Delete(i64),
    React {
        id: i64,
        emoji: String,
        on: bool,
    },
    Pin {
        id: i64,
        on: bool,
    },
    LoadPins(i64),
    /// Sends the file at `path` as the raw body of `POST /api/rooms/{room}/uploads`.
    Upload {
        key: u64,
        room: i64,
        path: PathBuf,
    },
    CancelUpload(u64),
    /// An image attachment, decoded small for the timeline.
    FetchImage(String),
    /// An attachment written to `to`; `open` hands it to the system's app after.
    Download {
        id: String,
        to: PathBuf,
        open: bool,
    },
    MarkRead {
        room: i64,
        id: i64,
    },
    /// To the call engine. `Start` starts a call, ending the one before.
    Voice(VoiceCmd),
    /// A message on the socket. Dropped while it is down.
    Signal(ClientMsg),
    /// `POST /api/users/{user_id}/{action}`: `force-mute` (with `muted`), `stop-share`, `kick` or `disconnect`.
    Moderate {
        user_id: String,
        action: &'static str,
        muted: Option<bool>,
    },
    /// `POST /api/dms`: the DM with this person, made the first time.
    OpenDm(String),
    /// Offline: try the socket now, not at the end of the wait.
    RetryNow,
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
    Sent {
        message: Box<Message>,
        keys: Vec<u64>,
    },
    SendFailed {
        room: i64,
        body: String,
        error: String,
    },
    ReadFailed {
        room: i64,
    },
    /// The new copy after a reaction or pin.
    Changed(Box<Message>),
    Edited(Box<Message>),
    /// An edit, delete, reaction or pin failed.
    ChangeFailed(String),
    Pins {
        room: i64,
        pins: Result<Vec<Message>, String>,
    },
    UploadProgress {
        key: u64,
        progress: f32,
    },
    Uploaded {
        key: u64,
        attachment: Attachment,
    },
    /// `fail` is `None` when the file could not be read.
    UploadFailed {
        key: u64,
        fail: Option<Fail>,
    },
    /// `None` when it did not load or decode.
    Image {
        id: String,
        picture: Option<Picture>,
    },
    Downloaded {
        path: PathBuf,
        open: bool,
    },
    DownloadFailed(String),
    /// A moderator action's answer: the error in words when it failed.
    Moderated(Result<(), String>),
    DmOpened(Box<vesper_protocol::RoomInfo>),
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
        Self::answer(req).await
    }

    async fn answer<T: DeserializeOwned>(req: RequestBuilder) -> Result<T, Fail> {
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

    pub async fn send(&self, room: i64, msg: SendMessage) -> Result<Message, Fail> {
        let b = serde_json::to_value(msg).expect("json");
        self.call(
            Method::POST,
            &format!("/api/rooms/{room}/messages"),
            Some(b),
        )
        .await
    }

    /// `PATCH`, `DELETE` or `PUT` on `/api/messages/{id}{path}`.
    pub async fn change<T: DeserializeOwned>(
        &self,
        method: Method,
        id: i64,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, Fail> {
        self.call(method, &format!("/api/messages/{id}{path}"), body)
            .await
    }

    pub async fn pins(&self, room: i64) -> Result<Vec<Message>, Fail> {
        self.call(Method::GET, &format!("/api/rooms/{room}/pins"), None)
            .await
    }

    /// Streams the file at `path` up, calling `progress` with the share sent so far. `Err(None)`: unreadable.
    pub async fn upload(
        &self,
        room: i64,
        path: &std::path::Path,
        progress: impl Fn(f32) + Send + Sync + 'static,
    ) -> Result<Attachment, Option<Fail>> {
        use tokio::io::AsyncReadExt;
        let file = tokio::fs::File::open(path).await.map_err(|_| None)?;
        let size = file.metadata().await.map_err(|_| None)?.len();
        let name = path
            .file_name()
            .map_or("file".into(), |n| n.to_string_lossy().into_owned());
        let mut url = self
            .base
            .join(&format!("/api/rooms/{room}/uploads"))
            .map_err(|_| Some(Fail::NotVesper))?;
        url.query_pairs_mut().append_pair("name", &name);
        let progress = Arc::new(progress);
        let stream = futures_util::stream::unfold((file, 0u64), move |(mut f, sent)| {
            let progress = progress.clone();
            async move {
                let mut buf = vec![0; 64 * 1024];
                match f.read(&mut buf).await {
                    Ok(0) => None,
                    Ok(n) => {
                        buf.truncate(n);
                        let sent = sent + n as u64;
                        progress(sent as f32 / size.max(1) as f32);
                        Some((Ok::<_, std::io::Error>(buf), (f, sent)))
                    }
                    Err(e) => Some((Err(e), (f, sent))),
                }
            }
        });
        let req = self
            .http
            .post(url)
            .header(CONTENT_LENGTH, size)
            .body(reqwest::Body::wrap_stream(stream))
            // The server cuts off a slow upload itself; this only ends one that hangs.
            .timeout(Duration::from_secs(3600));
        Self::answer::<UploadDone>(req)
            .await
            .map(|d| d.attachment)
            .map_err(Some)
    }

    /// An uploaded file, at most `max` bytes. The id must be the server's 32 hex.
    pub async fn download(&self, id: &str, max: u64) -> Result<Vec<u8>, Fail> {
        if !files::valid_id(id) {
            return Err(Fail::NotVesper);
        }
        let url = self
            .base
            .join(&format!("/api/uploads/{id}"))
            .map_err(|_| Fail::NotVesper)?;
        let mut res = self
            .http
            .get(url)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|_| Fail::Network)?;
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let code = res
                .json::<ApiError>()
                .await
                .map_or(ApiErrorCode::Internal, |e| e.code);
            return Err(Fail::Api { status, code });
        }
        let mut bytes = vec![];
        while let Some(chunk) = res.chunk().await.map_err(|_| Fail::Network)? {
            if (bytes.len() + chunk.len()) as u64 > max {
                return Err(Fail::Api {
                    status: 413,
                    code: ApiErrorCode::TooLarge,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    pub async fn mark_read(&self, room: i64, id: i64) -> Result<(), Fail> {
        let b = serde_json::to_value(ReadReq { last_read_id: id }).expect("json");
        self.call(Method::PUT, &format!("/api/rooms/{room}/read"), Some(b))
            .await
    }

    /// The DeepFilterNet model the web app loads, from `/dsp/`. It is checked by the caller. The error says why in a
    /// few words, for "DeepFilterNet is not available (...)".
    pub async fn model(&self) -> Result<Vec<u8>, String> {
        let url = self.base.join(MODEL_URL).map_err(|e| e.to_string())?;
        let mut res = self
            .http
            .get(url)
            .timeout(Duration::from_secs(300)) // 8 MB on a slow link
            .send()
            .await
            .map_err(|_| "the server did not answer".to_string())?;
        if !res.status().is_success() {
            return Err(format!("the server has no model: {}", res.status()));
        }
        let mut bytes = vec![];
        while let Some(chunk) = res
            .chunk()
            .await
            .map_err(|_| "the download broke off".to_string())?
        {
            if bytes.len() + chunk.len() > MODEL_MAX {
                return Err("the server sent something too large".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
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
                    uploads: HashMap::new(),
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
    /// Cuts the socket's wait before its next try.
    retry: Arc<tokio::sync::Notify>,
    /// Messages for the socket.
    out: UnboundedSender<ClientMsg>,
}

struct Net {
    store: Arc<dyn SessionStore>,
    devices: Arc<dyn AudioDevices>,
    tx: UnboundedSender<Event>,
    conn: Option<Conn>,
    call: Option<Call>,
    /// Running uploads by tray key, so Cancel can stop them.
    uploads: HashMap<u64, JoinHandle<()>>,
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
        for (_, u) in self.uploads.drain() {
            u.abort();
        }
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
        let retry = Arc::new(tokio::sync::Notify::new());
        let socket = tokio::spawn(socket(api.clone(), self.tx.clone(), out_rx, retry.clone()));
        let server = api.origin.clone();
        self.conn = Some(Conn {
            api,
            socket,
            retry,
            out,
        });
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
            Cmd::RetryNow => {
                if let Some(c) = &self.conn {
                    c.retry.notify_one();
                }
            }
            Cmd::Voice(VoiceCmd::Start { call, setup }) => {
                self.call = None; // the old call's thread ends first
                let (loaded, denoiser) = std::sync::mpsc::channel();
                if setup.denoise == Denoise::DeepFilter {
                    // The call starts now, with the noise gate; DeepFilterNet joins it once loaded.
                    let api = self.conn.as_ref().map(|c| c.api.clone());
                    tokio::spawn(async move {
                        let fetch = async {
                            match api {
                                Some(api) => api.model().await,
                                None => Err("not signed in".into()),
                            }
                        };
                        let _ = loaded.send(denoise::load(denoise::cache_path(), fetch).await);
                    });
                }
                let tx = self.tx.clone();
                let emit = move |ev| drop(tx.send(Event::Voice { call, ev }));
                self.call = Some(Call::start(self.devices.clone(), setup, denoiser, emit));
            }
            Cmd::Upload { key, room, path } => {
                let Some(c) = &self.conn else { return };
                let (api, tx) = (c.api.clone(), self.tx.clone());
                self.uploads.retain(|_, h| !h.is_finished());
                self.uploads.insert(
                    key,
                    tokio::spawn(async move {
                        let shown = std::sync::atomic::AtomicU32::new(0);
                        let ptx = tx.clone();
                        // At most one event per percent.
                        let progress = move |p: f32| {
                            let pct = (p * 100.0) as u32;
                            if shown.swap(pct, std::sync::atomic::Ordering::Relaxed) != pct {
                                let _ = ptx.send(Event::UploadProgress { key, progress: p });
                            }
                        };
                        let ev = match api.upload(room, &path, progress).await {
                            Ok(attachment) => Event::Uploaded { key, attachment },
                            Err(Some(f)) if f.unauthorized() => Event::SessionEnded,
                            Err(fail) => Event::UploadFailed { key, fail },
                        };
                        let _ = tx.send(ev);
                    }),
                );
            }
            // Dropping the request ends it; the server deletes the partial file.
            Cmd::CancelUpload(key) => {
                if let Some(h) = self.uploads.remove(&key) {
                    h.abort();
                }
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

pub const ATTACH_FAILED: &str = "That file could not be attached. Upload it again.";
pub const PINS_FULL: &str = "This room has 50 pins, the most it can hold. Unpin one first.";
pub const SAVE_FAILED: &str = "The file could not be saved there.";

/// The answer to an edit, reaction or pin.
fn changed(
    r: Result<Message, Fail>,
    ok: impl FnOnce(Message) -> Event,
) -> (Result<(), Fail>, Event) {
    match r {
        Ok(m) => (Ok(()), ok(m)),
        Err(f) => (Err(f), Event::ChangeFailed(f.text().into())),
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
        Cmd::Send {
            room,
            body,
            reply_to,
            attachments,
        } => {
            let (keys, ids): (Vec<u64>, Vec<String>) = attachments.into_iter().unzip();
            let msg = SendMessage {
                body: body.clone(),
                reply_to,
                attachments: ids,
            };
            match api.send(room, msg).await {
                Ok(m) => (
                    Ok(()),
                    Event::Sent {
                        message: Box::new(m),
                        keys,
                    },
                ),
                Err(f) => {
                    let error = match f {
                        Fail::Api { status: 409, .. } => ATTACH_FAILED,
                        f => f.text(),
                    };
                    let error = error.into();
                    (Err(f), Event::SendFailed { room, body, error })
                }
            }
        }
        Cmd::Edit { id, body } => {
            let b = serde_json::to_value(EditMessage { body }).expect("json");
            changed(api.change(Method::PATCH, id, "", Some(b)).await, |m| {
                Event::Edited(Box::new(m))
            })
        }
        Cmd::Delete(id) => match api.change::<()>(Method::DELETE, id, "", None).await {
            // The server pushes the deleted copy.
            Ok(()) => return,
            Err(f) => (Err(f), Event::ChangeFailed(f.text().into())),
        },
        Cmd::React { id, emoji, on } => {
            let b = serde_json::to_value(ReactionReq { emoji, on }).expect("json");
            let r = api.change(Method::PUT, id, "/reaction", Some(b)).await;
            changed(r, |m| Event::Changed(Box::new(m)))
        }
        Cmd::Pin { id, on } => {
            let b = serde_json::to_value(PinReq { on }).expect("json");
            match api.change(Method::PUT, id, "/pin", Some(b)).await {
                Ok(m) => (Ok(()), Event::Changed(Box::new(m))),
                Err(f @ Fail::Api { status: 409, .. }) => {
                    (Err(f), Event::ChangeFailed(PINS_FULL.into()))
                }
                Err(f) => (Err(f), Event::ChangeFailed(f.text().into())),
            }
        }
        Cmd::LoadPins(room) => {
            let r = api.pins(room).await;
            let res = r.as_ref().map(|_| ()).map_err(|f| *f);
            (
                res,
                Event::Pins {
                    room,
                    pins: r.map_err(|f| f.text().into()),
                },
            )
        }
        Cmd::FetchImage(id) => {
            let r = api.download(&id, IMAGE_MAX).await;
            let res = r.as_ref().map(|_| ()).map_err(|f| *f);
            let picture = match r {
                Ok(bytes) => tokio::task::spawn_blocking(move || files::decode_picture(&bytes))
                    .await
                    .ok()
                    .flatten(),
                Err(_) => None,
            };
            (res, Event::Image { id, picture })
        }
        Cmd::Download { id, to, open } => match api.download(&id, u64::MAX).await {
            Ok(bytes) => {
                let path = to.clone();
                let wrote = tokio::task::spawn_blocking(move || {
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&path, bytes)
                })
                .await;
                match wrote {
                    Ok(Ok(())) => (Ok(()), Event::Downloaded { path: to, open }),
                    _ => (Ok(()), Event::DownloadFailed(SAVE_FAILED.into())),
                }
            }
            Err(f) => (Err(f), Event::DownloadFailed(f.text().into())),
        },
        Cmd::Moderate {
            user_id,
            action,
            muted,
        } => {
            let body = muted.map(|m| serde_json::json!({ "muted": m }));
            let r = api
                .call::<()>(
                    Method::POST,
                    &format!("/api/users/{user_id}/{action}"),
                    body,
                )
                .await;
            (r, Event::Moderated(r.map_err(|f| f.text().into())))
        }
        Cmd::OpenDm(user_id) => {
            let body = serde_json::json!({ "user_id": user_id });
            match api.call(Method::POST, "/api/dms", Some(body)).await {
                Ok(room) => (Ok(()), Event::DmOpened(Box::new(room))),
                Err(f) => (Err(f), Event::Moderated(Err(f.text().into()))),
            }
        }
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
async fn socket(
    api: Api,
    tx: UnboundedSender<Event>,
    mut out: UnboundedReceiver<ClientMsg>,
    retry: Arc<tokio::sync::Notify>,
) {
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
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = retry.notified() => {}
        }
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
