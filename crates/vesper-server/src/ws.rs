//! One WebSocket per tab: JSON `ClientMsg` in, `ServerMsg` out. The upgrade needs our Origin
//! and a live session cookie; the socket then belongs to that session (see `hub.rs`).

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::time::Instant;
use vesper_protocol::{
    CLOSE_SESSION_ENDED, ClientMsg, ErrorCode, MAX_MSG_BYTES, Me, ServerMsg, UserInfo,
};
use vesper_store::{StoreError, now_ms};

use crate::AppState;
use crate::api_accounts::ApiError;
use crate::api_rooms::{can_see, listed};
use crate::auth::{cookie_token, hash_token};
use crate::hub::{Inbox, Outbox, outbox};
use crate::limits::{Bucket, Kind};
use crate::voice::JoinResult;

/// Close code when the server cannot build `Hello` (1011, "internal error").
const CLOSE_INTERNAL: u16 = 1011;
/// Close code for a socket that keeps sending past its rate limit (1008, "policy violation").
pub const CLOSE_RATE_LIMITED: u16 = 1008;

/// Inbound budget of one socket: a burst of 20 messages, then 5 a second. `answer` is exempt,
/// since the server asks for it.
const MSG_BURST: f64 = 20.0;
const MSG_REFILL: Duration = Duration::from_millis(200);
/// Refused messages a socket may send (20, then 1 a second) before it is closed.
const REFUSED_BURST: f64 = 20.0;
const REFUSED_REFILL: Duration = Duration::from_secs(1);

struct Budget {
    msgs: Bucket,
    refused: Bucket,
}

#[derive(Debug, PartialEq)]
enum Admit {
    Yes,
    RateLimited,
    Close,
}

impl Budget {
    fn new(now: std::time::Instant) -> Budget {
        Budget {
            msgs: Bucket::full(MSG_BURST, now),
            refused: Bucket::full(REFUSED_BURST, now),
        }
    }

    fn admit(&mut self, now: std::time::Instant) -> Admit {
        if self.msgs.take(MSG_BURST, MSG_REFILL, now).is_ok() {
            Admit::Yes
        } else if self
            .refused
            .take(REFUSED_BURST, REFUSED_REFILL, now)
            .is_ok()
        {
            Admit::RateLimited
        } else {
            Admit::Close
        }
    }
}

#[derive(Clone)]
pub struct WsState {
    pub app: AppState,
    pub sockets: Arc<Semaphore>,
    pub ping_interval: Duration,
}

pub async fn upgrade(ws: WebSocketUpgrade, State(st): State<WsState>, h: HeaderMap) -> Response {
    // Browsers send Origin on every upgrade; another site's page must not ride our cookie.
    if !crate::auth::origin_ok(&h, &st.app.cfg.origin) {
        return (StatusCode::FORBIDDEN, "request from another origin").into_response();
    }
    let Some(hash) = cookie_token(&h, st.app.cfg.secure_cookies).and_then(|t| hash_token(&t))
    else {
        return (StatusCode::UNAUTHORIZED, "not signed in").into_response();
    };
    let (session_id, user) = match st.app.store.session_user(&hash, now_ms()).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "not signed in").into_response(),
        Err(e) => return ApiError::from(e).into_response(),
    };
    let Ok(permit) = st.sockets.clone().try_acquire_owned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "too many connections").into_response();
    };
    // Bigger messages make recv() fail, which ends (and closes) this socket only.
    ws.max_message_size(MAX_MSG_BYTES)
        .max_frame_size(MAX_MSG_BYTES)
        .on_upgrade(move |socket| handle(socket, st, user, session_id, hash, permit))
}

/// `Hello` for `user`: everyone, the rooms in this user's list, and their voice presence.
async fn hello(st: &WsState, user: &UserInfo, session_id: i64) -> Result<ServerMsg, StoreError> {
    let users = st.app.store.users().await?;
    let mut rooms = st.app.store.rooms().await?;
    rooms.retain(|r| listed(user, r));
    let voice = st.app.voice.peers(|id| rooms.iter().any(|r| r.id == id));
    let ids: Vec<i64> = rooms.iter().filter(|r| !r.archived).map(|r| r.id).collect();
    let unread = st.app.store.unread(&user.user_id, &ids).await?;
    Ok(ServerMsg::Hello {
        me: Me {
            user: user.clone(),
            session_id,
            server: st.app.cfg.server_build(),
        },
        users,
        rooms,
        voice,
        unread,
    })
}

/// Send `m`, unless the socket is told to close first. False when the socket is done.
async fn send(
    socket: &mut WebSocket,
    close: &mut watch::Receiver<Option<u16>>,
    m: Message,
) -> bool {
    tokio::select! {
        r = socket.send(m) => r.is_ok(),
        _ = close.changed() => false,
    }
}

