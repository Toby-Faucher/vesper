//! Everything the app shows, and what it asks the network for. No GPUI here: the views read a `Model` and call its
//! methods, and the tests drive the same methods against a real server.

use std::collections::HashMap;

use vesper_protocol::ServerMsg;

use crate::net::{Cmd, Event};
use crate::state::{AppState, Chat, RoomLog, merge_latest, merge_older};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// Reading the stored session.
    Starting,
    SignIn,
    Main,
}

pub const SESSION_ENDED: &str = "Your session ended. Sign in again.";
pub const DISCONNECTED: &str = "A moderator disconnected you. Sign out and in again to come back.";
pub const NOT_REMEMBERED: &str = "There is no keyring on this computer, so Vesper will not remember this sign in after it closes.";

#[derive(Debug)]
pub struct Model {
    pub screen: Screen,
    /// The server address, for the sign in field.
    pub server: String,
    /// Why the sign in screen is shown, or why the socket is not coming back.
    pub notice: String,
    /// The last thing that failed, in plain words.
    pub error: String,
    /// A sign in is in flight.
    pub busy: bool,
    /// No keyring: say so once.
    pub not_remembered: bool,
    pub state: AppState,
    pub chat: Chat,
    /// The room shown.
    pub active: Option<i64>,
    /// Between `hello` and the socket dropping.
    pub online: bool,
    /// The shown room is scrolled to its newest message.
    pub at_bottom: bool,
    /// The window has focus.
    pub focused: bool,
    pub loading_older: bool,
    /// The text of a send that failed, to put back in the composer.
    pub draft_back: Option<String>,
    /// The read marker sent per room, so it is sent once.
    read_sent: HashMap<i64, i64>,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            screen: Screen::Starting,
            server: String::new(),
            notice: String::new(),
            error: String::new(),
            busy: false,
            not_remembered: false,
            state: AppState::default(),
            chat: Chat::default(),
            active: None,
            online: false,
            at_bottom: true,
            focused: true,
            loading_older: false,
            draft_back: None,
            read_sent: HashMap::new(),
        }
    }
}

impl Model {
    /// The shown room's history.
    pub fn log(&self) -> Option<&RoomLog> {
        self.chat.rooms.get(&self.active?)
    }

    /// Back to nothing signed in. The server address and the keyring note stay.
    fn reset(&mut self, screen: Screen, notice: &str) {
        *self = Model {
            screen,
            notice: notice.into(),
            server: std::mem::take(&mut self.server),
            not_remembered: self.not_remembered,
            focused: self.focused,
            ..Model::default()
        };
    }

    pub fn apply(&mut self, ev: Event) -> Vec<Cmd> {
        let signed_in_only = !matches!(
            ev,
            Event::SignedIn { .. }
                | Event::SignedOut { .. }
                | Event::SignInFailed(_)
                | Event::NotRemembered
        );
        // Late events of a session that was signed out here.
        if signed_in_only && self.screen != Screen::Main {
            return vec![];
        }
        match ev {
            Event::SignedIn { me, server } => {
                self.reset(Screen::Main, "");
                self.server = server;
                self.state.me = Some(*me);
            }
            Event::SignedOut { server, notice } => {
                // A sign out here already reset; this only fills in what the network knows.
                if self.screen != Screen::SignIn || !notice.is_empty() {
                    self.reset(Screen::SignIn, &notice);
                }
                if let Some(s) = server {
                    self.server = s;
                }
            }
            Event::SignInFailed(e) => (self.busy, self.error) = (false, e),
            Event::NotRemembered => self.not_remembered = true,
            Event::SessionEnded => {
                self.reset(Screen::SignIn, SESSION_ENDED);
                return vec![Cmd::Forget];
            }
            Event::Disconnected => (self.online, self.notice) = (false, DISCONNECTED.into()),
            Event::Offline { .. } => self.online = false,
            Event::Server(m) => return self.server_msg(*m),
            Event::Latest { room, page } => {
                let log = merge_latest(self.chat.rooms.get(&room), &page);
                self.chat.rooms.insert(room, log);
            }
            Event::Older { room, page } => {
                self.loading_older = false;
                if let Some(log) = self.chat.rooms.get_mut(&room) {
                    merge_older(log, &page);
                }
            }
            Event::LoadFailed { room, error } => {
                self.loading_older = false;
                // A room that never loaded: the next open tries again.
                if self
                    .chat
                    .rooms
                    .get(&room)
                    .is_some_and(|l| l.messages.is_empty())
                {
                    self.chat.rooms.remove(&room);
                }
                self.error = error;
            }
            Event::Activity(list) => {
                // Live messages that came meanwhile are newer.
                for (room, id) in list {
                    self.state.activity.entry(room).or_insert(id);
                }
            }
            Event::Sent(m) => self.chat.apply(&ServerMsg::MessageNew { message: *m }),
            Event::SendFailed { room, body, error } => {
                self.error = error;
                if self.active == Some(room) {
                    self.draft_back = Some(body);
                }
            }
            Event::ReadFailed { room } => {
                self.read_sent.remove(&room);
            }
        }
        self.maybe_read()
    }

