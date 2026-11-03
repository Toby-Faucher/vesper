//! Web Push (issue #58): "alice mentioned you in Games" and "alice started voice in Games" for
//! people with no Vesper tab open.
//!
//! - VAPID (RFC 8292): an ES256 key made on first start and kept in the database
//!   (`server_secrets`), so a copy of `vesper.db` holds it. Only the public key leaves the server.
//! - The payload is encrypted for the browser (RFC 8291, `aes128gcm`), so the push service sees
//!   neither names nor rooms. It never holds a message body.
//! - SSRF: the server POSTs to a URL the browser gave it. Only https on port 443 to the known push
//!   services' hosts; every resolved address must be public, and the connection goes to the
//!   checked address (no second lookup). No redirects, no cookies, a 10 s limit, and only the
//!   status line of the answer is read.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use aws_lc_rs::{aead, agreement, hkdf};
use axum::http::Uri;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tracing::{debug, warn};
use vesper_protocol::{PushPayload, RoomInfo, RoomKind, UserInfo};
use vesper_store::{PushKind, PushTarget, Store, StoreError, now_ms};

use crate::api_rooms::can_see;
use crate::hub::Hub;

#[cfg(all(feature = "test-push", not(debug_assertions)))]
compile_error!("the test-push feature lets push reach 127.0.0.1: never in a release build");

/// Hosts the server will POST to: an exact name, or any name under a `.suffix`.
const PUSH_HOSTS: &[(&str, bool)] = &[
    ("fcm.googleapis.com", false), // Chrome, Edge on Android, Opera, Samsung
    ("updates.push.services.mozilla.com", false), // Firefox
    ("push.apple.com", true),      // Safari (web.push.apple.com)
    ("notify.windows.com", true),  // Edge on Windows (wns2-*.notify.windows.com)
];
const MAX_ENDPOINT: usize = 2048;
/// Lookup, connect, TLS and the answer together, so a slow push service holds a slot this long at most.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// Sends in flight at once, server-wide.
const PARALLEL: usize = 8;
/// Sends waiting or in flight, server-wide; past this new pushes are dropped (a mention storm).
pub const MAX_PENDING: usize = 256;
/// One "started voice" push per room and person in this long.
pub const VOICE_EVERY: Duration = Duration::from_secs(10 * 60);
/// One "sent you a message" push per DM and person in this long, so a burst is one push.
pub const DM_EVERY: Duration = Duration::from_secs(30);
const VAPID_SECRET: &str = "vapid_es256_pkcs8";
/// Record size in the `aes128gcm` header: the whole payload is one record.
const RECORD_SIZE: u32 = 4096;

/// Where a push goes, after [`parse_endpoint`].
#[derive(Debug, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    /// Path and query, as sent on the request line.
    pub path: String,
    /// The JWT `aud`: scheme, host and a non-default port.
    pub audience: String,
    tls: bool,
}

/// Checks a subscription's endpoint before it is stored and again before each send.
/// `allow_loopback` (the `test-push` feature only) also lets `http://127.0.0.1:<port>` through.
pub fn parse_endpoint(s: &str, allow_loopback: bool) -> Result<Endpoint, &'static str> {
    if s.len() > MAX_ENDPOINT {
        return Err("endpoint too long");
    }
    let uri: Uri = s.parse().map_err(|_| "endpoint is not a URL")?;
    let auth = uri.authority().ok_or("endpoint has no host")?;
    if auth.as_str().contains('@') {
        return Err("endpoint has a user part");
    }
    let host = auth.host().to_ascii_lowercase();
    let path = uri.path_and_query().map_or("/", |p| p.as_str()).to_string();
    let test = allow_loopback && host == "127.0.0.1" && uri.scheme_str() == Some("http");
    if test {
        let port = auth.port_u16().ok_or("endpoint needs a port")?;
        return Ok(Endpoint {
            audience: format!("http://{host}:{port}"),
            host,
            port,
            path,
            tls: false,
        });
    }
    if uri.scheme_str() != Some("https") {
        return Err("endpoint must be https");
    }
    // From the raw text: `port_u16` reads 65979 or 0x as "no port".
    if auth
        .as_str()
        .rsplit_once(':')
        .is_some_and(|(_, p)| p != "443")
    {
        return Err("endpoint must use port 443");
    }
    if !path.starts_with('/') {
        return Err("endpoint path must start with /");
    }
    let known = PUSH_HOSTS.iter().any(|&(h, sub)| {
        host == h || (sub && host.strip_suffix(h).is_some_and(|p| p.ends_with('.')))
    });
    if !known {
        return Err("endpoint is not a known push service");
    }
    Ok(Endpoint {
        audience: format!("https://{host}"),
        host,
        port: 443,
        path,
        tls: true,
    })
}

