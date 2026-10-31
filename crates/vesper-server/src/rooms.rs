//! Presence: who is in which room, muted or not. Media lives in the SFU; this module keeps the
//! two in step and broadcasts presence changes to everyone else in the room.
//!
//! A user is identified by display name for now (stable ids arrive with accounts), so a second
//! join with the same name replaces the first session. Every join gets a fresh session number;
//! messages from a socket whose session is no longer current are ignored, because the SFU keys
//! sessions by user alone and would otherwise apply them to the new session.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;
use vesper_protocol::{ErrorCode, MAX_NAME_CHARS, Peer, ServerMsg};
use vesper_sfu::{JoinError, SfuEvent, SfuHandle};

/// How long a rejoin waits for the SFU to confirm the old session closed.
const REPLACE_TIMEOUT: Duration = Duration::from_secs(2);

/// What [`Rooms`] sends to a socket.
#[derive(Debug)]
pub enum Out {
    Msg(ServerMsg),
    /// The session is over (replaced or closed by the SFU): close the socket.
    Close,
}

pub struct JoinReq {
    pub room: String,
    pub name: String,
    pub key: String,
    pub offer: String,
    /// Where this session's messages go: `Joined` first, then presence and SFU offers.
    pub tx: mpsc::UnboundedSender<Out>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinResult {
    pub user_id: String,
    pub session: u64,
}

#[derive(Clone)]
pub struct Rooms(Arc<Inner>);

struct Inner {
    sfu: SfuHandle,
    key_hash: [u8; 32],
    max_room_size: usize,
    next_session: AtomicU64,
    /// Serialises joins, so a rejoin finishes closing the old session before anyone else joins
    /// and the room-size check cannot race another join.
    join_lock: tokio::sync::Mutex<()>,
    members: Mutex<HashMap<String, Member>>,
}

struct Member {
    room: String,
    name: String,
    muted: bool,
    session: u64,
    tx: mpsc::UnboundedSender<Out>,
    /// Fires once the SFU reports this session closed.
    sfu_closed: Option<oneshot::Receiver<()>>,
}

impl Member {
    fn peer(&self, user_id: &str) -> Peer {
        Peer {
            user_id: user_id.into(),
            name: self.name.clone(),
            muted: self.muted,
        }
    }
}

impl Rooms {
    pub fn new(sfu: SfuHandle, dev_key: &str, max_room_size: usize) -> Rooms {
        Rooms(Arc::new(Inner {
            sfu,
            key_hash: Sha256::digest(dev_key).into(),
            max_room_size,
            next_session: AtomicU64::new(1),
            join_lock: tokio::sync::Mutex::new(()),
            members: Mutex::default(),
        }))
    }

    /// Check the key and names, replace any earlier session of the same user, start the SFU
    /// session and enter the room. On success `Joined` has been queued on `req.tx`.
    pub async fn join(&self, req: JoinReq) -> Result<JoinResult, ErrorCode> {
        if !self.key_ok(&req.key) {
            return Err(ErrorCode::BadKey);
        }
        let (Some(room), Some(name)) = (valid_name(&req.room), valid_name(&req.name)) else {
            return Err(ErrorCode::BadMessage);
        };
        let user_id = user_id(&name);
        let _serial = self.0.join_lock.lock().await;

        let old = self.remove(&user_id, None);
        if let Some(mut old) = old {
            let _ = old.tx.send(Out::Close);
            self.0.sfu.leave(&user_id);
            if let Some(closed) = old.sfu_closed.take()
                && tokio::time::timeout(REPLACE_TIMEOUT, closed).await.is_err()
            {
                warn!("old session of {user_id} did not close in {REPLACE_TIMEOUT:?}");
            }
        }

        if self.room_size(&room) >= self.0.max_room_size {
            return Err(ErrorCode::RoomFull);
        }
        let ok = match self
            .0
            .sfu
            .join(room.clone(), user_id.clone(), req.offer)
            .await
        {
            Ok(ok) => ok,
            Err(JoinError::BadOffer(e)) => {
                warn!("join by {user_id}: {e}");
                return Err(ErrorCode::BadMessage);
            }
            Err(JoinError::Shutdown) => {
                tracing::error!("join by {user_id}: the SFU has shut down");
                return Err(ErrorCode::BadMessage);
            }
        };

        let session = self.0.next_session.fetch_add(1, Ordering::Relaxed);
        let (closed_tx, closed_rx) = oneshot::channel();
        {
            let mut members = self.0.members.lock().unwrap();
            let member = Member {
                room: room.clone(),
                name,
                muted: false,
                session,
                tx: req.tx.clone(),
                sfu_closed: Some(closed_rx),
            };
            let peers = members
                .iter()
                .filter(|(_, m)| m.room == room)
                .map(|(id, m)| m.peer(id))
                .collect();
            // Queued under the lock, so every later presence message for this room follows it.
            let _ = req.tx.send(Out::Msg(ServerMsg::Joined {
                user_id: user_id.clone(),
                answer: ok.answer_sdp,
                peers,
            }));
            broadcast(
                &members,
                &room,
                ServerMsg::PeerJoined {
                    peer: member.peer(&user_id),
                },
            );
            members.insert(user_id.clone(), member);
        }

        let (rooms, mut events, uid) = (self.clone(), ok.events, user_id.clone());
        tokio::spawn(async move {
            while let Some(SfuEvent::Offer(sdp)) = events.recv().await {
                let _ = req.tx.send(Out::Msg(ServerMsg::Offer { sdp }));
            }
            // SfuEvent::Closed (or the SFU went away). Close the socket if this session was
            // still current, i.e. the SFU ended it rather than a leave or rejoin.
            let _ = closed_tx.send(());
            if rooms.leave(&uid, session) {
                let _ = req.tx.send(Out::Close);
            }
        });
        Ok(JoinResult { user_id, session })
    }

