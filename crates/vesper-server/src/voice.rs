//! Voice presence: who is in which room, muted, deafened or force-muted. Media lives in the SFU;
//! this module keeps the two in step and tells every socket that can see the room (see
//! `Hub::broadcast_room`), so the room list shows who is talking even outside the call.
//!
//! A user is their account id, so a second join by the same account (another tab) replaces the
//! first voice session; the first tab's socket stays open and gets `PeerLeft` for itself. Every
//! join gets a fresh session number; messages from a socket whose session is no longer current
//! are ignored, because the SFU keys sessions by user alone and would otherwise apply them to
//! the new session.
//!
//! Who gets what: `PeerJoined` and `PeerState` go to every socket that can see the room except
//! the session's own socket (it has `Joined` or sent the change). `PeerLeft` also goes to the
//! session's own socket, which is how a tab learns its call ended (kick, other tab, SFU).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tracing::warn;
use vesper_protocol::{ErrorCode, Peer, RoomInfo, ServerMsg, UserInfo};
use vesper_sfu::{JoinError, SfuEvent, SfuHandle};
use vesper_store::random_hex;

use crate::api_rooms::can_see;
use crate::hub::{Hub, Outbox};

/// How long a rejoin waits for the SFU to confirm the old session closed.
const REPLACE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinResult {
    pub user_id: String,
    pub session: u64,
}

#[derive(Clone)]
pub struct Voice(Arc<Inner>);

struct Inner {
    sfu: SfuHandle,
    hub: Hub,
    max_room_size: usize,
    next_session: AtomicU64,
    /// Serialises joins, so a rejoin finishes closing the old session before anyone else joins
    /// and the room-size check cannot race another join.
    join_lock: tokio::sync::Mutex<()>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    members: HashMap<String, Member>,
    /// The latest known version of each room, for `broadcast_room`. Set by joins and `refresh`.
    rooms: HashMap<i64, RoomInfo>,
    /// Force-muted account ids. Kept across leave and rejoin until lifted (not across restarts).
    force_muted: HashSet<String>,
    /// SFU ids of the connection checks that run.
    checks: HashSet<String>,
}

struct Member {
    room: i64,
    name: String,
    muted: bool,
    deafened: bool,
    force_muted: bool,
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
            force_muted: self.force_muted,
        }
    }

    fn state_msg(&self, user_id: &str) -> ServerMsg {
        ServerMsg::PeerState {
            user_id: user_id.into(),
            muted: self.muted,
            deafened: self.deafened,
            force_muted: self.force_muted,
        }
    }
}

impl State {
    /// `msg` to every socket that can see `room`, except `skip`.
    fn tell(&self, hub: &Hub, room: i64, msg: ServerMsg, skip: &[&Outbox]) {
        if let Some(room) = self.rooms.get(&room) {
            hub.broadcast_room_except(room, msg, skip);
        }
    }
}

fn sfu_error(who: &str, e: JoinError) -> ErrorCode {
    match e {
        // Not the parser's text: it can echo SDP (addresses) from an untrusted client.
        JoinError::BadOffer(_) => warn!("join by {who}: bad sdp offer"),
        // The SFU thread logs its own death once at error level.
        JoinError::Shutdown => warn!("join by {who}: the SFU has shut down"),
    }
    ErrorCode::BadMessage
}

impl Voice {
    pub fn new(sfu: SfuHandle, hub: Hub, max_room_size: usize) -> Voice {
        Voice(Arc::new(Inner {
            sfu,
            hub,
            max_room_size,
            next_session: AtomicU64::new(1),
            join_lock: tokio::sync::Mutex::new(()),
            state: Mutex::default(),
        }))
    }

    pub fn sfu_alive(&self) -> bool {
        self.0.sfu.is_alive()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().expect("voice lock poisoned")
    }