/// A public unicast address: no loopback, private, link-local (cloud metadata), shared (CGNAT),
/// documentation, benchmark, multicast or reserved space, and no IPv6 form that wraps IPv4.
pub fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (18..20).contains(&b)))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_is_public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            // Global unicast is 2000::/3; within it, documentation, 6to4 and Teredo are out.
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && s[1] == 0x0db8)
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && s[1] == 0)
        }
    }
}

/// The address to connect to: the first of `addrs`, and only if every one is allowed (a name
/// with one private address is refused, so DNS cannot pick the internal one later).
pub fn pick_addr(addrs: &[SocketAddr], allow_loopback: bool) -> Result<SocketAddr, &'static str> {
    let ok = |a: &SocketAddr| ip_is_public(a.ip()) || (allow_loopback && a.ip().is_loopback());
    match addrs.first() {
        None => Err("endpoint host has no address"),
        Some(_) if !addrs.iter().all(ok) => Err("endpoint host has a private address"),
        Some(&a) => Ok(a),
    }
}

/// Resolves once, checks every address, and returns the one to connect to.
pub async fn resolve(ep: &Endpoint, allow_loopback: bool) -> Result<SocketAddr, &'static str> {
    // No timeout of its own: `Push::send` bounds the lookup and the exchange together.
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((ep.host.as_str(), ep.port))
        .await
        .map_err(|_| "endpoint lookup failed")?
        .collect();
    pick_addr(&addrs, allow_loopback)
}

/// The VAPID key. Debug never prints the private half.
pub struct Vapid {
    key: EcdsaKeyPair,
    public: String,
}

impl std::fmt::Debug for Vapid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vapid")
            .field("public", &self.public)
            .finish()
    }
}

impl Vapid {
    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Vapid, String> {
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8)
            .map_err(|e| format!("stored VAPID key does not load: {e}"))?;
        let public = B64.encode(key.public_key().as_ref());
        Ok(Vapid { key, public })
    }

    /// The stored key, made and stored on first use.
    pub async fn load(store: &Store) -> Result<Vapid, String> {
        let make = || {
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .expect("P-256 key generation")
                .as_ref()
                .to_vec()
        };
        let pkcs8 = store
            .secret_or_insert(VAPID_SECRET, make, now_ms())
            .await
            .map_err(|e| e.to_string())?;
        Vapid::from_pkcs8(&pkcs8)
    }

    /// base64url of the 65-byte public point: the browser's `applicationServerKey`.
    pub fn public_key(&self) -> &str {
        &self.public
    }

    /// `Authorization` value: a JWT for `audience` that expires in 12 hours (RFC 8292 allows 24).
    pub fn authorization(&self, audience: &str, subject: &str, now_s: u64) -> String {
        let head = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims =
            serde_json::json!({ "aud": audience, "exp": now_s + 12 * 3600, "sub": subject });
        let body = B64.encode(claims.to_string());
        let signed = format!("{head}.{body}");
        let sig = self
            .key
            .sign(&SystemRandom::new(), signed.as_bytes())
            .expect("ES256 signing");
        format!(
            "vapid t={signed}.{}, k={}",
            B64.encode(sig.as_ref()),
            self.public
        )
    }
}

struct Len(usize);
impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand(prk: &hkdf::Prk, info: &[u8], len: usize) -> Result<Vec<u8>, &'static str> {
    let mut out = vec![0; len];
    prk.expand(&[info], Len(len))
        .and_then(|okm| okm.fill(&mut out))
        .map_err(|_| "hkdf")?;
    Ok(out)
}

/// RFC 8291 `aes128gcm` body for one browser: a fresh key pair and salt per message.
pub fn encrypt(plain: &[u8], ua_public: &[u8], auth: &[u8]) -> Result<Vec<u8>, &'static str> {
    let key = agreement::PrivateKey::generate(&agreement::ECDH_P256).map_err(|_| "keygen")?;
    let mut salt = [0u8; 16];
    aws_lc_rs::rand::fill(&mut salt).map_err(|_| "random")?;
    encrypt_with(plain, ua_public, auth, &salt, &key)
}

