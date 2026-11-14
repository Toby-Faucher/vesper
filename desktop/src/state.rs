//! What the server pushes, applied to plain data. A Rust copy of `web/src/lib/state.ts`, `peers.ts` and the pure
//! parts of `chat.svelte.ts`; keep the two in step.

use std::collections::HashMap;

use vesper_protocol::{
    Me, Message, MessagePage, Peer, RoomInfo, RoomKind, ServerMsg, Unread, UserInfo,
};

#[derive(Debug, Default, Clone)]
pub struct AppState {
    pub me: Option<Me>,
    pub users: HashMap<String, UserInfo>,
    /// Rooms that are not archived, by position.
    pub rooms: Vec<RoomInfo>,
    /// Archived rooms: the server sends them to the owner only. Never shown here.
    pub archived: Vec<RoomInfo>,
    /// Your DMs. Each one's `name` is the other person's display name, set here.
    pub dms: Vec<RoomInfo>,
    pub voice: Vec<Peer>,
    /// By room id. A room missing here has nothing unread.
    pub unread: HashMap<i64, Unread>,
    /// By room id: the newest message id seen, a stand-in for activity time. Orders DMs.
    pub activity: HashMap<i64, i64>,
}

impl AppState {
    pub fn my_id(&self) -> &str {
        self.me.as_ref().map_or("", |m| m.user.user_id.as_str())
    }

    pub fn name_of(&self, user_id: &str) -> String {
        self.users
            .get(user_id)
            .map_or_else(|| "Someone".into(), |u| u.display_name.clone())
    }

    /// A room or a DM by id.
    pub fn find_room(&self, id: i64) -> Option<&RoomInfo> {
        self.rooms.iter().chain(&self.dms).find(|r| r.id == id)
    }

    /// DMs with unread first, then newest message first, then by name.
    pub fn sorted_dms(&self) -> Vec<&RoomInfo> {
        let unread = |r: &RoomInfo| self.unread.get(&r.id).is_some_and(|u| u.unread > 0);
        let act = |r: &RoomInfo| self.activity.get(&r.id).copied().unwrap_or(0);
        let mut v: Vec<&RoomInfo> = self.dms.iter().collect();
        v.sort_by(|a, b| {
            unread(b)
                .cmp(&unread(a))
                .then(act(b).cmp(&act(a)))
                .then(a.name.cmp(&b.name))
        });
        v
    }

    /// Display names of the people in voice in a room.
    pub fn voice_in(&self, room_id: i64) -> Vec<String> {
        self.voice
            .iter()
            .filter(|p| p.room_id == room_id)
            .map(|p| p.name.clone())
            .collect()
    }

    fn set_rooms(&mut self, list: &[RoomInfo]) {
        let (dms, rooms): (Vec<_>, Vec<_>) =
            list.iter().cloned().partition(|r| r.kind == RoomKind::Dm);
        (self.archived, self.rooms) = rooms.into_iter().partition(|r| r.archived);
        self.dms = dms;
        self.title_dms();
    }

    /// DMs named after the other person (the stored name is generated).
    fn title_dms(&mut self) {
        let me = self.my_id().to_string();
        for i in 0..self.dms.len() {
            let r = &self.dms[i];
            let peer = r.members.iter().find(|m| **m != me).unwrap_or(&me).clone();
            self.dms[i].name = self.name_of(&peer);
        }
    }

