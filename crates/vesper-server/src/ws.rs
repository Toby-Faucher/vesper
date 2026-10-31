//! One WebSocket per client: JSON `ClientMsg` in, `ServerMsg` out.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use tokio::sync::mpsc;
use vesper_protocol::{ClientMsg, ErrorCode, MAX_MSG_BYTES, ServerMsg};

use crate::rooms::{JoinReq, JoinResult, Out, Rooms};

pub async fn upgrade(ws: WebSocketUpgrade, State(rooms): State<Rooms>) -> Response {
    // Bigger messages make recv() fail, which ends (and closes) this socket only.
    ws.max_message_size(MAX_MSG_BYTES)
        .max_frame_size(MAX_MSG_BYTES)
        .on_upgrade(move |socket| handle(socket, rooms))
}

async fn handle(mut socket: WebSocket, rooms: Rooms) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut joined: Option<JoinResult> = None;
    loop {
        tokio::select! {
            out = rx.recv() => match out {
                Some(Out::Msg(m)) => {
                    let text = serde_json::to_string(&m).expect("ServerMsg serialises");
                    if socket.send(Message::text(text)).await.is_err() {
                        break;
                    }
                }
                // Replaced by a newer session or closed by the SFU: already out of the room.
                Some(Out::Close) | None => {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            },
            msg = socket.recv() => {
                let text = match msg {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                let msg = match serde_json::from_str::<ClientMsg>(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        send_error(&tx, ErrorCode::BadMessage, &format!("bad message: {e}"));
                        continue;
                    }
                };
                match (msg, &joined) {
                    (ClientMsg::Join { room, name, key, offer }, None) => {
                        let req = JoinReq { room, name, key, offer, tx: tx.clone() };
                        match rooms.join(req).await {
                            Ok(j) => joined = Some(j),
                            Err(code) => send_error(&tx, code, describe(code)),
                        }
                    }
                    (ClientMsg::Join { .. }, Some(_)) => {
                        send_error(&tx, ErrorCode::BadMessage, "already joined: leave first");
                    }
                    (ClientMsg::Answer { sdp }, Some(j)) => rooms.answer(&j.user_id, j.session, sdp),
                    (ClientMsg::SetMuted { muted }, Some(j)) => {
                        rooms.set_muted(&j.user_id, j.session, muted);
                    }
                    (ClientMsg::Leave, Some(j)) => {
                        rooms.leave(&j.user_id, j.session);
                        joined = None;
                    }
                    (_, None) => send_error(&tx, ErrorCode::BadMessage, "join first"),
                }
            }
        }
    }
    if let Some(j) = joined {
        rooms.leave(&j.user_id, j.session);
    }
}

fn send_error(tx: &mpsc::UnboundedSender<Out>, code: ErrorCode, message: &str) {
    let _ = tx.send(Out::Msg(ServerMsg::Error {
        code,
        message: message.into(),
    }));
}

fn describe(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::BadKey => "wrong key",
        ErrorCode::RoomFull => "the room is full",
        ErrorCode::BadMessage => "invalid room, name or offer",
    }
}