/// [`encrypt`] with a given salt and server key, for the RFC's test vector.
pub fn encrypt_with(
    plain: &[u8],
    ua_public: &[u8],
    auth: &[u8],
    salt: &[u8; 16],
    as_key: &agreement::PrivateKey,
) -> Result<Vec<u8>, &'static str> {
    if ua_public.len() != 65 || auth.len() != 16 || plain.len() + 17 > RECORD_SIZE as usize {
        return Err("bad keys or payload too large");
    }
    let as_public = as_key.compute_public_key().map_err(|_| "public key")?;
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, ua_public);
    let ecdh = agreement::agree(as_key, peer, "bad browser key", |s| Ok(s.to_vec()))?;
    // PRK_key = HMAC(auth, ecdh); IKM = HKDF-Expand(PRK_key, "WebPush: info\0" ua as, 32)
    let prk_key = hkdf::Salt::new(hkdf::HKDF_SHA256, auth).extract(&ecdh);
    let key_info = [b"WebPush: info\0".as_slice(), ua_public, as_public.as_ref()].concat();
    let ikm = expand(&prk_key, &key_info, 32)?;
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&ikm);
    let cek = expand(&prk, b"Content-Encoding: aes128gcm\0", 16)?;
    let nonce = expand(&prk, b"Content-Encoding: nonce\0", 12)?;
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| "aes key")?,
    );
    let mut record = [plain, &[2u8]].concat(); // 2: the last (only) record, no padding
    key.seal_in_place_append_tag(
        aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| "nonce")?,
        aead::Aad::empty(),
        &mut record,
    )
    .map_err(|_| "seal")?;
    let mut out = Vec::with_capacity(86 + record.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(65);
    out.extend_from_slice(as_public.as_ref());
    out.extend_from_slice(&record);
    Ok(out)
}

/// Who gets a push about `room` from `sender`: of `users` (the candidates), those not banned,
/// who can see the room, are not the sender and have no socket open (`online`).
pub fn recipients<'a>(
    users: impl IntoIterator<Item = &'a UserInfo>,
    room: &RoomInfo,
    sender: &str,
    online: impl Fn(&str) -> bool,
) -> Vec<String> {
    users
        .into_iter()
        .filter(|u| !u.banned && u.user_id != sender && can_see(u, room) && !online(&u.user_id))
        .map(|u| u.user_id.clone())
        .collect()
}

/// At most one "started voice" push per (room, person) per [`VOICE_EVERY`].
#[derive(Default)]
pub struct VoiceDedupe(HashMap<(i64, String), Instant>);

impl VoiceDedupe {
    /// True (and remembered) when `who` has had no voice push for `room` lately.
    pub fn take(&mut self, room: i64, who: &str, now: Instant) -> bool {
        self.take_within(room, who, now, VOICE_EVERY)
    }

    /// Like [`VoiceDedupe::take`] with your own window.
    pub fn take_within(&mut self, room: i64, who: &str, now: Instant, every: Duration) -> bool {
        if self.0.len() > 10_000 {
            self.0
                .retain(|_, t| now.saturating_duration_since(*t) < every);
        }
        let key = (room, who.to_string());
        match self.0.get(&key) {
            Some(t) if now.saturating_duration_since(*t) < every => false,
            _ => {
                self.0.insert(key, now);
                true
            }
        }
    }
}

/// The path the service worker fetches for a person's picture; none while they have the letter avatar.
fn avatar_icon(u: &UserInfo) -> Option<String> {
    let v = u.avatar.as_ref()?;
    Some(format!("/api/users/{}/avatar?v={v}", u.user_id))
}

/// One place in the send queue; dropping it (the task ended, panicked or was cancelled) gives
/// the place back.
pub struct Reserved(Arc<AtomicUsize>);

impl Drop for Reserved {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A place in the queue `count`, or None when `max` are taken.
pub fn reserve(count: &Arc<AtomicUsize>, max: usize) -> Option<Reserved> {
    if count.fetch_add(1, Ordering::SeqCst) >= max {
        count.fetch_sub(1, Ordering::SeqCst);
        return None;
    }
    Some(Reserved(count.clone()))
}

/// What a send came to.
#[derive(Debug, PartialEq, Eq)]
pub enum Sent {
    Ok,
    /// 404 or 410: the subscription is gone and was deleted.
    Gone,
    Failed(String),
}

/// The push sender. Off (every call a no-op) when there is no VAPID subject.
#[derive(Clone)]
pub struct Push(Arc<Inner>);

struct Inner {
    store: Store,
    hub: Hub,
    vapid: Vapid,
    subject: Option<String>,
    allow_loopback: bool,
    tls: tokio_rustls::TlsConnector,
    slots: Arc<Semaphore>,
    pending: Arc<AtomicUsize>,
    max_pending: usize,
    /// When a dropped push was last logged at warn level.
    dropped_warned: Mutex<Option<Instant>>,
    timeout: Duration,
    voice: Mutex<VoiceDedupe>,
    dm: Mutex<VoiceDedupe>,
    /// When each recent failed send ended, oldest first, for Admin > Server.
    failures: Mutex<VecDeque<Instant>>,
}

const FAILURE_WINDOW: Duration = Duration::from_secs(24 * 3600);
/// Bounds the memory; a count past it shows as this many.
const MAX_FAILURES_KEPT: usize = 10_000;

impl Push {
    pub async fn new(store: Store, hub: Hub, subject: Option<String>) -> Result<Push, String> {
        Push::with_limits(store, hub, subject, SEND_TIMEOUT, MAX_PENDING).await
    }