async fn handle(
    mut socket: WebSocket,
    st: WsState,
    user: UserInfo,
    session_id: i64,
    token_hash: [u8; 32],
    _permit: OwnedSemaphorePermit,
) {
    let (
        tx,
        Inbox {
            mut queue,
            mut close,
        },
    ) = outbox();
    // Added before Hello is read, so no event in between is lost. Events queued meanwhile go
    // out after Hello, which is written straight to the socket.
    st.app.hub.wait_gate().await;
    let conn = st.app.hub.add(user.clone(), session_id, tx.clone());
    let user_id = user.user_id.clone();
    let voice = st.app.voice.clone();
    let mut joined: Option<JoinResult> = None;
    // The SFU id of this socket's connection check, if one runs.
    let mut check: Option<String> = None;
    // A logout or ban between the upgrade's check and `add` found no socket to close, so check
    // again now that later ones will. Nothing queued meanwhile is sent: the close wins.
    match st.app.store.session_user(&token_hash, now_ms()).await {
        Ok(Some((id, _))) if id == session_id => match hello(&st, &user, session_id).await {
            Ok(h) => {
                let text = serde_json::to_string(&h).expect("ServerMsg serialises");
                if !send(&mut socket, &mut close, Message::text(text)).await {
                    st.app.hub.remove(conn);
                    return;
                }
            }
            Err(e) => {
                tracing::error!("hello: store: {e}");
                tx.close(CLOSE_INTERNAL);
            }
        },
        Ok(_) => tx.close(CLOSE_SESSION_ENDED),
        Err(e) => {
            tracing::error!("socket session check: store: {e}");
            tx.close(CLOSE_INTERNAL);
        }
    }
    let mut budget = Budget::new(std::time::Instant::now());
    let mut ping = tokio::time::interval(st.ping_interval);
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            biased;
            _ = close.changed() => break,
            Some(m) = queue.recv() => {
                let text = serde_json::to_string(&m).expect("ServerMsg serialises");
                if !send(&mut socket, &mut close, Message::text(text)).await {
                    break;
                }
            }
            _ = ping.tick() => {
                // Catches clients that vanished without a FIN (NAT drop, power loss).
                if last_seen.elapsed() > st.ping_interval * 2
                    || !send(&mut socket, &mut close, Message::Ping(Default::default())).await
                {
                    break;
                }
            }
            msg = socket.recv() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => {
                        last_seen = Instant::now();
                        continue;
                    }
                };
                last_seen = Instant::now();
                let parsed = serde_json::from_str::<ClientMsg>(&text);
                if !matches!(parsed, Ok(ClientMsg::Answer { .. })) {
                    match budget.admit(std::time::Instant::now()) {
                        Admit::Yes => {}
                        Admit::RateLimited => {
                            let message = describe(ErrorCode::RateLimited).into();
                            tx.send(ServerMsg::Error { code: ErrorCode::RateLimited, message });
                            continue;
                        }
                        Admit::Close => {
                            tx.close(CLOSE_RATE_LIMITED);
                            continue;
                        }
                    }
                }
                let error = match (parsed, &joined) {
                    (Err(e), _) => Some((ErrorCode::BadMessage, format!("bad message: {e}"))),
                    (Ok(ClientMsg::Join { .. }), Some(j)) if voice.is_current(&j.user_id, j.session) => {
                        Some((ErrorCode::BadMessage, "already joined: leave first".into()))
                    }
                    // Not joined, or the session ended under us (kick, other tab, SFU).
                    (Ok(ClientMsg::Join { room_id, offer }), _) => {
                        match join(&st, &user_id, room_id, offer, &tx).await {
                            Ok(j) => {
                                joined = Some(j);
                                None
                            }
                            Err(e) => Some(e),
                        }
                    }
                    (Ok(ClientMsg::Check { offer }), _) => {
                        if let Some(old) = check.take() {
                            voice.end_check(&old);
                        }
                        match voice.check(offer, tx.clone()).await {
                            Ok(j) => {
                                check = Some(j.user_id);
                                None
                            }
                            Err(code) => Some(voice_error(&st, code)),
                        }
                    }
                    (Ok(ClientMsg::Typing { room_id }), _) => {
                        typing(&st, &user_id, room_id, &tx).await;
                        None
                    }
                    (Ok(ClientMsg::Answer { sdp }), Some(j)) => {
                        voice.answer(&j.user_id, j.session, sdp);
                        None
                    }
                    (Ok(ClientMsg::SetVoiceState { muted, deafened }), Some(j)) => {
                        voice.set_state(&j.user_id, j.session, muted, deafened);
                        None
                    }
                    // Ends the call and the connection check, whichever runs.
                    (Ok(ClientMsg::Leave), j) if j.is_some() || check.is_some() => {
                        if let Some(j) = j {
                            voice.leave(&j.user_id, j.session);
                        }
                        if let Some(c) = check.take() {
                            voice.end_check(&c);
                        }
                        joined = None;
                        None
                    }
                    (Ok(_), _) => Some((ErrorCode::BadMessage, "join first".into())),
                };
                if let Some((code, message)) = error {
                    tx.send(ServerMsg::Error { code, message });
                }
            }
        }
    }
    // Leave first, so the room sees PeerLeft now, not after the close handshake below.
    st.app.hub.remove(conn);
    if let Some(j) = joined {
        voice.leave(&j.user_id, j.session);
    }
    if let Some(c) = check {
        voice.end_check(&c);
    }
    let code = *close.borrow();
    if let Some(code) = code {
        let frame = CloseFrame {
            code,
            reason: Default::default(),
        };
        // Wait for the client's close reply, so the frame is not lost to a TCP reset. Bounded: a
        // client that stopped reading cannot hold the task.
        let _ = tokio::time::timeout(Duration::from_secs(1), async {
            if socket.send(Message::Close(Some(frame))).await.is_ok() {
                while let Some(Ok(m)) = socket.recv().await {
                    if matches!(m, Message::Close(_)) {
                        break;
                    }
                }
            }
        })
        .await;
    }
}