    pub fn apply(&mut self, m: &ServerMsg) {
        match m {
            ServerMsg::Hello {
                me,
                users,
                rooms,
                voice,
                unread,
            } => {
                *self = AppState {
                    me: Some(me.clone()),
                    users: users
                        .iter()
                        .map(|u| (u.user_id.clone(), u.clone()))
                        .collect(),
                    voice: voice.clone(),
                    unread: unread.iter().map(|u| (u.room_id, u.clone())).collect(),
                    ..Default::default()
                };
                self.set_rooms(rooms);
            }
            ServerMsg::MessageNew { message: msg } => {
                let seen = self.activity.entry(msg.room_id).or_insert(0);
                *seen = (*seen).max(msg.id);
                let mine = msg.author_id == self.my_id();
                let mention = msg.mentions.iter().any(|id| id == self.my_id());
                let u = self.unread.entry(msg.room_id).or_insert_with(|| Unread {
                    room_id: msg.room_id,
                    last_read_id: 0,
                    unread: 0,
                    mentions: 0,
                });
                if mine || msg.id <= u.last_read_id {
                    return;
                }
                u.unread += 1;
                u.mentions += u32::from(mention);
            }
            // A marker older than the newest message clears the count too, as in the web app.
            ServerMsg::Read {
                room_id,
                last_read_id,
            } => {
                let u = Unread {
                    room_id: *room_id,
                    last_read_id: *last_read_id,
                    unread: 0,
                    mentions: 0,
                };
                self.unread.insert(*room_id, u);
            }
            ServerMsg::RoomsChanged { rooms } => {
                // The server sends no peer_left for rooms we can no longer see.
                self.set_rooms(rooms);
                let seen: Vec<i64> = self.rooms.iter().chain(&self.dms).map(|r| r.id).collect();
                self.voice.retain(|p| seen.contains(&p.room_id));
            }
            ServerMsg::UserCreated { user } | ServerMsg::UserUpdated { user } => {
                if let Some(me) = self
                    .me
                    .as_mut()
                    .filter(|me| me.user.user_id == user.user_id)
                {
                    me.user = user.clone();
                }
                self.users.insert(user.user_id.clone(), user.clone());
                self.title_dms();
            }
            ServerMsg::PeerJoined { peer } => {
                // One room at a time: a join elsewhere replaces the old entry.
                match self.voice.iter_mut().find(|p| p.user_id == peer.user_id) {
                    Some(p) => *p = peer.clone(),
                    None => self.voice.push(peer.clone()),
                }
            }
            ServerMsg::PeerLeft { user_id } => self.voice.retain(|p| p.user_id != *user_id),
            ServerMsg::PeerState {
                user_id,
                muted,
                deafened,
                force_muted,
            } => {
                for p in self.voice.iter_mut().filter(|p| p.user_id == *user_id) {
                    (p.muted, p.deafened, p.force_muted) = (*muted, *deafened, *force_muted);
                }
            }
            ServerMsg::PeerShare { user_id, share } => {
                for p in self.voice.iter_mut().filter(|p| p.user_id == *user_id) {
                    p.share = *share;
                }
            }
            _ => {}
        }
    }
}

/// The loaded slice of one room's history, oldest first.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct RoomLog {
    pub messages: Vec<Message>,
    pub more_before: bool,
}

/// Loaded history by room id.
#[derive(Debug, Default, Clone)]
pub struct Chat {
    pub rooms: HashMap<i64, RoomLog>,
}

/// True when `incoming` is older than `held`: the server raises `updated_at` on every change.
pub fn stale(held: &Message, incoming: &Message) -> bool {
    held.updated_at > incoming.updated_at
}

/// Inserts or replaces `m` by id, keeping the list sorted.
pub fn upsert(list: &mut Vec<Message>, m: &Message, insert: bool) {
    match list.binary_search_by_key(&m.id, |x| x.id) {
        Ok(i) if !stale(&list[i], m) => list[i] = m.clone(),
        Ok(_) => {}
        Err(i) if insert => list.insert(i, m.clone()),
        Err(_) => {}
    }
}

/// The newest page merged into what is held. With a gap between them the held slice is dropped. Held messages newer
/// than the page are kept: they arrived live while the page was in flight.
pub fn merge_latest(log: Option<&RoomLog>, page: &MessagePage) -> RoomLog {
    let last = page.messages.last().map_or(0, |m| m.id);
    let newer = log
        .into_iter()
        .flat_map(|l| &l.messages)
        .filter(|m| m.id > last)
        .cloned();
    let Some(first) = page.messages.first().map(|m| m.id) else {
        return RoomLog {
            messages: newer.collect(),
            more_before: page.more_before,
        };
    };
    let log = log.filter(|l| l.messages.iter().any(|m| m.id >= first));
    let Some(log) = log else {
        let messages = page.messages.iter().cloned().chain(newer).collect();
        return RoomLog {
            messages,
            more_before: page.more_before,
        };
    };
    let older: Vec<Message> = log
        .messages
        .iter()
        .filter(|m| m.id < first)
        .cloned()
        .collect();
    let merged = page
        .messages
        .iter()
        .map(|p| match log.messages.iter().find(|h| h.id == p.id) {
            Some(h) if stale(h, p) => h.clone(),
            _ => p.clone(),
        });
    let more_before = if older.is_empty() {
        page.more_before
    } else {
        log.more_before
    };
    RoomLog {
        messages: older.into_iter().chain(merged).chain(newer).collect(),
        more_before,
    }
}

/// An older page put in front of what is held.
pub fn merge_older(log: &mut RoomLog, page: &MessagePage) {
    let first = log.messages.first().map_or(i64::MAX, |m| m.id);
    let older = page.messages.iter().filter(|m| m.id < first).cloned();
    log.messages.splice(0..0, older);
    log.more_before = page.more_before;
}