    /// [`Push::new`] with another send timeout and queue cap, for tests.
    #[doc(hidden)]
    pub async fn with_limits(
        store: Store,
        hub: Hub,
        subject: Option<String>,
        timeout: Duration,
        max_pending: usize,
    ) -> Result<Push, String> {
        let vapid = Vapid::load(&store).await?;
        let roots =
            rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let allow_loopback = cfg!(feature = "test-push");
        if allow_loopback {
            warn!("test-push build: push may go to http://127.0.0.1 (never ship this binary)");
        }
        Ok(Push(Arc::new(Inner {
            store,
            hub,
            vapid,
            subject,
            allow_loopback,
            tls: tokio_rustls::TlsConnector::from(Arc::new(tls)),
            slots: Arc::new(Semaphore::new(PARALLEL)),
            pending: Arc::default(),
            max_pending,
            dropped_warned: Mutex::default(),
            timeout,
            voice: Mutex::default(),
            dm: Mutex::default(),
            failures: Mutex::default(),
        })))
    }

    /// Failed sends in the last 24 hours.
    pub fn failures_24h(&self) -> usize {
        let mut f = self.0.failures.lock().expect("push lock poisoned");
        while f.front().is_some_and(|t| t.elapsed() >= FAILURE_WINDOW) {
            f.pop_front();
        }
        f.len()
    }

    fn failed(&self) {
        let mut f = self.0.failures.lock().expect("push lock poisoned");
        if f.len() >= MAX_FAILURES_KEPT {
            f.pop_front();
        }
        f.push_back(Instant::now());
    }

    /// The public key, or None when push is off.
    pub fn public_key(&self) -> Option<&str> {
        self.0.subject.as_ref().map(|_| self.0.vapid.public_key())
    }

    pub fn allow_loopback(&self) -> bool {
        self.0.allow_loopback
    }

    /// Sends waiting for a slot or in flight.
    pub fn pending(&self) -> usize {
        self.0.pending.load(Ordering::SeqCst)
    }

    /// `room` got a message from `sender` that mentions `mentioned` (already filtered to people
    /// who can see the room and are not banned).
    pub async fn mentioned(&self, room: &RoomInfo, sender: &UserInfo, mentioned: &[String]) {
        if self.public_key().is_none() || mentioned.is_empty() {
            return;
        }
        let users = match self.0.store.users().await {
            Ok(u) => u,
            Err(e) => return warn!("push: store: {e}"),
        };
        let who = recipients(
            users.iter().filter(|u| mentioned.contains(&u.user_id)),
            room,
            &sender.user_id,
            |id| self.0.hub.online(id),
        );
        let payload = PushPayload {
            kind: "mention".into(),
            room_id: room.id,
            title: "Vesper".into(),
            body: format!("{} mentioned you in {}", sender.display_name, room.name),
            icon: None,
        };
        self.fan_out(
            who,
            PushKind::Mentions,
            payload,
            24 * 3600,
            "high",
            format!("m{}", room.id),
        );
    }

    /// `sender` wrote in the DM `room`. The other member gets one push per [`DM_EVERY`], and only
    /// with no socket open. It uses the device's mentions switch, and carries no message text.
    pub async fn direct_message(&self, room: &RoomInfo, sender: &UserInfo) {
        if self.public_key().is_none() {
            return;
        }
        let users = match self.0.store.users().await {
            Ok(u) => u,
            Err(e) => return warn!("push: store: {e}"),
        };
        let mut who = recipients(&users, room, &sender.user_id, |id| self.0.hub.online(id));
        {
            let mut d = self.0.dm.lock().expect("push lock poisoned");
            let now = Instant::now();
            who.retain(|id| d.take_within(room.id, id, now, DM_EVERY));
        }
        let payload = PushPayload {
            kind: "dm".into(),
            room_id: room.id,
            title: "Vesper".into(),
            body: format!("{} sent you a message", sender.display_name),
            icon: avatar_icon(sender),
        };
        self.fan_out(
            who,
            PushKind::Mentions,
            payload,
            24 * 3600,
            "high",
            format!("d{}", room.id),
        );
    }

