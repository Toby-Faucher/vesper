//! Every open WebSocket, with its user, so the REST routes can push events and close sockets.
//!
//! Each socket has a bounded queue. Sends never wait: a send to a full queue closes that socket
//! instead (the client reconnects and gets a fresh `Hello`). The close signal does not go
//! through the queue, so it also reaches a socket whose writer is stuck on a client that stopped
//! reading.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, watch};
use vesper_protocol::{RoomInfo, ServerMsg, UserInfo};

use crate::api_rooms::can_see;

/// Messages one socket may have waiting to be written.
pub const QUEUE: usize = 256;
/// Close code for a socket whose queue filled up (1013, "try again later").
pub const CLOSE_TOO_SLOW: u16 = 1013;

/// The sending side of one socket. Cheap to clone; every clone feeds the same socket.
#[derive(Clone, Debug)]
pub struct Outbox {
    queue: mpsc::Sender<ServerMsg>,
    close: Arc<watch::Sender<Option<u16>>>,
}

/// The socket task's side of an [`Outbox`].
pub struct Inbox {
    pub queue: mpsc::Receiver<ServerMsg>,
    /// Becomes `Some(code)` once the socket must close.
    pub close: watch::Receiver<Option<u16>>,
}

pub fn outbox() -> (Outbox, Inbox) {
    let (queue, queue_rx) = mpsc::channel(QUEUE);
    let (close, close_rx) = watch::channel(None);
    (
        Outbox {
            queue,
            close: Arc::new(close),
        },
        Inbox {
            queue: queue_rx,
            close: close_rx,
        },
    )
}

impl Outbox {
    /// Queue `msg`, or close the socket if its queue is full.
    pub fn send(&self, msg: ServerMsg) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.queue.try_send(msg) {
            self.close(CLOSE_TOO_SLOW);
        }
    }

    /// Close the socket with `code`. The first code wins.
    pub fn close(&self, code: u16) {
        self.close.send_if_modified(|c| {
            let first = c.is_none();
            if first {
                *c = Some(code);
            }
            first
        });
    }
}

pub type ConnId = u64;

struct Conn {
    user: UserInfo,
    session_id: i64,
    tx: Outbox,
}

/// The open sockets, plus a gate that a test can hold to pause sockets before [`Hub::add`].
#[derive(Clone, Default)]
pub struct Hub(Arc<Mutex<Inner>>, Arc<tokio::sync::RwLock<()>>);

#[derive(Default)]
struct Inner {
    next: ConnId,
    conns: HashMap<ConnId, Conn>,
}