    /// Leave if `session` is still `user_id`'s current one; true if it was.
    pub fn leave(&self, user_id: &str, session: u64) -> bool {
        let left = self.remove(user_id, Some(session)).is_some();
        if left {
            self.0.sfu.leave(user_id);
        }
        left
    }

    /// Set the session's mute state and tell the room (ignored if the session was replaced).
    pub fn set_muted(&self, user_id: &str, session: u64, muted: bool) {
        let mut members = self.0.members.lock().unwrap();
        let Some(m) = members.get_mut(user_id) else {
            return;
        };
        if m.session != session || m.muted == muted {
            return;
        }
        m.muted = muted;
        let room = m.room.clone();
        broadcast(
            &members,
            &room,
            ServerMsg::PeerMuted {
                user_id: user_id.into(),
                muted,
            },
        );
    }

    /// Pass an SDP answer to the SFU, unless `session` has been replaced: the SFU cannot tell
    /// which offer an answer belongs to.
    pub fn answer(&self, user_id: &str, session: u64, sdp: String) {
        let current = self
            .0
            .members
            .lock()
            .unwrap()
            .get(user_id)
            .is_some_and(|m| m.session == session);
        if current {
            self.0.sfu.answer(user_id, sdp);
        }
    }

    fn remove(&self, user_id: &str, session: Option<u64>) -> Option<Member> {
        let mut members = self.0.members.lock().unwrap();
        if session.is_some_and(|s| members.get(user_id).is_none_or(|m| m.session != s)) {
            return None;
        }
        let m = members.remove(user_id)?;
        broadcast(
            &members,
            &m.room,
            ServerMsg::PeerLeft {
                user_id: user_id.into(),
            },
        );
        Some(m)
    }

    fn room_size(&self, room: &str) -> usize {
        let members = self.0.members.lock().unwrap();
        members.values().filter(|m| m.room == room).count()
    }

    /// Compares SHA-256 digests in constant time, so neither content nor length leaks.
    fn key_ok(&self, key: &str) -> bool {
        let got: [u8; 32] = Sha256::digest(key).into();
        got.iter()
            .zip(self.0.key_hash)
            .fold(0, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

fn broadcast(members: &HashMap<String, Member>, room: &str, msg: ServerMsg) {
    for m in members.values().filter(|m| m.room == room) {
        let _ = m.tx.send(Out::Msg(msg.clone()));
    }
}

/// Trimmed, 1..=MAX_NAME_CHARS chars, no control characters.
fn valid_name(s: &str) -> Option<String> {
    let s = s.trim();
    let n = s.chars().count();
    ((1..=MAX_NAME_CHARS).contains(&n) && !s.chars().any(char::is_control)).then(|| s.into())
}

/// First 32 hex chars of sha256(name): lowercase hex, so it is a valid MSID stream id
/// (RFC 8830) and the client can key remote streams on it.
fn user_id(name: &str) -> String {
    Sha256::digest(name)[..16]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
