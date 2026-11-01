//! Voice presence: who is in which room, muted or deafened. Media lives in the SFU; this module
//! keeps the two in step and broadcasts presence changes to everyone else in the room.
//!
//! A user is their account id, so a second join by the same account (another tab) replaces the
//! first session. Every join gets a fresh session number;
//! messages from a socket whose session is no longer current are ignored, because the SFU keys
//! sessions by user alone and would otherwise apply them to the new session.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::warn;
use vesper_protocol::{ErrorCode, Peer, ServerMsg};
use vesper_sfu::{JoinError, SfuEvent, SfuHandle};

use crate::hub::Outbox;

/// How long a rejoin waits for the SFU to confirm the old session closed.
const REPLACE_TIMEOUT: Duration = Duration::from_secs(2);

/// Close code sent when a voice session ends under a socket (replaced or closed by the SFU).
const CLOSE_VOICE_ENDED: u16 = 1000;

pub struct JoinReq {
    pub room_id: i64,
    /// The account id.
    pub user_id: String,
    /// The display name shown to peers.
    pub name: String,
    pub offer: String,
    /// Where this session's messages go: `Joined` first, then presence and SFU offers.
    pub tx: Outbox,
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
    max_room_size: usize,
    next_session: AtomicU64,
    /// Serialises joins, so a rejoin finishes closing the old session before anyone else joins
    /// and the room-size check cannot race another join.
    join_lock: tokio::sync::Mutex<()>,
    members: Mutex<HashMap<String, Member>>,
}

struct Member {
    room: i64,
    name: String,
    muted: bool,
    deafened: bool,
    session: u64,
    tx: Outbox,
    /// Fires once the SFU reports this session closed.
    sfu_closed: Option<oneshot::Receiver<()>>,
}

impl Member {
    fn peer(&self, user_id: &str) -> Peer {
        Peer {
            user_id: user_id.into(),
            name: self.name.clone(),
            room_id: self.room,
            muted: self.muted,
            deafened: self.deafened,
            force_muted: false,
        }
    }
}

impl Rooms {
    pub fn sfu_alive(&self) -> bool {
        self.0.sfu.is_alive()
    }

    pub fn new(sfu: SfuHandle, max_room_size: usize) -> Rooms {
        Rooms(Arc::new(Inner {
            sfu,
            max_room_size,
            next_session: AtomicU64::new(1),
            join_lock: tokio::sync::Mutex::new(()),
            members: Mutex::default(),
        }))
    }

    /// Replace any earlier session of the same user, start the SFU session and enter the room.
    /// The caller has checked that the user may see the room. On success `Joined` has been
    /// queued on `req.tx`.
    pub async fn join(&self, req: JoinReq) -> Result<JoinResult, ErrorCode> {
        let JoinReq {
            room_id: room,
            user_id,
            name,
            offer,
            tx,
        } = req;
        let _serial = self.0.join_lock.lock().await;

        let old = self.remove(&user_id, None);
        if let Some(mut old) = old {
            old.tx.close(CLOSE_VOICE_ENDED);
            if let Some(closed) = old.sfu_closed.take()
                && tokio::time::timeout(REPLACE_TIMEOUT, closed).await.is_err()
            {
                warn!("old session of {user_id} did not close in {REPLACE_TIMEOUT:?}");
            }
        }

        if self.room_size(room) >= self.0.max_room_size {
            return Err(ErrorCode::RoomFull);
        }
        let ok = match self
            .0
            .sfu
            .join(format!("room-{room}"), user_id.clone(), offer)
            .await
        {
            Ok(ok) => ok,
            Err(JoinError::BadOffer(_)) => {
                // Not the parser's text: it can echo SDP (addresses) from an untrusted client.
                warn!("join by {user_id}: bad sdp offer");
                return Err(ErrorCode::BadMessage);
            }
            Err(JoinError::Shutdown) => {
                // The SFU thread logs its own death once at error level.
                warn!("join by {user_id}: the SFU has shut down");
                return Err(ErrorCode::BadMessage);
            }
        };

        let session = self.0.next_session.fetch_add(1, Ordering::Relaxed);
        let (closed_tx, closed_rx) = oneshot::channel();
        {
            let mut members = self.0.members.lock().expect("members lock poisoned");
            let member = Member {
                room,
                name,
                muted: false,
                deafened: false,
                session,
                tx: tx.clone(),
                sfu_closed: Some(closed_rx),
            };
            let peers = members
                .iter()
                .filter(|(_, m)| m.room == room)
                .map(|(id, m)| m.peer(id))
                .collect();
            // Queued under the lock, so every later presence message for this room follows it.
            tx.send(ServerMsg::Joined {
                user_id: user_id.clone(),
                answer: ok.answer_sdp,
                peers,
            });
            broadcast(
                &members,
                room,
                ServerMsg::PeerJoined {
                    peer: member.peer(&user_id),
                },
            );
            members.insert(user_id.clone(), member);
        }

        tokio::spawn(self.clone().forward_events(
            user_id.clone(),
            session,
            ok.events,
            tx,
            closed_tx,
        ));
        Ok(JoinResult { user_id, session })
    }