/// Look the user and the room up, so the access check in `Voice::join` uses the current role,
/// ban and member list (not the ones at connect), then join voice.
async fn join(
    st: &WsState,
    user_id: &str,
    room_id: i64,
    offer: String,
    tx: &Outbox,
) -> Result<JoinResult, (ErrorCode, String)> {
    let internal = |e: vesper_store::StoreError| {
        tracing::error!("store: {e}");
        (ErrorCode::BadMessage, "server error".to_string())
    };
    let user = st.app.store.user(user_id).await.map_err(internal)?;
    let rooms = st.app.store.rooms().await.map_err(internal)?;
    // An unknown room gets the same refusal as a hidden one, so a join cannot probe room ids.
    let Some(room) = rooms.iter().find(|r| r.id == room_id) else {
        return Err(voice_error(st, ErrorCode::Forbidden));
    };
    let j = st
        .app
        .voice
        .join(&user, room, offer, tx.clone())
        .await
        .map_err(|code| voice_error(st, code))?;
    if j.first {
        let (push, room) = (st.app.push.clone(), room.clone());
        tokio::spawn(async move { push.voice_started(&room, &user).await });
    }
    Ok(j)
}

/// `typing` to everyone else who can see the room. Never stored and never answered: over the
/// limit (1 per 3 s per room), an unknown or hidden room, or a store error (logged) drops it.
async fn typing(st: &WsState, user_id: &str, room_id: i64, tx: &Outbox) {
    // Fresh user and room, so a role or member list change counts (like `join`).
    let (user, rooms) = match tokio::join!(st.app.store.user(user_id), st.app.store.rooms()) {
        (Ok(user), Ok(rooms)) => (user, rooms),
        (Err(e), _) | (_, Err(e)) => {
            tracing::warn!("typing: store: {e}");
            return;
        }
    };
    let Some(room) = rooms.iter().find(|r| r.id == room_id && can_see(&user, r)) else {
        return;
    };
    // Keyed only after the room checks out, so made-up room ids cannot grow the limiter's map.
    let key = format!("{user_id} {room_id}");
    if st
        .app
        .limits
        .check(Kind::Typing, &key, st.app.limits.now())
        .is_err()
    {
        return;
    }
    let msg = ServerMsg::Typing {
        room_id,
        user_id: user_id.to_string(),
    };
    st.app.hub.broadcast_room_except(room, msg, &[tx]);
}

fn voice_error(st: &WsState, code: ErrorCode) -> (ErrorCode, String) {
    match code {
        // Same wire code (the enum is shared with the client), clearer text: that is what a dead
        // SFU produces.
        ErrorCode::BadMessage if !st.app.voice.sfu_alive() => {
            (code, "server voice engine unavailable".into())
        }
        code => (code, describe(code).into()),
    }
}

fn describe(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::RoomFull => "the room is full",
        ErrorCode::BadMessage => "invalid offer",
        ErrorCode::Forbidden => "not allowed",
        ErrorCode::RateLimited => "too many requests",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_limits_then_closes() {
        let now = std::time::Instant::now();
        let mut b = Budget::new(now);
        for _ in 0..20 {
            assert_eq!(b.admit(now), Admit::Yes);
        }
        for _ in 0..20 {
            assert_eq!(b.admit(now), Admit::RateLimited);
        }
        assert_eq!(b.admit(now), Admit::Close);
        // 5 a second once the burst is spent.
        let later = now + Duration::from_secs(1);
        for _ in 0..5 {
            assert_eq!(b.admit(later), Admit::Yes);
        }
        assert_eq!(b.admit(later), Admit::RateLimited);
    }
}