    fn server_msg(&mut self, m: ServerMsg) -> Vec<Cmd> {
        self.state.apply(&m);
        self.chat.apply(&m);
        let mut cmds = vec![];
        let shown = self.active.filter(|id| self.state.find_room(*id).is_some());
        if let ServerMsg::Hello { .. } = m {
            (self.online, self.notice) = (true, String::new());
            // Like resync(): events may have been missed while the socket was down, so drop the other rooms and
            // fetch the shown one's newest page again, merged into what is held.
            let keep = shown.or_else(|| self.state.rooms.first().map(|r| r.id));
            let kept = keep.and_then(|id| self.chat.rooms.remove(&id));
            self.chat.rooms.clear();
            if let Some(id) = keep {
                self.chat.rooms.insert(id, kept.unwrap_or_default());
                self.active = Some(id);
                cmds.push(Cmd::LoadLatest(id));
            }
            let dms: Vec<i64> = self.state.dms.iter().map(|r| r.id).collect();
            if !dms.is_empty() {
                cmds.push(Cmd::SeedActivity(dms));
            }
        } else if shown.is_none() && self.active.is_some() {
            // The shown room went away (a member list or an archive).
            self.active = None;
            if let Some(id) = self.state.rooms.first().map(|r| r.id) {
                cmds.extend(self.open_room(id));
            }
        }
        cmds.extend(self.maybe_read());
        cmds
    }

    /// Marks the shown room read when its newest message is on screen in a focused window, like the web timeline.
    fn maybe_read(&mut self) -> Vec<Cmd> {
        if self.screen != Screen::Main || !self.focused || !self.at_bottom {
            return vec![];
        }
        let Some(room) = self.active else {
            return vec![];
        };
        let Some(last) = self.log().and_then(|l| l.messages.last()).map(|m| m.id) else {
            return vec![];
        };
        let read = self.state.unread.get(&room).map_or(0, |u| u.last_read_id);
        if read >= last || self.read_sent.get(&room).is_some_and(|&s| s >= last) {
            return vec![];
        }
        self.read_sent.insert(room, last);
        vec![Cmd::MarkRead { room, id: last }]
    }

    pub fn sign_in(&mut self, server: &str, username: &str, password: &str) -> Vec<Cmd> {
        if self.busy {
            return vec![];
        }
        if server.trim().is_empty() || username.trim().is_empty() || password.is_empty() {
            self.error = "Fill in the server address, your username and your password.".into();
            return vec![];
        }
        (self.busy, self.error, self.notice) = (true, String::new(), String::new());
        self.server = server.trim().into();
        vec![Cmd::Login {
            server: self.server.clone(),
            username: username.trim().into(),
            password: password.into(),
        }]
    }

    pub fn sign_out(&mut self) -> Vec<Cmd> {
        self.reset(Screen::SignIn, "");
        vec![Cmd::Logout]
    }

    /// Shows a room: its newest page, unless it is held already.
    pub fn open_room(&mut self, id: i64) -> Vec<Cmd> {
        if self.active == Some(id) {
            return vec![];
        }
        (self.active, self.at_bottom, self.loading_older, self.error) =
            (Some(id), true, false, String::new());
        let mut cmds = vec![];
        // An empty log first, so a message pushed while the page is in flight is held and then merged.
        if let std::collections::hash_map::Entry::Vacant(e) = self.chat.rooms.entry(id) {
            e.insert(RoomLog::default());
            cmds.push(Cmd::LoadLatest(id));
        }
        cmds.extend(self.maybe_read());
        cmds
    }

    /// The message list moved. At the top, the next older page loads.
    pub fn scrolled(&mut self, at_top: bool, at_bottom: bool) -> Vec<Cmd> {
        self.at_bottom = at_bottom;
        let mut cmds = self.maybe_read();
        let (Some(room), Some(log)) = (self.active, self.log()) else {
            return cmds;
        };
        if at_top
            && log.more_before
            && !self.loading_older
            && let Some(first) = log.messages.first()
        {
            cmds.push(Cmd::LoadOlder {
                room,
                before: first.id,
            });
            self.loading_older = true;
        }
        cmds
    }

    pub fn set_focused(&mut self, focused: bool) -> Vec<Cmd> {
        self.focused = focused;
        self.maybe_read()
    }

