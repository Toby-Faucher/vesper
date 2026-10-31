//! One WebSocket per client: JSON `ClientMsg` in, `ServerMsg` out.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::time::Instant;
use vesper_protocol::{ClientMsg, ErrorCode, MAX_MSG_BYTES, ServerMsg};

use crate::rooms::{JoinReq, JoinResult, Out, Rooms};

/// Failed messages (bad key, bad JSON, ...) a socket may send before joining, then it is closed.
const MAX_PREJOIN_ERRORS: u32 = 5;

#[derive(Clone)]
pub struct WsState {
    pub rooms: Rooms,
    pub sockets: Arc<Semaphore>,
    pub join_deadline: Duration,
    pub ping_interval: Duration,
}

pub async fn upgrade(ws: WebSocketUpgrade, State(st): State<WsState>) -> Response {
    let Ok(permit) = st.sockets.clone().try_acquire_owned() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "too many connections").into_response();
    };
    // Bigger messages make recv() fail, which ends (and closes) this socket only.
    ws.max_message_size(MAX_MSG_BYTES)
        .max_frame_size(MAX_MSG_BYTES)
        .on_upgrade(move |socket| handle(socket, st, permit))
}

async fn handle(mut socket: WebSocket, st: WsState, _permit: OwnedSemaphorePermit) {
    let rooms = st.rooms;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut joined: Option<JoinResult> = None;
    let mut join_by = Instant::now() + st.join_deadline;
    let mut errors = 0;
    let mut ping = tokio::time::interval(st.ping_interval);
    let mut last_seen = Instant::now();
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(Out::Msg(m)) => {
                    let Ok(text) = serde_json::to_string(&m) else { continue };
                    if socket.send(Message::text(text)).await.is_err() {
                        break;
                    }
                }
                // Replaced, closed by the SFU, or too many errors: already out of the room.
                Some(Out::Close) | None => {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            },
            _ = tokio::time::sleep_until(join_by), if joined.is_none() => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
            _ = ping.tick() => {
                // Catches clients that vanished without a FIN (NAT drop, power loss).
                if last_seen.elapsed() > st.ping_interval * 2
                    || socket.send(Message::Ping(Default::default())).await.is_err()
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
                    (Ok(ClientMsg::Join { room, name, key, offer }), None) => {
                        let req = JoinReq { room, name, key, offer, tx: tx.clone() };
                        match rooms.join(req).await {
                            Ok(j) => {
                                joined = Some(j);
                                None
                            }
                            // Same wire code (the enum is shared with the client), clearer text. Only BadMessage:
                            // that is what a dead SFU produces; a wrong key must still say so.
                            Err(code @ ErrorCode::BadMessage) if !rooms.sfu_alive() => {
                                Some((code, "server voice engine unavailable".into()))
                            }
                            Err(code) => Some((code, describe(code).into())),
                        }
                    }
                    (Ok(ClientMsg::Join { .. }), Some(_)) => {
                        Some((ErrorCode::BadMessage, "already joined: leave first".into()))
                    }
                    (Ok(ClientMsg::Answer { sdp }), Some(j)) => {
                        rooms.answer(&j.user_id, j.session, sdp);
                        None
                    }
                    (Ok(ClientMsg::SetMuted { muted }), Some(j)) => {
                        rooms.set_muted(&j.user_id, j.session, muted);
                        None
                    }
                    (Ok(ClientMsg::Leave), Some(j)) => {
                        rooms.leave(&j.user_id, j.session);
                        joined = None;
                        join_by = Instant::now() + st.join_deadline;
                        None
                    }
                    (Ok(_), None) => Some((ErrorCode::BadMessage, "join first".into())),
                };
                if let Some((code, message)) = error {
                    let _ = tx.send(Out::Msg(ServerMsg::Error { code, message }));
                    if joined.is_none() {
                        errors += 1;
                        if errors >= MAX_PREJOIN_ERRORS {
                            let _ = tx.send(Out::Close);
                        }
                    }
                }
            }
        }
    }
    if let Some(j) = joined {
        rooms.leave(&j.user_id, j.session);
    }
}

fn describe(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::BadKey => "wrong key",
        ErrorCode::RoomFull => "the room is full",
        ErrorCode::BadMessage => "invalid room, name or offer",
    }
}