    /// Check access, replace any earlier session of the same user, start the SFU session and
    /// enter `room`. `user` and `room` must be fresh from the store. On success `Joined` has
    /// been queued on `tx`. The room cap, a dead SFU and a malformed offer are refused before
    /// the user's earlier session ends; an SFU refusal after that ends it.
    pub async fn join(
        &self,
        user: &UserInfo,
        room: &RoomInfo,
        offer: String,
        tx: Outbox,
    ) -> Result<JoinResult, ErrorCode> {
        if user.banned || !can_see(user, room) {
            return Err(ErrorCode::Forbidden);
        }
        let user_id = &user.user_id;
        let _serial = self.0.join_lock.lock().await;

        // Under the join lock the room can only shrink until the insert below.
        if self.room_size_without(room.id, user_id) >= self.0.max_room_size {
            return Err(ErrorCode::RoomFull);
        }
        if !self.sfu_alive() {
            return Err(sfu_error(user_id, JoinError::Shutdown));
        }
        vesper_sfu::check_offer(&offer).map_err(|e| sfu_error(user_id, e))?;

        if let Some(mut old) = self.remove(user_id, None, Some(&tx))
            && let Some(closed) = old.sfu_closed.take()
            && tokio::time::timeout(REPLACE_TIMEOUT, closed).await.is_err()
        {
            warn!("old session of {user_id} did not close in {REPLACE_TIMEOUT:?}");
        }

        let ok = self
            .0
            .sfu
            .join(format!("room-{}", room.id), user_id.clone(), offer)
            .await
            .map_err(|e| sfu_error(user_id, e))?;

        let session = self.0.next_session.fetch_add(1, Ordering::Relaxed);
        let (closed_tx, closed_rx) = oneshot::channel();
        {
            let mut st = self.lock();
            // A change since `room` was read is already here (`refresh`): never put the old copy
            // back, and check access against the new one.
            let cached = st.rooms.entry(room.id).or_insert_with(|| room.clone());
            if !can_see(user, cached) {
                self.0.sfu.leave(user_id);
                return Err(ErrorCode::Forbidden);
            }
            let force_muted = st.force_muted.contains(user_id);
            let member = Member {
                room: room.id,
                name: user.display_name.clone(),
                muted: force_muted,
                deafened: false,
                force_muted,
                session,
                tx: tx.clone(),
                sfu_closed: Some(closed_rx),
            };
            let peers = st
                .members
                .iter()
                .filter(|(_, m)| m.room == room.id)
                .map(|(id, m)| m.peer(id))
                .collect();
            // Queued under the lock, so every later presence message for this room follows it.
            tx.send(ServerMsg::Joined {
                user_id: user_id.clone(),
                answer: ok.answer_sdp,
                peers,
            });
            if force_muted {
                tx.send(ServerMsg::ForceMuted { muted: true });
            }
            let joined = ServerMsg::PeerJoined {
                peer: member.peer(user_id),
            };
            st.tell(&self.0.hub, room.id, joined, &[&tx]);
            st.members.insert(user_id.clone(), member);
        }

        tokio::spawn(self.clone().forward_events(
            user_id.clone(),
            session,
            ok.events,
            tx,
            closed_tx,
        ));
        Ok(JoinResult {
            user_id: user_id.clone(),
            session,
        })
    }

    /// The connection check: an SFU session under a random id in a room of its own, so it
    /// neither shows in presence nor ends the user's own call (the SFU keys sessions by user).
    /// `Joined` (with no peers) is queued on `tx`; end it with [`Voice::end_check`].
    pub async fn check(&self, offer: String, tx: Outbox) -> Result<JoinResult, ErrorCode> {
        let id = random_hex(16);
        let ok = self
            .0
            .sfu
            .join(format!("check-{id}"), id.clone(), offer)
            .await
            .map_err(|e| sfu_error("a check", e))?;
        self.lock().checks.insert(id.clone());
        // Events are dropped: alone in its room, the session gets no renegotiation offers.
        tx.send(ServerMsg::Joined {
            user_id: id.clone(),
            answer: ok.answer_sdp,
            peers: vec![],
        });
        Ok(JoinResult {
            user_id: id,
            session: 0,
        })
    }

    pub fn end_check(&self, id: &str) {
        self.lock().checks.remove(id);
        self.0.sfu.leave(id);
    }

    /// How many connection checks run.
    pub fn checks(&self) -> usize {
        self.lock().checks.len()
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
        // SfuEvent::Closed (or the SFU went away). If the session was still current the SFU
        // ended it: leave, which tells the socket with `PeerLeft` (the socket stays open).
        let _ = closed_tx.send(());
        self.leave(&user_id, session);
    }

    /// Leave if `session` is still `user_id`'s current one; true if it was.
    pub fn leave(&self, user_id: &str, session: u64) -> bool {
        self.remove(user_id, Some(session), None).is_some()
    }

    pub fn is_current(&self, user_id: &str, session: u64) -> bool {
        self.lock()
            .members
            .get(user_id)
            .is_some_and(|m| m.session == session)
    }

    /// Set the session's mute and deafen state and tell the room (ignored if the session was
    /// replaced). A force-muted user stays muted.
    pub fn set_state(&self, user_id: &str, session: u64, muted: bool, deafened: bool) {
        let mut st = self.lock();
        let Some(m) = st.members.get_mut(user_id) else {
            return;
        };
        let muted = muted || m.force_muted;
        if m.session != session || (m.muted, m.deafened) == (muted, deafened) {
            return;
        }
        (m.muted, m.deafened) = (muted, deafened);
        let (room, msg, tx) = (m.room, m.state_msg(user_id), m.tx.clone());
        st.tell(&self.0.hub, room, msg, &[&tx]);
    }

