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
use vesper_protocol::{ErrorCode, Peer, RoomInfo, RoomKind, ServerMsg, ShareInfo, UserInfo};
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
    /// The room was empty before this join ("started voice").
    pub first: bool,
}

#[derive(Clone)]
pub struct Voice(Arc<Inner>);

struct Inner {
    sfu: SfuHandle,
    hub: Hub,
    max_room_size: usize,
    /// How long a DM ring lasts if nobody answers.
    ring_timeout: Duration,
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
    /// The DM ring that is up, by room: one per call.
    rings: HashMap<i64, Ring>,
    next_ring: u64,
}

struct Ring {
    id: u64,
    caller: String,
    callee: String,
}

struct Member {
    room: i64,
    name: String,
    muted: bool,
    deafened: bool,
    force_muted: bool,
    share: Option<ShareInfo>,
    /// Sharers whose screen this member receives.
    watching: HashSet<String>,
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
            share: self.share,
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

impl State {
    /// `user_id`'s member, only if `session` is still its current one.
    fn current_mut(&mut self, user_id: &str, session: u64) -> Option<&mut Member> {
        self.members
            .get_mut(user_id)
            .filter(|m| m.session == session)
    }

    /// Tell `sharer` how many people in their call watch the share. Nothing if none runs.
    fn send_watchers(&self, sharer: &str) {
        let Some(s) = self.members.get(sharer).filter(|s| s.share.is_some()) else {
            return;
        };
        let count = self
            .members
            .values()
            .filter(|m| m.room == s.room && m.watching.contains(sharer))
            .count();
        s.tx.send(ServerMsg::Watchers {
            count: count as u32,
        });
    }
}

/// Most the shares of one room may add up to, counting each one once per viewer who could watch
/// it (kbps). The highest total the video load test passed (4 people, two cores); higher totals were not tested.
const MAX_ROOM_SHARE_KBPS: u64 = 120_000;

/// The frame rates and bitrates a share may ask for.
fn valid_share(info: &ShareInfo) -> bool {
    matches!(info.fps, 30 | 60) && (500..=30_000).contains(&info.bitrate_kbps)
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
    pub fn new(sfu: SfuHandle, hub: Hub, max_room_size: usize, ring_timeout: Duration) -> Voice {
        Voice(Arc::new(Inner {
            sfu,
            hub,
            max_room_size,
            ring_timeout,
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
        let first;
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
                share: None,
                watching: HashSet::new(),
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
            first = !st.members.values().any(|m| m.room == room.id);
            st.members.insert(user_id.clone(), member);
            if st.rings.get(&room.id).is_some_and(|r| r.callee == *user_id) {
                self.stop_ring(&mut st, room.id);
            }
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
            first,
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
            first: false,
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
        while let Some(ev) = events.recv().await {
            match ev {
                // A replaced or left session's offers must not reach a socket that rejoined.
                SfuEvent::Offer(sdp) if self.is_current(&user_id, session) => {
                    tx.send(ServerMsg::Offer { sdp });
                }
                SfuEvent::Offer(_) => {}
                SfuEvent::ShareCut => self.cut_share(&user_id, session),
                SfuEvent::Closed => break,
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
        let Some(m) = st.current_mut(user_id, session) else {
            return;
        };
        let muted = muted || m.force_muted;
        if (m.muted, m.deafened) == (muted, deafened) {
            return;
        }
        (m.muted, m.deafened) = (muted, deafened);
        let (room, msg, tx) = (m.room, m.state_msg(user_id), m.tx.clone());
        st.tell(&self.0.hub, room, msg, &[&tx]);
    }

    /// Start `user_id`'s screen share, or change its settings. Group rooms only. Ignored if
    /// `session` was replaced.
    pub fn share_start(
        &self,
        user_id: &str,
        session: u64,
        info: ShareInfo,
    ) -> Result<(), ErrorCode> {
        let mut st = self.lock();
        let Some(m) = st.current_mut(user_id, session) else {
            return Ok(());
        };
        let room = m.room;
        if st.rooms.get(&room).is_none_or(|r| r.kind == RoomKind::Dm) {
            return Err(ErrorCode::Forbidden);
        }
        if !valid_share(&info) {
            return Err(ErrorCode::BadMessage);
        }
        let viewers = st.members.values().filter(|o| o.room == room).count() as u64 - 1;
        let others: u64 = st
            .members
            .iter()
            .filter(|(id, o)| o.room == room && id.as_str() != user_id)
            .filter_map(|(_, o)| o.share)
            .map(|sh| u64::from(sh.bitrate_kbps) * viewers)
            .sum();
        if others + u64::from(info.bitrate_kbps) * viewers > MAX_ROOM_SHARE_KBPS {
            return Err(ErrorCode::ShareCap);
        }
        let m = st.members.get_mut(user_id).expect("checked above");
        let began = m.share.replace(info).is_none();
        if began {
            self.0.sfu.set_sharing(user_id, true);
        }
        let tx = m.tx.clone();
        let msg = ServerMsg::PeerShare {
            user_id: user_id.into(),
            share: Some(info),
        };
        st.tell(&self.0.hub, room, msg, &[&tx]);
        if began {
            st.send_watchers(user_id);
        }
        Ok(())
    }

    /// End `user_id`'s screen share, if one runs. Ignored if `session` was replaced.
    pub fn share_stop(&self, user_id: &str, session: u64) {
        self.end_share(user_id, session, None);
    }

    /// The SFU cut `user_id`'s share for going over the rate ceiling: end it for everyone and
    /// tell the sharer why. Ignored if `session` was replaced.
    pub fn cut_share(&self, user_id: &str, session: u64) {
        let why = ServerMsg::Error {
            code: ErrorCode::RateLimited,
            message: "Your screen share sent more than 35 Mbps and was stopped.".into(),
        };
        self.end_share(user_id, session, Some(why));
    }

    /// A moderator (`by`) ends `user_id`'s share as if they had stopped it, and the sharer is
    /// told with `ShareStopped`. Only in a group room `by` can see. False if no such share runs.
    /// Nothing holds afterwards: they may share again.
    pub fn moderator_stop_share(&self, user_id: &str, by: &UserInfo) -> bool {
        let session = {
            let st = self.lock();
            let Some(m) = st.members.get(user_id).filter(|m| m.share.is_some()) else {
                return false;
            };
            match st.rooms.get(&m.room) {
                Some(r) if r.kind != RoomKind::Dm && can_see(by, r) => m.session,
                _ => return false,
            }
        };
        self.end_share(user_id, session, Some(ServerMsg::ShareStopped))
    }

    /// `why` goes to the sharer after the share's end, when they did not ask for the stop.
    /// False if no share ran for this session.
    fn end_share(&self, user_id: &str, session: u64, why: Option<ServerMsg>) -> bool {
        let mut st = self.lock();
        let Some(m) = st.current_mut(user_id, session) else {
            return false;
        };
        if m.share.take().is_none() {
            return false;
        }
        self.0.sfu.set_sharing(user_id, false);
        let (room, tx) = (m.room, m.tx.clone());
        let msg = ServerMsg::PeerShare {
            user_id: user_id.into(),
            share: None,
        };
        st.tell(&self.0.hub, room, msg.clone(), &[&tx]);
        if let Some(why) = why {
            tx.send(msg);
            tx.send(why);
        }
        true
    }

    /// `user_id` starts or stops receiving `sharer`'s screen. Ignored if `session` was replaced
    /// or `user_id` is `sharer`. Starting also needs `sharer` in the same call; stopping always
    /// applies, so a viewer's watch is theirs to end even after the sharer left.
    pub fn watch(&self, user_id: &str, session: u64, sharer: &str, on: bool) {
        let mut st = self.lock();
        let Some(m) = st.members.get(user_id) else {
            return;
        };
        if m.session != session
            || user_id == sharer
            || (on && st.members.get(sharer).is_none_or(|s| s.room != m.room))
        {
            return;
        }
        let m = st.members.get_mut(user_id).expect("checked above");
        let changed = if on {
            m.watching.insert(sharer.into())
        } else {
            m.watching.remove(sharer)
        };
        self.0.sfu.watch(user_id, sharer, on);
        if changed {
            st.send_watchers(sharer);
        }
    }

    /// `user_id` points at a spot on `sharer`'s screen: everyone else in the call sees it.
    pub fn point(
        &self,
        user_id: &str,
        session: u64,
        sharer: &str,
        x: f32,
        y: f32,
    ) -> Result<(), ErrorCode> {
        let unit = 0.0..=1.0;
        if !unit.contains(&x) || !unit.contains(&y) {
            return Err(ErrorCode::BadMessage);
        }
        let st = self.lock();
        let Some(m) = st.members.get(user_id) else {
            return Ok(());
        };
        if m.session != session
            || !st
                .members
                .get(sharer)
                .is_some_and(|s| s.room == m.room && s.share.is_some())
        {
            return Ok(());
        }
        let msg = ServerMsg::Point {
            user_id: sharer.into(),
            from: user_id.into(),
            x,
            y,
        };
        for (id, o) in &st.members {
            if o.room == m.room && id != user_id {
                o.tx.send(msg.clone());
            }
        }
        Ok(())
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
        if st.rings.get(&m.room).is_some_and(|r| r.caller == user_id) {
            self.stop_ring(&mut st, m.room);
        }
        let left = ServerMsg::PeerLeft {
            user_id: user_id.into(),
        };
        let skip: Vec<&Outbox> = [Some(&m.tx), joining].into_iter().flatten().collect();
        st.tell(&self.0.hub, m.room, left.clone(), &skip);
        m.tx.send(left);
        for sharer in &m.watching {
            st.send_watchers(sharer);
        }
        Some(m)
    }

    /// Ring the other member of a DM after `caller` joined its voice. Nothing for a room, when the
    /// other member is already in the call, or when a ring is already up. It ends when the caller
    /// leaves, the other member joins or declines, or after the timeout.
    pub fn ring(&self, room: &RoomInfo, caller: &str) {
        if room.kind != RoomKind::Dm {
            return;
        }
        let Some(callee) = room.members.iter().find(|m| *m != caller) else {
            return;
        };
        let id = {
            let mut st = self.lock();
            let in_call = |u: &str| st.members.get(u).is_some_and(|m| m.room == room.id);
            if !in_call(caller) || in_call(callee) || st.rings.contains_key(&room.id) {
                return;
            }
            st.next_ring += 1;
            let id = st.next_ring;
            let ring = Ring {
                id,
                caller: caller.into(),
                callee: callee.clone(),
            };
            st.rings.insert(room.id, ring);
            let msg = ServerMsg::RingStart {
                room_id: room.id,
                user_id: caller.into(),
            };
            self.0.hub.send_user(callee, msg);
            id
        };
        let (voice, room_id) = (self.clone(), room.id);
        tokio::spawn(async move {
            tokio::time::sleep(voice.0.ring_timeout).await;
            let mut st = voice.lock();
            if st.rings.get(&room_id).is_some_and(|r| r.id == id) {
                voice.stop_ring(&mut st, room_id);
            }
        });
    }

    /// `user_id` declines the ring in `room`: it stops for them, the caller stays in the call.
    pub fn decline_ring(&self, user_id: &str, room: i64) {
        let mut st = self.lock();
        if st.rings.get(&room).is_some_and(|r| r.callee == user_id) {
            self.stop_ring(&mut st, room);
        }
    }

    fn stop_ring(&self, st: &mut State, room: i64) {
        if let Some(r) = st.rings.remove(&room) {
            let msg = ServerMsg::RingStop { room_id: room };
            self.0.hub.send_user(&r.callee, msg);
        }
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
        let voice = Voice::new(sfu, Hub::default(), 12, Duration::from_secs(30));
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

    #[tokio::test]
    async fn cut_share_tells_everyone() {
        let sfu = Sfu::spawn(SfuConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            ..Default::default()
        })
        .unwrap();
        let hub = Hub::default();
        let voice = Voice::new(sfu, hub.clone(), 12, Duration::from_secs(30));
        let (a_tx, mut a_rx) = crate::hub::outbox();
        let (b_tx, mut b_rx) = crate::hub::outbox();
        let bob = UserInfo {
            user_id: "b".repeat(32),
            username: "bob".into(),
            display_name: "bob".into(),
            accent: String::new(),
            role: vesper_protocol::Role::Member,
            banned: false,
            bio: String::new(),
            avatar: None,
            ui_accent: None,
        };
        hub.add(bob, 1, b_tx.clone());
        let member = |tx, share| Member {
            room: 1,
            name: "n".into(),
            muted: false,
            deafened: false,
            force_muted: false,
            share,
            watching: HashSet::new(),
            session: 5,
            tx,
            sfu_closed: None,
        };
        let info = ShareInfo {
            fps: 30,
            bitrate_kbps: 2000,
        };
        {
            let mut st = voice.lock();
            st.rooms.insert(
                1,
                RoomInfo {
                    id: 1,
                    ..RoomInfo::default()
                },
            );
            st.members.insert("alice".into(), member(a_tx, Some(info)));
            st.members.insert("bob".into(), member(b_tx, None));
        }
        voice.cut_share("alice", 5);
        let gone = ServerMsg::PeerShare {
            user_id: "alice".into(),
            share: None,
        };
        assert_eq!(b_rx.queue.try_recv().unwrap(), gone);
        assert_eq!(a_rx.queue.try_recv().unwrap(), gone);
        assert!(matches!(
            a_rx.queue.try_recv().unwrap(),
            ServerMsg::Error {
                code: ErrorCode::RateLimited,
                ..
            }
        ));
    }

    #[test]
    fn share_settings_are_checked() {
        let ok = |fps, bitrate_kbps| valid_share(&ShareInfo { fps, bitrate_kbps });
        assert!(ok(30, 500) && ok(60, 30000));
        assert!(!ok(45, 8000) && !ok(60, 499) && !ok(60, 30001) && !ok(0, 8000));
    }
}