impl Hub {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().expect("hub lock poisoned")
    }

    /// Test seam: while the guard lives, upgraded sockets wait in [`Hub::wait_gate`], which is
    /// the gap between the upgrade's session check and `add`.
    #[doc(hidden)]
    pub async fn hold_adds(&self) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.1.clone().write_owned().await
    }

    /// Returns at once unless a test holds [`Hub::hold_adds`].
    pub async fn wait_gate(&self) {
        drop(self.1.read().await);
    }

    pub fn add(&self, user: UserInfo, session_id: i64, tx: Outbox) -> ConnId {
        let mut h = self.lock();
        h.next += 1;
        let id = h.next;
        h.conns.insert(
            id,
            Conn {
                user,
                session_id,
                tx,
            },
        );
        id
    }

    pub fn remove(&self, id: ConnId) {
        self.lock().conns.remove(&id);
    }

    pub fn send_user(&self, user_id: &str, msg: ServerMsg) {
        for c in self.lock().conns.values() {
            if c.user.user_id == user_id {
                c.tx.send(msg.clone());
            }
        }
    }

    pub fn broadcast(&self, msg: ServerMsg) {
        for c in self.lock().conns.values() {
            c.tx.send(msg.clone());
        }
    }

    /// Only to sockets whose user can see `room`.
    pub fn broadcast_room(&self, room: &RoomInfo, msg: ServerMsg) {
        self.broadcast_room_except(room, msg, &[]);
    }

    /// [`Hub::broadcast_room`], but not to the sockets `skip` feeds.
    pub fn broadcast_room_except(&self, room: &RoomInfo, msg: ServerMsg, skip: &[&Outbox]) {
        for c in self.lock().conns.values() {
            if can_see(&c.user, room) && !skip.iter().any(|s| s.queue.same_channel(&c.tx.queue)) {
                c.tx.send(msg.clone());
            }
        }
    }

    /// `RoomsChanged` to every socket, each with only the rooms its user can see.
    pub fn rooms_changed(&self, rooms: &[RoomInfo]) {
        for c in self.lock().conns.values() {
            let rooms = rooms.iter().filter(|r| can_see(&c.user, r)).cloned();
            c.tx.send(ServerMsg::RoomsChanged {
                rooms: rooms.collect(),
            });
        }
    }

    pub fn close_session(&self, session_id: i64, code: u16) {
        for c in self.lock().conns.values() {
            if c.session_id == session_id {
                c.tx.close(code);
            }
        }
    }

    pub fn close_user(&self, user_id: &str, code: u16) {
        self.close_user_except(user_id, None, code);
    }

    /// Like [`Hub::close_user`], but sockets of session `keep` stay open (password change).
    pub fn close_user_except(&self, user_id: &str, keep: Option<i64>, code: u16) {
        for c in self.lock().conns.values() {
            if c.user.user_id == user_id && Some(c.session_id) != keep {
                c.tx.close(code);
            }
        }
    }

    /// A role, ban or profile change: later room filtering uses the new role, and everyone
    /// gets `UserUpdated`.
    pub fn update_user(&self, user: UserInfo) {
        let mut h = self.lock();
        for c in h.conns.values_mut() {
            if c.user.user_id == user.user_id {
                c.user = user.clone();
            }
            c.tx.send(ServerMsg::UserUpdated { user: user.clone() });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vesper_protocol::Role;

    fn user(id: char, role: Role) -> UserInfo {
        UserInfo {
            user_id: id.to_string().repeat(32),
            username: "u".into(),
            display_name: "u".into(),
            accent: String::new(),
            role,
            banned: false,
            bio: String::new(),
            avatar: None,
            ui_accent: None,
        }
    }

    #[test]
    fn full_queue_closes_only_that_socket() {
        let hub = Hub::default();
        let (slow, mut slow_rx) = outbox();
        let (fast, mut fast_rx) = outbox();
        hub.add(user('a', Role::Member), 1, slow);
        hub.add(user('b', Role::Member), 2, fast);
        for _ in 0..QUEUE {
            hub.broadcast(ServerMsg::ForceMuted { muted: true });
            assert!(fast_rx.queue.try_recv().is_ok());
        }
        assert_eq!(
            *slow_rx.close.borrow(),
            None,
            "full but nothing dropped yet"
        );
        hub.broadcast(ServerMsg::ForceMuted { muted: true });
        assert_eq!(*slow_rx.close.borrow_and_update(), Some(CLOSE_TOO_SLOW));
        assert_eq!(*fast_rx.close.borrow(), None);
        assert!(fast_rx.queue.try_recv().is_ok());
        // A later close keeps the first code.
        hub.close_session(1, vesper_protocol::CLOSE_SESSION_ENDED);
        assert_eq!(*slow_rx.close.borrow(), Some(CLOSE_TOO_SLOW));
    }

    #[test]
    fn room_messages_follow_can_see_and_role_updates() {
        let hub = Hub::default();
        let (tx, mut rx) = outbox();
        let member = user('a', Role::Member);
        hub.add(member.clone(), 1, tx);
        let room = RoomInfo {
            id: 1,
            name: "r".into(),
            position: 0,
            restricted: true,
            members: vec![],
        };
        hub.broadcast_room(&room, ServerMsg::ForceMuted { muted: true });
        assert!(rx.queue.try_recv().is_err());
        hub.update_user(UserInfo {
            role: Role::Moderator,
            ..member
        });
        assert!(matches!(
            rx.queue.try_recv(),
            Ok(ServerMsg::UserUpdated { .. })
        ));
        hub.broadcast_room(&room, ServerMsg::ForceMuted { muted: true });
        assert!(rx.queue.try_recv().is_ok());
    }
}