    /// `starter` joined `room` while it was empty.
    pub async fn voice_started(&self, room: &RoomInfo, starter: &UserInfo) {
        if self.public_key().is_none() {
            return;
        }
        let users = match self.0.store.users().await {
            Ok(u) => u,
            Err(e) => return warn!("push: store: {e}"),
        };
        let mut who = recipients(&users, room, &starter.user_id, |id| self.0.hub.online(id));
        {
            let mut d = self.0.voice.lock().expect("push lock poisoned");
            let now = Instant::now();
            who.retain(|id| d.take(room.id, id, now));
        }
        let body = if room.kind == RoomKind::Dm {
            format!("{} is in voice with you", starter.display_name)
        } else {
            format!("{} started voice in {}", starter.display_name, room.name)
        };
        let payload = PushPayload {
            kind: "voice".into(),
            room_id: room.id,
            title: "Vesper".into(),
            body,
            icon: (room.kind == RoomKind::Dm)
                .then(|| avatar_icon(starter))
                .flatten(),
        };
        // A stale "started voice" is no use: the push service drops it after 10 minutes.
        self.fan_out(
            who,
            PushKind::Voice,
            payload,
            600,
            "normal",
            format!("v{}", room.id),
        );
    }

    fn fan_out(
        &self,
        who: Vec<String>,
        kind: PushKind,
        payload: PushPayload,
        ttl: u32,
        urgency: &'static str,
        topic: String,
    ) {
        if who.is_empty() {
            return;
        }
        let push = self.clone();
        tokio::spawn(async move {
            let targets = match push.0.store.push_targets(&who, kind, now_ms()).await {
                Ok(t) => t,
                Err(e) => return warn!("push: store: {e}"),
            };
            let body = serde_json::to_vec(&payload).expect("payload serialises");
            for t in targets {
                // Counted before waiting for a slot, so a backlog cannot grow without bound.
                let Some(reserved) = reserve(&push.0.pending, push.0.max_pending) else {
                    push.dropped();
                    continue;
                };
                let (push, body, topic) = (push.clone(), body.clone(), topic.clone());
                tokio::spawn(async move {
                    // Held for the whole task: a panic or a cancel still gives the place back.
                    let _reserved = reserved;
                    if let Ok(_slot) = push.0.slots.clone().acquire_owned().await {
                        push.send(&t, &body, ttl, urgency, &topic).await;
                    }
                });
            }
        });
    }

    /// A push was dropped at the queue cap: warn at most once a minute, debug otherwise.
    fn dropped(&self) {
        let mut last = self.0.dropped_warned.lock().expect("push lock poisoned");
        let now = Instant::now();
        if last.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(60)) {
            *last = Some(now);
            warn!(
                "push: {} sends pending, dropping new ones",
                self.0.max_pending
            );
        } else {
            debug!("push: {} sends pending, dropped one", self.0.max_pending);
        }
    }