    /// Everyone in voice in a room for which `visible(room_id)` is true.
    pub fn peers(&self, visible: impl Fn(i64) -> bool) -> Vec<Peer> {
        self.lock()
            .members
            .iter()
            .filter(|(_, m)| visible(m.room))
            .map(|(id, m)| m.peer(id))
            .collect()
    }

    /// Take `user_id` out of voice (moderator kick). False if they were not in voice.
    pub fn kick(&self, user_id: &str) -> bool {
        self.remove(user_id, None, None).is_some()
    }

    /// Set or lift a force-mute and tell the user's sockets with `ForceMuted`. It holds across
    /// leave and rejoin until lifted. Lifting does not unmute: the user does that. The SFU drops
    /// their audio while it holds, so a client that ignores it is not heard. True if the user is
    /// in voice now.
    pub fn force_mute(&self, user_id: &str, muted: bool) -> bool {
        let mut st = self.lock();
        if muted {
            st.force_muted.insert(user_id.into());
        } else {
            st.force_muted.remove(user_id);
        }
        // Under the lock, so two force-mutes reach the SFU in the order they change `force_muted`.
        self.0.sfu.set_muted(user_id, muted);
        self.0
            .hub
            .send_user(user_id, ServerMsg::ForceMuted { muted });
        let Some(m) = st.members.get_mut(user_id) else {
            return false;
        };
        m.force_muted = muted;
        m.muted |= muted;
        let (room, msg, tx) = (m.room, m.state_msg(user_id), m.tx.clone());
        st.tell(&self.0.hub, room, msg, &[&tx]);
        true
    }

    /// The room list or a role changed: remember the new rooms and take out of voice everyone
    /// who can no longer see their room (or whose room or account is gone, or who is banned).
    pub fn refresh(&self, rooms: &[RoomInfo], users: &[UserInfo]) {
        let lost: Vec<(String, u64)> = {
            let mut st = self.lock();
            st.rooms = rooms.iter().map(|r| (r.id, r.clone())).collect();
            st.members
                .iter()
                .filter(|(id, m)| {
                    let user = users.iter().find(|u| &u.user_id == *id);
                    let room = st.rooms.get(&m.room);
                    !matches!((user, room), (Some(u), Some(r)) if !u.banned && can_see(u, r))
                })
                .map(|(id, m)| (id.clone(), m.session))
                .collect()
        };
        for (id, session) in lost {
            self.leave(&id, session);
        }
    }

    /// Pass an SDP answer to the SFU, unless `session` has been replaced: the SFU cannot tell
    /// which offer an answer belongs to.
    pub fn answer(&self, user_id: &str, session: u64, sdp: String) {
        if self.is_current(user_id, session) {
            self.0.sfu.answer(user_id, sdp);
        }
    }

    /// Remove `user_id` (only if `session` is current, when given), tell the room and the
    /// session's own socket, and end the SFU session. The SFU leave is sent under the lock: a
    /// rejoin's `remove` takes the same lock before its SFU join, so this Leave can never land
    /// after (and kill) that Join. `joining` is the socket of a rejoin, which gets `Joined`
    /// instead.
    fn remove(
        &self,
        user_id: &str,
        session: Option<u64>,
        joining: Option<&Outbox>,
    ) -> Option<Member> {
        let mut st = self.lock();
        if session.is_some_and(|s| st.members.get(user_id).is_none_or(|m| m.session != s)) {
            return None;
        }
        let m = st.members.remove(user_id)?;
        self.0.sfu.leave(user_id);
        let left = ServerMsg::PeerLeft {
            user_id: user_id.into(),
        };
        let skip: Vec<&Outbox> = [Some(&m.tx), joining].into_iter().flatten().collect();
        st.tell(&self.0.hub, m.room, left.clone(), &skip);
        m.tx.send(left);
        Some(m)
    }

    /// People in voice in `room`, not counting `user_id`, whose join would replace their own.
    fn room_size_without(&self, room: i64, user_id: &str) -> usize {
        self.lock()
            .members
            .iter()
            .filter(|(id, m)| m.room == room && *id != user_id)
            .count()
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
        let voice = Voice::new(sfu, Hub::default(), 12);
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let (tx, mut rx) = crate::hub::outbox();
        let (closed_tx, closed_rx) = oneshot::channel();
        // Session 7 of "u" is not current (nobody is in any room).
        let pump = tokio::spawn(voice.forward_events("u".into(), 7, ev_rx, tx, closed_tx));
        ev_tx.send(SfuEvent::Offer("v=0".into())).unwrap();
        ev_tx.send(SfuEvent::Closed).unwrap();
        pump.await.unwrap();
        closed_rx.await.expect("closed signal fires");
        assert!(rx.queue.try_recv().is_err(), "nothing forwarded");
        assert_eq!(*rx.close.borrow(), None, "no close");
    }
}