impl Chat {
    pub fn apply(&mut self, m: &ServerMsg) {
        let (msg, insert) = match m {
            ServerMsg::MessageNew { message } => (message, true),
            ServerMsg::MessageUpdated { message } => (message, false),
            _ => return,
        };
        // Not loaded: the message comes with the newest page.
        if let Some(log) = self.rooms.get_mut(&msg.room_id) {
            upsert(&mut log.messages, msg, insert);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use vesper_protocol::{Role, ServerBuild};

    pub fn user(id: &str, name: &str) -> UserInfo {
        UserInfo {
            user_id: id.into(),
            username: name.to_lowercase(),
            display_name: name.into(),
            accent: String::new(),
            role: Role::Member,
            banned: false,
            bio: String::new(),
            avatar: None,
            ui_accent: None,
        }
    }

    pub fn room(id: i64, name: &str, kind: RoomKind, members: &[&str]) -> RoomInfo {
        RoomInfo {
            kind,
            id,
            name: name.into(),
            position: id,
            restricted: false,
            members: members.iter().map(|s| s.to_string()).collect(),
            archived: false,
        }
    }

    pub fn msg(id: i64, room_id: i64, author: &str, body: &str) -> Message {
        Message {
            id,
            room_id,
            author_id: author.into(),
            body: body.into(),
            created_at: 1_700_000_000_000 + id * 60_000,
            edited_at: None,
            updated_at: id,
            deleted: false,
            reply_to: None,
            reactions: vec![],
            attachments: vec![],
            mentions: vec![],
            pinned: false,
        }
    }

    pub fn hello() -> ServerMsg {
        ServerMsg::Hello {
            me: Me {
                user: user("a", "Ann"),
                session_id: 1,
                server: ServerBuild {
                    version: "1".into(),
                    build: "b".into(),
                },
                upload_max: 1,
            },
            users: vec![user("a", "Ann"), user("b", "Bob"), user("c", "Cy")],
            rooms: vec![
                room(1, "general", RoomKind::Room, &[]),
                RoomInfo {
                    archived: true,
                    ..room(2, "old", RoomKind::Room, &[])
                },
                room(3, "dm-x", RoomKind::Dm, &["a", "b"]),
                room(4, "dm-y", RoomKind::Dm, &["c", "a"]),
            ],
            voice: vec![Peer {
                user_id: "b".into(),
                name: "Bob".into(),
                room_id: 1,
                muted: false,
                deafened: false,
                force_muted: false,
                share: None,
            }],
            unread: vec![Unread {
                room_id: 1,
                last_read_id: 10,
                unread: 2,
                mentions: 0,
            }],
        }
    }

    #[test]
    fn hello_splits_rooms_and_names_dms() {
        let mut s = AppState::default();
        s.apply(&hello());
        assert_eq!(s.rooms.iter().map(|r| r.id).collect::<Vec<_>>(), [1]);
        assert_eq!(s.archived.len(), 1);
        assert_eq!(
            s.dms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["Bob", "Cy"]
        );
        assert_eq!(s.voice_in(1), ["Bob"]);
        assert_eq!(s.unread[&1].unread, 2);
    }

    #[test]
    fn message_new_counts_unread_and_mentions_but_not_mine_or_read() {
        let mut s = AppState::default();
        s.apply(&hello());
        let mut m = msg(11, 1, "b", "hi @Ann");
        m.mentions = vec!["a".into()];
        s.apply(&ServerMsg::MessageNew { message: m });
        assert_eq!((s.unread[&1].unread, s.unread[&1].mentions), (3, 1));
        s.apply(&ServerMsg::MessageNew {
            message: msg(12, 1, "a", "mine"),
        });
        s.apply(&ServerMsg::MessageNew {
            message: msg(9, 1, "b", "already read"),
        });
        assert_eq!(s.unread[&1].unread, 3);
        assert_eq!(s.activity[&1], 12);
        s.apply(&ServerMsg::MessageNew {
            message: msg(13, 3, "b", "new room entry"),
        });
        assert_eq!(s.unread[&3].unread, 1);
        s.apply(&ServerMsg::Read {
            room_id: 1,
            last_read_id: 12,
        });
        assert_eq!((s.unread[&1].unread, s.unread[&1].mentions), (0, 0));
    }

    #[test]
    fn dms_sort_unread_then_activity_then_name() {
        let mut s = AppState::default();
        s.apply(&hello());
        let names = |s: &AppState| {
            s.sorted_dms()
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&s), ["Bob", "Cy"]);
        s.activity.insert(4, 5);
        assert_eq!(names(&s), ["Cy", "Bob"]);
        s.apply(&ServerMsg::MessageNew {
            message: msg(3, 3, "b", "x"),
        });
        assert_eq!(
            names(&s),
            ["Bob", "Cy"],
            "unread first, even with older activity"
        );
    }

    #[test]
    fn peers_rooms_and_users() {
        let mut s = AppState::default();
        s.apply(&hello());
        let mut p = s.voice[0].clone();
        p.room_id = 3;
        s.apply(&ServerMsg::PeerJoined { peer: p });
        assert_eq!((s.voice.len(), s.voice_in(3).len()), (1, 1));
        s.apply(&ServerMsg::PeerState {
            user_id: "b".into(),
            muted: true,
            deafened: false,
            force_muted: false,
        });
        assert!(s.voice[0].muted);
        s.apply(&ServerMsg::RoomsChanged {
            rooms: vec![room(1, "general", RoomKind::Room, &[])],
        });
        assert!(
            s.voice.is_empty(),
            "peers of a room no longer seen are dropped"
        );
        assert!(s.dms.is_empty());
        s.apply(&ServerMsg::PeerLeft {
            user_id: "b".into(),
        });
        s.apply(&ServerMsg::RoomsChanged {
            rooms: vec![room(3, "dm-x", RoomKind::Dm, &["a", "b"])],
        });
        s.apply(&ServerMsg::UserUpdated {
            user: user("b", "Robert"),
        });
        assert_eq!(s.dms[0].name, "Robert");
        s.apply(&ServerMsg::UserUpdated {
            user: user("a", "Annie"),
        });
        assert_eq!(s.me.as_ref().unwrap().user.display_name, "Annie");
    }

    #[test]
    fn chat_upsert_edit_delete_and_stale_copies() {
        let mut c = Chat::default();
        c.apply(&ServerMsg::MessageNew {
            message: msg(1, 1, "b", "not loaded"),
        });
        assert!(c.rooms.is_empty());
        c.rooms.insert(
            1,
            RoomLog {
                messages: vec![msg(1, 1, "b", "one"), msg(3, 1, "b", "three")],
                more_before: false,
            },
        );
        c.apply(&ServerMsg::MessageNew {
            message: msg(2, 1, "b", "two"),
        });
        let ids = |c: &Chat| {
            c.rooms[&1]
                .messages
                .iter()
                .map(|m| m.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&c), [1, 2, 3]);
        let mut edit = msg(2, 1, "b", "two, edited");
        edit.updated_at = 50;
        c.apply(&ServerMsg::MessageUpdated {
            message: edit.clone(),
        });
        let mut older = msg(2, 1, "b", "late copy");
        older.updated_at = 40;
        c.apply(&ServerMsg::MessageUpdated { message: older });
        assert_eq!(c.rooms[&1].messages[1].body, "two, edited");
        let mut del = msg(3, 1, "b", "");
        (del.deleted, del.updated_at) = (true, 60);
        c.apply(&ServerMsg::MessageUpdated { message: del });
        assert!(c.rooms[&1].messages[2].deleted);
        c.apply(&ServerMsg::MessageUpdated {
            message: msg(9, 1, "b", "update of an unknown one"),
        });
        assert_eq!(ids(&c), [1, 2, 3]);
    }

    fn page(ids: &[i64], more_before: bool) -> MessagePage {
        MessagePage {
            messages: ids.iter().map(|&i| msg(i, 1, "b", "p")).collect(),
            more_before,
            more_after: false,
        }
    }

    #[test]
    fn merge_latest_keeps_live_and_newer_held_copies() {
        // first open: a message pushed while the page was in flight is kept
        let held = RoomLog {
            messages: vec![msg(7, 1, "b", "live")],
            more_before: false,
        };
        let got = merge_latest(Some(&held), &page(&[4, 5, 6], true));
        assert_eq!(
            got.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            [4, 5, 6, 7]
        );
        assert!(got.more_before);
        // overlap: older held messages stay, a newer held copy wins
        let mut edited = msg(5, 1, "b", "edited");
        edited.updated_at = 99;
        let held = RoomLog {
            messages: vec![msg(2, 1, "b", "x"), msg(3, 1, "b", "x"), edited],
            more_before: false,
        };
        let got = merge_latest(Some(&held), &page(&[3, 4, 5], true));
        assert_eq!(
            got.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            [2, 3, 4, 5]
        );
        assert_eq!(got.messages[3].body, "edited");
        assert!(!got.more_before, "the held slice reaches the start");
        // a gap: the held slice is dropped
        let held = RoomLog {
            messages: vec![msg(1, 1, "b", "x")],
            more_before: false,
        };
        let got = merge_latest(Some(&held), &page(&[8, 9], true));
        assert_eq!(
            got.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            [8, 9]
        );
        assert!(merge_latest(None, &page(&[], false)).messages.is_empty());
    }

    #[test]
    fn merge_older_prepends_without_duplicates() {
        let mut log = RoomLog {
            messages: vec![msg(5, 1, "b", "x"), msg(6, 1, "b", "x")],
            more_before: true,
        };
        merge_older(&mut log, &page(&[3, 4, 5], false));
        assert_eq!(
            log.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            [3, 4, 5, 6]
        );
        assert!(!log.more_before);
    }
}