    /// One push to one subscription: deletes it on 404 or 410 (gone) and on 401 or 403 (it was
    /// made for another VAPID key: a restored or lost key), stamps it on success. Logs the
    /// push service's host and status only, never the endpoint path.
    pub async fn send(
        &self,
        t: &PushTarget,
        payload: &[u8],
        ttl: u32,
        urgency: &str,
        topic: &str,
    ) -> Sent {
        let Some(subject) = &self.0.subject else {
            return Sent::Failed("push is off".into());
        };
        let exchange = async {
            let ep = parse_endpoint(&t.endpoint, self.0.allow_loopback)?;
            let addr = resolve(&ep, self.0.allow_loopback).await?;
            let body = encrypt(payload, &t.p256dh, &t.auth)?;
            let now_s = (now_ms() / 1000) as u64;
            let auth = self.0.vapid.authorization(&ep.audience, subject, now_s);
            let head = format!(
                "POST {} HTTP/1.1\r\nHost: {}\r\nAuthorization: {auth}\r\nTTL: {ttl}\r\n\
                 Urgency: {urgency}\r\nTopic: {topic}\r\nContent-Type: application/octet-stream\r\n\
                 Content-Encoding: aes128gcm\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                ep.path,
                if ep.tls {
                    ep.host.clone()
                } else {
                    format!("{}:{}", ep.host, ep.port)
                },
                body.len()
            );
            self.post(&ep, addr, head, &body)
                .await
                .map_err(|_| "push service connection failed")
        };
        let result = tokio::time::timeout(self.0.timeout, exchange)
            .await
            .unwrap_or(Err("push service timed out"));
        let host = t.endpoint.split('/').nth(2).unwrap_or("?").to_string();
        let r = match result {
            Ok(200..=299) => self
                .0
                .store
                .push_delivered(t.id, now_ms())
                .await
                .map(|_| Sent::Ok),
            Ok(404 | 410) => self
                .0
                .store
                .delete_push_subscription(t.id)
                .await
                .map(|_| Sent::Gone),
            // Bounded: the row is gone, so each subscription logs this once.
            Ok(code @ (401 | 403)) => {
                warn!(
                    "push: {host} answered {code}: the subscription is for another VAPID key, deleted"
                );
                self.0
                    .store
                    .delete_push_subscription(t.id)
                    .await
                    .map(|_| Sent::Gone)
            }
            Ok(code) => Ok(Sent::Failed(format!("{host} answered {code}"))),
            Err(e) => Ok(Sent::Failed(format!("{host}: {e}"))),
        };
        let sent = match r {
            Ok(Sent::Failed(why)) => {
                warn!("push: {why}");
                Sent::Failed(why)
            }
            Ok(s) => s,
            Err(StoreError::Db(e)) => {
                warn!("push: store: {e}");
                Sent::Failed("store".into())
            }
            Err(e) => Sent::Failed(e.to_string()),
        };
        if matches!(sent, Sent::Failed(_)) {
            self.failed();
        }
        sent
    }

    async fn post(
        &self,
        ep: &Endpoint,
        addr: SocketAddr,
        head: String,
        body: &[u8],
    ) -> std::io::Result<u16> {
        let tcp = TcpStream::connect(addr).await?;
        if !ep.tls {
            return exchange(tcp, head, body).await;
        }
        let name = rustls::pki_types::ServerName::try_from(ep.host.clone())
            .map_err(std::io::Error::other)?;
        exchange(self.0.tls.connect(name, tcp).await?, head, body).await
    }
}

/// Writes the request and reads only the status line (at most 1 KiB) of the answer.
async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    mut s: S,
    head: String,
    body: &[u8],
) -> std::io::Result<u16> {
    s.write_all(head.as_bytes()).await?;
    s.write_all(body).await?;
    s.flush().await?;
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 256];
    while !buf.windows(2).any(|w| w == b"\r\n") && buf.len() < 1024 {
        let n = s.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    status_code(&buf).ok_or_else(|| std::io::Error::other("no HTTP status line"))
}