    /// Pump one session's SFU events to its socket until the SFU closes the session.
    async fn forward_events(
        self,
        user_id: String,
        session: u64,
        mut events: mpsc::UnboundedReceiver<SfuEvent>,
        tx: Outbox,
        closed_tx: oneshot::Sender<()>,
    ) {
        while let Some(SfuEvent::Offer(sdp)) = events.recv().await {
            // A replaced or left session's offers must not reach a socket that rejoined.
            if self.is_current(&user_id, session) {
                tx.send(ServerMsg::Offer { sdp });
            }
        }
        // SfuEvent::Closed (or the SFU went away). Close the socket if this session was
        // still current, i.e. the SFU ended it rather than a leave or rejoin.
        let _ = closed_tx.send(());
        if self.leave(&user_id, session) {
            tx.close(CLOSE_VOICE_ENDED);
        }
    }

    /// Leave if `session` is still `user_id`'s current one; true if it was.
    pub fn leave(&self, user_id: &str, session: u64) -> bool {
        self.remove(user_id, Some(session)).is_some()
    }

    fn is_current(&self, user_id: &str, session: u64) -> bool {
        let members = self.0.members.lock().expect("members lock poisoned");
        members.get(user_id).is_some_and(|m| m.session == session)
    }

    /// Set the session's mute and deafen state and tell the room (ignored if the session was
    /// replaced).
    pub fn set_state(&self, user_id: &str, session: u64, muted: bool, deafened: bool) {
        let mut members = self.0.members.lock().expect("members lock poisoned");
        let Some(m) = members.get_mut(user_id) else {
            return;
        };
        if m.session != session || (m.muted, m.deafened) == (muted, deafened) {
            return;
        }
        (m.muted, m.deafened) = (muted, deafened);
        let room = m.room;
        broadcast(
            &members,
            room,
            ServerMsg::PeerState {
                user_id: user_id.into(),
                muted,
                deafened,
                force_muted: false,
            },
        );
    }

    /// Everyone in voice in a room for which `visible(room_id)` is true.
    pub fn peers(&self, visible: impl Fn(i64) -> bool) -> Vec<Peer> {
        let members = self.0.members.lock().expect("members lock poisoned");
        members
            .iter()
            .filter(|(_, m)| visible(m.room))
            .map(|(id, m)| m.peer(id))
            .collect()
    }

    /// Pass an SDP answer to the SFU, unless `session` has been replaced: the SFU cannot tell
    /// which offer an answer belongs to.
    pub fn answer(&self, user_id: &str, session: u64, sdp: String) {
        if self.is_current(user_id, session) {
            self.0.sfu.answer(user_id, sdp);
        }
    }

    /// Remove `user_id` (only if `session` is current, when given), tell the room and end the
    /// SFU session. The SFU leave is sent under the members lock: a rejoin's `remove` takes the
    /// same lock before its SFU join, so this Leave can never land after (and kill) that Join.
    fn remove(&self, user_id: &str, session: Option<u64>) -> Option<Member> {
        let mut members = self.0.members.lock().expect("members lock poisoned");
        if session.is_some_and(|s| members.get(user_id).is_none_or(|m| m.session != s)) {
            return None;
        }
        let m = members.remove(user_id)?;
        self.0.sfu.leave(user_id);
        broadcast(
            &members,
            m.room,
            ServerMsg::PeerLeft {
                user_id: user_id.into(),
            },
        );
        Some(m)
    }

    fn room_size(&self, room: i64) -> usize {
        let members = self.0.members.lock().expect("members lock poisoned");
        members.values().filter(|m| m.room == room).count()
    }
}

fn broadcast(members: &HashMap<String, Member>, room: i64, msg: ServerMsg) {
    for m in members.values().filter(|m| m.room == room) {
        m.tx.send(msg.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vesper_sfu::{Sfu, SfuConfig};

    #[tokio::test]
    async fn stale_session_offers_are_not_forwarded() {
        let sfu = Sfu::spawn(SfuConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        })
        .unwrap();
        let rooms = Rooms::new(sfu, 12);
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let (tx, mut rx) = crate::hub::outbox();
        let (closed_tx, closed_rx) = oneshot::channel();
        // Session 7 of "u" is not current (nobody is in any room).
        let pump = tokio::spawn(rooms.forward_events("u".into(), 7, ev_rx, tx, closed_tx));
        ev_tx.send(SfuEvent::Offer("v=0".into())).unwrap();
        ev_tx.send(SfuEvent::Closed).unwrap();
        pump.await.unwrap();
        closed_rx.await.expect("closed signal fires");
        assert!(rx.queue.try_recv().is_err(), "nothing forwarded");
        assert_eq!(*rx.close.borrow(), None, "no close");
    }
}
