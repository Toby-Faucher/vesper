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
use vesper_protocol::{ClientMsg, ErrorCode, MAX_MSG_BYTES, Me, ServerMsg, UserInfo};

use crate::AppState;
use crate::api_accounts::session_of;
use crate::api_rooms::can_see;
use crate::hub::{Inbox, outbox};
use crate::rooms::{JoinReq, JoinResult, Rooms};

/// Close code when the server cannot build `Hello` (1011, "internal error").
const CLOSE_INTERNAL: u16 = 1011;

#[derive(Clone)]
pub struct WsState {
    pub app: AppState,
    pub rooms: Rooms,
    pub sockets: Arc<Semaphore>,
    pub ping_interval: Duration,
}

pub async fn upgrade(ws: WebSocketUpgrade, State(st): State<WsState>, h: HeaderMap) -> Response {
    // Browsers send Origin on every upgrade; another site's page must not ride our cookie.
    if !crate::auth::origin_ok(&h, &st.app.cfg.origin) {
        return (StatusCode::FORBIDDEN, "request from another origin").into_response();
    }
    let (session_id, user) = match session_of(&h, &st.app).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::UNAUTHORIZED, "not signed in").into_response(),
        Err(e) => return e.into_response(),
    };
    let Ok(permit) = st.sockets.clone().try_acquire_owned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "too many connections").into_response();
    };
    // Bigger messages make recv() fail, which ends (and closes) this socket only.
    ws.max_message_size(MAX_MSG_BYTES)
        .max_frame_size(MAX_MSG_BYTES)
        .on_upgrade(move |socket| handle(socket, st, user, session_id, permit))
}

/// `Hello` for `user`: everyone, and the rooms and voice presence this user can see.
async fn hello(st: &WsState, user: &UserInfo, session_id: i64) -> Option<ServerMsg> {
    let users = st.app.store.users().await.ok()?;
    let mut rooms = st.app.store.rooms().await.ok()?;
    rooms.retain(|r| can_see(user, r));
    let voice = st.rooms.peers(|id| rooms.iter().any(|r| r.id == id));
    Some(ServerMsg::Hello {
        me: Me {
            user: user.clone(),
            session_id,
        },
        users,
        rooms,
        voice,
        unread: vec![],
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
    let conn = st.app.hub.add(user.clone(), session_id, tx.clone());
    let user_id = user.user_id.clone();
    let mut joined: Option<JoinResult> = None;
    match hello(&st, &user, session_id).await {
        Some(h) => {
            let text = serde_json::to_string(&h).expect("ServerMsg serialises");
            if !send(&mut socket, &mut close, Message::text(text)).await {
                st.app.hub.remove(conn);
                return;
            }
        }
        None => tx.close(CLOSE_INTERNAL),
    }
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
                let error = match (serde_json::from_str::<ClientMsg>(&text), &joined) {
                    (Err(e), _) => Some((ErrorCode::BadMessage, format!("bad message: {e}"))),
                    (Ok(ClientMsg::Join { room_id, offer }), None) => {
                        match join(&st, &user_id, room_id, offer, &tx).await {
                            Ok(j) => {
                                joined = Some(j);
                                None
                            }
                            Err(e) => Some(e),
                        }
                    }
                    (Ok(ClientMsg::Join { .. }), Some(_)) => {
                        Some((ErrorCode::BadMessage, "already joined: leave first".into()))
                    }
                    // shortcut: no connectivity check yet; Task 9 adds it (a hidden SFU session).
                    (Ok(ClientMsg::Check { .. }), _) => {
                        Some((ErrorCode::BadMessage, "connection check is not available yet".into()))
                    }
                    // Typing goes out with text chat (Task 13).
                    (Ok(ClientMsg::Typing { .. }), _) => None,
                    (Ok(ClientMsg::Answer { sdp }), Some(j)) => {
                        st.rooms.answer(&j.user_id, j.session, sdp);
                        None
                    }
                    (Ok(ClientMsg::SetVoiceState { muted, deafened }), Some(j)) => {
                        st.rooms.set_state(&j.user_id, j.session, muted, deafened);
                        None
                    }
                    (Ok(ClientMsg::Leave), Some(j)) => {
                        st.rooms.leave(&j.user_id, j.session);
                        joined = None;
                        None
                    }
                    (Ok(_), None) => Some((ErrorCode::BadMessage, "join first".into())),
                };
                if let Some((code, message)) = error {
                    tx.send(ServerMsg::Error { code, message });
                }
            }
        }
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
    st.app.hub.remove(conn);
    if let Some(j) = joined {
        st.rooms.leave(&j.user_id, j.session);
    }
}

/// Look the room up and check the user may see it, with their current role (not the one at
/// connect), then join voice.
async fn join(
    st: &WsState,
    user_id: &str,
    room_id: i64,
    offer: String,
    tx: &crate::hub::Outbox,
) -> Result<JoinResult, (ErrorCode, String)> {
    let internal = |e: vesper_store::StoreError| {
        tracing::error!("store: {e}");
        (ErrorCode::BadMessage, "server error".to_string())
    };
    let user = st.app.store.user(user_id).await.map_err(internal)?;
    let rooms = st.app.store.rooms().await.map_err(internal)?;
    let Some(room) = rooms.iter().find(|r| r.id == room_id) else {
        return Err((ErrorCode::BadMessage, "no such room".into()));
    };
    if !can_see(&user, room) {
        return Err((ErrorCode::Forbidden, describe(ErrorCode::Forbidden).into()));
    }
    let req = JoinReq {
        room_id,
        user_id: user.user_id,
        name: user.display_name,
        offer,
        tx: tx.clone(),
    };
    st.rooms.join(req).await.map_err(|code| match code {
        // Same wire code (the enum is shared with the client), clearer text: that is what a dead
        // SFU produces.
        ErrorCode::BadMessage if !st.rooms.sfu_alive() => {
            (code, "server voice engine unavailable".into())
        }
        code => (code, describe(code).into()),
    })
}

fn describe(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::RoomFull => "the room is full",
        ErrorCode::BadMessage => "invalid offer",
        ErrorCode::Forbidden => "not allowed",
        ErrorCode::RateLimited => "too many requests",
    }
}