/// `HTTP/1.x NNN ...` at the start of `buf`.
fn status_code(buf: &[u8]) -> Option<u16> {
    let line = buf.split(|&b| b == b'\r').next()?;
    let line = std::str::from_utf8(line).ok()?;
    let mut parts = line.split(' ');
    parts.next().filter(|v| v.starts_with("HTTP/1."))?;
    parts.next().filter(|c| c.len() == 3)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};
    use vesper_protocol::Role;

    fn b64(s: &str) -> Vec<u8> {
        B64.decode(s.replace([' ', '\n'], "")).unwrap()
    }

    #[test]
    fn rfc8291_appendix_a() {
        let as_key = agreement::PrivateKey::from_private_key(
            &agreement::ECDH_P256,
            &b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw"),
        )
        .unwrap();
        let ua_public = b64(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let out = encrypt_with(
            b"When I grow up, I want to be a watermelon",
            &ua_public,
            &b64("BTBZMqHH6r4Tts7J_aSIgg"),
            &salt,
            &as_key,
        )
        .unwrap();
        let header = b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z 9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml
             mlMoZIIgDll6e3vCYLocInmYWAmS6Tlz AC8wEqKK6PBru3jl7A8",
        );
        let cipher = b64(
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrd irN6GcG7sFz1y1sqLgVi1VhjVkHsUoEs
             bI_0LpXMuGvnzQ",
        );
        assert_eq!(out, [header, cipher].concat());
        // A fresh encryption differs every time (new key and salt) and has the same length.
        let a = encrypt(b"x", &ua_public, &[7; 16]).unwrap();
        let b = encrypt(b"x", &ua_public, &[7; 16]).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 86 + 1 + 1 + 16);
        assert!(encrypt(b"x", &ua_public[..64], &[7; 16]).is_err());
        assert!(encrypt(&[0; 4096], &ua_public, &[7; 16]).is_err());
    }

    #[test]
    fn vapid_jwt_is_signed_and_short_lived() {
        let v = Vapid::from_pkcs8(
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap();
        let now = 1_800_000_000;
        let h = v.authorization("https://fcm.googleapis.com", "https://vesper.example", now);
        let rest = h.strip_prefix("vapid t=").unwrap();
        let (jwt, k) = rest.split_once(", k=").unwrap();
        assert_eq!(k, v.public_key());
        let public = b64(k);
        assert_eq!((public.len(), public[0]), (65, 4));
        let (signed, sig) = jwt.rsplit_once('.').unwrap();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public)
            .verify(signed.as_bytes(), &b64(sig))
            .expect("signature verifies with the public key");
        let mut tampered = signed.to_string();
        tampered.push('x');
        assert!(
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public)
                .verify(tampered.as_bytes(), &b64(sig))
                .is_err()
        );
        let (head, claims) = signed.split_once('.').unwrap();
        let head: serde_json::Value = serde_json::from_slice(&b64(head)).unwrap();
        assert_eq!(head["alg"], "ES256");
        let c: serde_json::Value = serde_json::from_slice(&b64(claims)).unwrap();
        assert_eq!(c["aud"], "https://fcm.googleapis.com");
        assert_eq!(c["sub"], "https://vesper.example");
        let exp = c["exp"].as_u64().unwrap();
        assert!(exp > now && exp - now <= 24 * 3600, "{exp}");
        assert!(!format!("{v:?}").contains("PRIVATE"));
    }

    #[test]
    fn ssrf_endpoints_are_refused() {
        let ok = |s: &str| parse_endpoint(s, false);
        let ep = ok("https://fcm.googleapis.com/fcm/send/abc:def?x=1").unwrap();
        assert_eq!(
            (ep.host.as_str(), ep.port, ep.path.as_str()),
            ("fcm.googleapis.com", 443, "/fcm/send/abc:def?x=1")
        );
        assert_eq!(ep.audience, "https://fcm.googleapis.com");
        assert!(ok("https://updates.push.services.mozilla.com/wpush/v2/x").is_ok());
        assert!(ok("https://web.push.apple.com/QGx").is_ok());
        assert!(ok("https://wns2-by3p.notify.windows.com/w/?token=x").is_ok());
        assert!(ok("https://FCM.googleapis.com:443/fcm/send/x").is_ok());
        let long = format!("https://fcm.googleapis.com/{}", "a".repeat(MAX_ENDPOINT));
        for bad in [
            "http://fcm.googleapis.com/fcm/send/x",
            "https://127.0.0.1/x",
            "https://10.0.0.1/x",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/x",
            "https://localhost/x",
            "https://user:pw@fcm.googleapis.com/x",
            "https://fcm.googleapis.com@10.0.0.1/x",
            "https://fcm.googleapis.com:8443/x",
            "https://fcm.googleapis.com:0/x",
            "https://fcm.googleapis.com:65979/x",
            "https://fcm.googleapis.com:99999999/x",
            "https://fcm.googleapis.com?x",
            "https://fcm.googleapis.com.evil.example/x",
            "https://evilfcm.googleapis.com/x",
            "https://push.apple.com.evil/x",
            "https://xpush.apple.com/x",
            "https://notify.windows.com.attacker.net/x",
            "ftp://fcm.googleapis.com/x",
            "fcm.googleapis.com/x",
            "https:///x",
            "https://fcm.googleapis.com/a b",
            "https://fcm.googleapis.com/a\r\nX-Evil: 1",
            "",
            long.as_str(),
        ] {
            assert!(ok(bad).is_err(), "{bad}");
        }
        // The test-only loopback door is shut unless asked for, and opens only for 127.0.0.1.
        assert!(ok("http://127.0.0.1:9000/x").is_err());
        assert!(parse_endpoint("http://127.0.0.1:9000/x", true).is_ok());
        assert!(parse_endpoint("http://10.0.0.1:9000/x", true).is_err());
        assert!(parse_endpoint("http://localhost:9000/x", true).is_err());
    }

    #[test]
    fn only_public_addresses_pass() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "192.0.0.8",
            "198.18.0.1",
            "192.0.2.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12::1",
            "ff02::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
            "64:ff9b::a00:1",
            "2001:db8::1",
            "2002:a00:1::1",
            "2001:0::1",
            "::10.0.0.1",
        ] {
            assert!(!ip_is_public(bad.parse().unwrap()), "{bad}");
        }
        for good in [
            "142.250.1.1",
            "8.8.8.8",
            "2a00:1450:4001::1",
            "::ffff:8.8.8.8",
        ] {
            assert!(ip_is_public(good.parse().unwrap()), "{good}");
        }
        let a = |s: &str| -> SocketAddr { s.parse().unwrap() };
        assert_eq!(pick_addr(&[a("8.8.8.8:443")], false), Ok(a("8.8.8.8:443")));
        // One private answer among public ones refuses the whole name.
        assert!(pick_addr(&[a("8.8.8.8:443"), a("10.0.0.1:443")], false).is_err());
        assert!(pick_addr(&[], false).is_err());
        assert!(pick_addr(&[a("127.0.0.1:9")], false).is_err());
        assert!(pick_addr(&[a("127.0.0.1:9")], true).is_ok());
        assert!(pick_addr(&[a("10.0.0.1:9")], true).is_err());
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_loopback_is_refused() {
        let ep = Endpoint {
            host: "localhost".into(),
            port: 443,
            path: "/".into(),
            audience: String::new(),
            tls: true,
        };
        assert!(resolve(&ep, false).await.is_err());
    }

    #[test]
    fn status_line_is_all_that_is_read() {
        assert_eq!(status_code(b"HTTP/1.1 201 Created\r\nX: y"), Some(201));
        assert_eq!(status_code(b"HTTP/1.0 410 Gone\r\n"), Some(410));
        for bad in [&b"HTTP/2 200\r\n"[..], b"SSH-2.0", b"HTTP/1.1 2000 x", b""] {
            assert_eq!(status_code(bad), None);
        }
    }

    fn user(id: char, role: Role) -> UserInfo {
        UserInfo {
            user_id: id.to_string().repeat(32),
            username: id.to_string(),
            display_name: id.to_string(),
            accent: String::new(),
            role,
            banned: false,
            bio: String::new(),
            avatar: None,
            ui_accent: None,
        }
    }

    #[test]
    // `..Default::default()` on purpose: a RoomInfo field added by another PR (archived) must not break this.
    #[allow(clippy::needless_update)]
    fn recipients_skip_sender_online_banned_and_hidden() {
        let a = user('a', Role::Member);
        let b = user('b', Role::Member);
        let c = user('c', Role::Member);
        let d = UserInfo {
            banned: true,
            ..user('d', Role::Member)
        };
        let m = user('e', Role::Moderator);
        let room = RoomInfo {
            id: 1,
            name: "r".into(),
            position: 0,
            restricted: true,
            members: vec![
                a.user_id.clone(),
                b.user_id.clone(),
                c.user_id.clone(),
                d.user_id.clone(),
            ],
            // A field added later (archived) keeps its default here.
            ..Default::default()
        };
        let users = [
            a.clone(),
            b.clone(),
            c.clone(),
            d,
            m.clone(),
            user('f', Role::Member),
        ];
        // a sends; b has a tab open; c, and the moderator who sees every room, get it.
        let got = recipients(&users, &room, &a.user_id, |id| id == b.user_id);
        assert_eq!(got, [c.user_id.clone(), m.user_id.clone()]);
        let open = RoomInfo {
            restricted: false,
            members: vec![],
            ..room
        };
        assert_eq!(recipients(&users, &open, &a.user_id, |_| false).len(), 4);
    }

    #[tokio::test]
    async fn a_panicking_or_cancelled_send_gives_its_place_back() {
        let count = Arc::new(AtomicUsize::new(0));
        let a = reserve(&count, 2).unwrap();
        let b = reserve(&count, 2).unwrap();
        assert!(reserve(&count, 2).is_none(), "full");
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let panics = tokio::spawn(async move {
            let _a = a;
            panic!("send failed hard");
        });
        assert!(panics.await.is_err());
        let stuck = tokio::spawn(async move {
            let _b = b;
            std::future::pending::<()>().await;
        });
        tokio::task::yield_now().await;
        stuck.abort();
        let _ = stuck.await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(reserve(&count, 2).is_some());
    }

    #[test]
    fn voice_push_once_per_ten_minutes_per_room_and_person() {
        let mut d = VoiceDedupe::default();
        let t = Instant::now();
        assert!(d.take(1, "a", t));
        assert!(!d.take(1, "a", t + Duration::from_secs(60)));
        assert!(d.take(2, "a", t), "another room");
        assert!(d.take(1, "b", t), "another person");
        assert!(!d.take(1, "a", t + VOICE_EVERY - Duration::from_secs(1)));
        assert!(d.take(1, "a", t + VOICE_EVERY));
    }
}