    /// Sends the composer's text to the shown room. Blank text sends nothing.
    pub fn send(&mut self, body: &str) -> Vec<Cmd> {
        let (Some(room), body) = (self.active, body.trim()) else {
            return vec![];
        };
        if body.is_empty() {
            return vec![];
        }
        self.error.clear();
        vec![Cmd::Send {
            room,
            body: body.into(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::{hello, msg};
    use vesper_protocol::MessagePage;

    fn signed_in() -> Model {
        let mut m = Model::default();
        let ServerMsg::Hello { me, .. } = hello() else {
            unreachable!()
        };
        m.apply(Event::SignedIn {
            me: Box::new(me),
            server: "https://v.example".into(),
        });
        m
    }

    fn page(ids: &[i64], more_before: bool) -> MessagePage {
        MessagePage {
            messages: ids.iter().map(|&i| msg(i, 1, "b", "x")).collect(),
            more_before,
            more_after: false,
        }
    }

    #[test]
    fn hello_opens_the_first_room_and_seeds_dms() {
        let mut m = signed_in();
        let cmds = m.apply(Event::Server(Box::new(hello())));
        assert!(m.online);
        assert_eq!(m.active, Some(1));
        assert!(
            matches!(cmds.as_slice(), [Cmd::LoadLatest(1), Cmd::SeedActivity(d)] if d == &[3, 4])
        );
    }

    #[test]
    fn newest_page_marks_read_once_when_at_bottom_and_focused() {
        let mut m = signed_in();
        m.apply(Event::Server(Box::new(hello())));
        let cmds = m.apply(Event::Latest {
            room: 1,
            page: page(&[10, 11, 12], true),
        });
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::MarkRead { room: 1, id: 12 }]
        ));
        assert!(m.apply(Event::Activity(vec![])).is_empty(), "sent once");
        m.set_focused(false);
        m.apply(Event::Server(Box::new(ServerMsg::MessageNew {
            message: msg(13, 1, "b", "x"),
        })));
        assert!(matches!(
            m.set_focused(true).as_slice(),
            [Cmd::MarkRead { room: 1, id: 13 }]
        ));
    }

    #[test]
    fn scrolling_up_loads_older_once() {
        let mut m = signed_in();
        m.apply(Event::Server(Box::new(hello())));
        m.apply(Event::Latest {
            room: 1,
            page: page(&[10, 11], true),
        });
        let cmds = m.scrolled(true, false);
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::LoadOlder {
                room: 1,
                before: 10
            }]
        ));
        assert!(m.scrolled(true, false).is_empty(), "one at a time");
        m.apply(Event::Older {
            room: 1,
            page: page(&[8, 9], false),
        });
        assert_eq!(m.log().unwrap().messages.len(), 4);
        assert!(m.scrolled(true, false).is_empty(), "nothing more before");
    }

    #[test]
    fn reconnect_resyncs_the_shown_room_and_drops_the_others() {
        let mut m = signed_in();
        m.apply(Event::Server(Box::new(hello())));
        m.apply(Event::Latest {
            room: 1,
            page: page(&[10], false),
        });
        assert!(matches!(m.open_room(3).as_slice(), [Cmd::LoadLatest(3)]));
        m.apply(Event::Offline {
            retry_in: std::time::Duration::from_secs(1),
        });
        assert!(!m.online);
        let cmds = m.apply(Event::Server(Box::new(hello())));
        assert_eq!(m.active, Some(3));
        assert!(matches!(cmds.first(), Some(Cmd::LoadLatest(3))));
        assert_eq!(m.chat.rooms.keys().collect::<Vec<_>>(), [&3]);
    }

    #[test]
    fn session_end_and_sign_out_go_back_to_sign_in() {
        let mut m = signed_in();
        m.apply(Event::Server(Box::new(hello())));
        let cmds = m.apply(Event::SessionEnded);
        assert!(matches!(cmds.as_slice(), [Cmd::Forget]));
        assert_eq!(
            (m.screen, m.notice.as_str()),
            (Screen::SignIn, SESSION_ENDED)
        );
        assert!(m.state.me.is_none() && m.chat.rooms.is_empty());
        assert_eq!(
            m.server, "https://v.example",
            "the address stays for the next sign in"
        );
        assert!(
            m.apply(Event::Server(Box::new(hello()))).is_empty(),
            "late events are dropped"
        );
        let mut m = signed_in();
        assert!(matches!(m.sign_out().as_slice(), [Cmd::Logout]));
        assert_eq!(m.screen, Screen::SignIn);
    }

    #[test]
    fn sign_in_checks_the_fields_and_sends_once() {
        let mut m = Model::default();
        assert!(m.sign_in("", "ann", "pw").is_empty());
        assert!(!m.error.is_empty());
        assert!(matches!(
            m.sign_in(" v.example ", "ann", "pw").as_slice(),
            [Cmd::Login { .. }]
        ));
        assert!(m.sign_in("v.example", "ann", "pw").is_empty(), "busy");
        m.apply(Event::SignInFailed("Wrong username or password.".into()));
        assert!(!m.busy);
        assert_eq!(m.error, "Wrong username or password.");
    }

    #[test]
    fn send_trims_and_a_failed_send_gives_the_text_back() {
        let mut m = signed_in();
        m.apply(Event::Server(Box::new(hello())));
        assert!(m.send("  \n ").is_empty());
        assert!(
            matches!(m.send(" hi\n").as_slice(), [Cmd::Send { room: 1, body }] if body == "hi")
        );
        m.apply(Event::SendFailed {
            room: 1,
            body: "hi".into(),
            error: "Too many tries.".into(),
        });
        assert_eq!(m.draft_back.as_deref(), Some("hi"));
    }
}
