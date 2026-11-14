//! Everything the app shows, and what it asks the network for. No GPUI here: the views read a `Model` and call its
//! methods, and the tests drive the same methods against a real server.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use vesper_protocol::{ClientMsg, ErrorCode, Message, Role, RoomKind, ServerMsg};

use crate::net::{Cmd, Event};
use crate::settings::Settings;
use crate::state::{AppState, Chat, RoomLog, merge_latest, merge_older};
use crate::uploads::Tray;
use crate::voice::{Frame, Setup, VoiceCmd, VoiceEvent};

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
pub const HEADPHONES: &str =
    "Echo cancellation is off. Use headphones, or the others may hear themselves.";
pub const NOT_ONLINE: &str = "Not connected to the server. Wait a moment and try again.";
pub const CALL_ENDED: &str =
    "Your call ended. You may have joined from another device, or a moderator removed you.";
pub const ROOM_GONE: &str = "You no longer have access to that room.";
pub const REJOINING: &str = "Connection lost. You rejoin the call as soon as it is back.";
pub const CALL_DROPPED: &str = "Disconnected from the server.";
/// How long a call whose socket dropped waits for it to come back, like the web's `REJOIN_MS`.
pub const REJOIN_FOR: Duration = Duration::from_secs(60);
/// A share stops being watched this long after its view went away, like the web's `WATCH_GRACE_MS`, so a quick
/// look at another room costs no keyframe wait.
pub const WATCH_GRACE: Duration = Duration::from_secs(10);
/// Typing is sent at most this often (the web's `TYPING_EVERY_MS`; the server takes one per 3 s).
pub const TYPING_EVERY: Duration = Duration::from_secs(3);
/// "X is typing" shows this long after the last typing event (the web's `TYPING_MS`).
pub const TYPING_FOR: Duration = Duration::from_secs(5);
/// The upload limit until the first `Me` says it, as the web.
const UPLOAD_MAX: i64 = 25 << 20;
pub const UPLOADS_BUSY: &str = "Wait for the files to finish uploading.";
/// The web's three quick reactions.
pub const EMOJIS: [&str; 3] = ["\u{1f44d}", "\u{1f602}", "\u{2764}\u{fe0f}"];
/// The web's text for `share_stopped`. Only a sharer gets it, and the desktop app cannot share yet.
pub const SHARE_STOPPED: &str = "A moderator stopped your screen share.";

/// Whether the server and the engine have been told to watch one share, like the web's `WatchGate`.
#[derive(Debug, Default)]
struct Gate {
    on: bool,
    /// When `watch off` goes, once the share is no longer wanted.
    off_at: Option<Instant>,
}

/// The server's socket error codes in plain words, as `SIGNAL_MESSAGES` in `web/src/lib/signaling.ts`.
pub fn signal_text(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::RoomFull => "This room is full.",
        ErrorCode::Forbidden => "You cannot join this room.",
        ErrorCode::BadMessage => "The server did not accept that. Try again.",
        ErrorCode::RateLimited => "Too many tries. Wait a moment and try again.",
        ErrorCode::ShareCap => "Too many screens are being shared right now.",
        ErrorCode::Internal => "Something went wrong on the server. Try again in a moment.",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Idle,
    /// The engine is opening the devices and making the offer.
    Starting,
    /// `join` was sent.
    Joining,
    /// The server answered: in the call.
    Joined,
}

/// My voice call, like `voice.svelte.ts`.
#[derive(Debug)]
pub struct Call {
    pub phase: Phase,
    /// The call's room, from Join voice until the call ends.
    pub room: Option<i64>,
    /// Tags the engine's events, so a late one from an ended call is dropped.
    pub id: u64,
    /// Media flows.
    pub connected: bool,
    pub muted: bool,
    pub deafened: bool,
    pub force_muted: bool,
    pub me_speaking: bool,
    /// User ids of the others speaking now.
    pub speaking: Vec<String>,
    /// The room to join again once the socket is back, and since when.
    pub rejoin: Option<(i64, Instant)>,
    /// Why the last call ended or what went wrong in this one.
    pub error: String,
    /// What the noise filter does, or why DeepFilterNet is not running.
    pub filter: String,
    /// Why echo cancellation turned off during this call.
    pub echo_off: String,
    /// A lost device ("Your microphone was disconnected."), empty when both are there.
    pub devices: String,
    /// The headphones note is shown (echo cancellation off).
    pub headphones: bool,
    headphones_seen: bool,
    /// The screen share I chose to watch. One at a time.
    pub watching: Option<String>,
    /// The share view is on screen: the call's room is shown, not Settings. Set by the view.
    pub share_visible: bool,
    gates: HashMap<String, Gate>,
    /// The newest picture of the watched share. The view takes its pixels to draw; the next picture comes after.
    pub frame: Option<Frame>,
    /// Pictures of the watched share so far.
    pub frames: u64,
    /// Watched shares that lose packets or frames.
    pub struggling: HashSet<String>,
    /// Why the watched share went away.
    pub share_notice: String,
}

impl Default for Call {
    fn default() -> Self {
        Call {
            phase: Phase::Idle,
            room: None,
            id: 0,
            connected: false,
            muted: false,
            deafened: false,
            force_muted: false,
            me_speaking: false,
            speaking: vec![],
            rejoin: None,
            error: String::new(),
            filter: String::new(),
            echo_off: String::new(),
            devices: String::new(),
            headphones: false,
            headphones_seen: false,
            watching: None,
            share_visible: false,
            gates: HashMap::new(),
            frame: None,
            frames: 0,
            struggling: HashSet::new(),
            share_notice: String::new(),
        }
    }
}

impl Call {
    /// Back to no call. What lasts across calls stays. The server forgets the watches with the session, so the
    /// gates go without a `watch off`.
    fn end(&mut self, error: &str) {
        *self = Call {
            id: self.id,
            headphones: self.headphones,
            headphones_seen: self.headphones_seen,
            share_visible: self.share_visible,
            error: error.into(),
            ..Call::default()
        };
    }

    /// The pixels of the newest picture, for the view to draw. Taking them lets the next picture come.
    pub fn take_frame(&mut self) -> Option<Frame> {
        self.frame.take()
    }

    /// When the next `watch off` is due, for the view's timer.
    pub fn watch_deadline(&self) -> Option<Instant> {
        self.gates.values().filter_map(|g| g.off_at).min()
    }
}

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
    pub call: Call,
    /// Loaded at start and saved by the view; the model reads them when a call starts.
    pub settings: Settings,
    /// The push-to-talk key is held (or its release delay runs).
    pub ptt_open: bool,
    /// The message the next send replies to.
    pub reply_to: Option<Message>,
    /// My message open for editing, and why its last save failed.
    pub editing: Option<i64>,
    pub edit_error: String,
    /// The message whose delete waits for a confirm.
    pub confirm_delete: Option<i64>,
    /// The message whose reaction picker is open.
    pub picking: Option<i64>,
    /// Room id, then user id, then when the "is typing" line ends.
    pub typing: HashMap<i64, HashMap<String, Instant>>,
    last_typing: Option<Instant>,
    pub tray: Tray,
    /// The pins panel: its room, and the pins once loaded.
    pub pins: Option<(i64, Option<Vec<Message>>)>,
    /// A loaded message the list scrolls to.
    pub jump: Option<i64>,
    /// Where the last saved file went.
    pub saved: Option<PathBuf>,
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
            call: Call::default(),
            settings: Settings::default(),
            ptt_open: false,
            reply_to: None,
            editing: None,
            edit_error: String::new(),
            confirm_delete: None,
            picking: None,
            typing: HashMap::new(),
            last_typing: None,
            tray: Tray::default(),
            pins: None,
            jump: None,
            saved: None,
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
            settings: std::mem::take(&mut self.settings),
            ptt_open: self.ptt_open,
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
            Event::Disconnected => {
                (self.online, self.notice) = (false, DISCONNECTED.into());
                return self.end_call(CALL_DROPPED, false);
            }
            Event::Offline { .. } => {
                self.online = false;
                // The media goes with the socket; the call is joined again once a new one is up.
                let Some(room) = self.call.room else {
                    return vec![];
                };
                let cmds = self.end_call("", false);
                self.call.rejoin = Some((room, Instant::now()));
                return cmds;
            }
            Event::Voice { call, ev } => return self.voice_event(call, ev),
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
            Event::Sent { message, keys } => {
                self.tray.sent(&keys);
                if self.reply_to.as_ref().map(|r| r.id) == message.reply_to.as_ref().map(|r| r.id) {
                    self.reply_to = None;
                }
                self.chat
                    .apply(&ServerMsg::MessageNew { message: *message });
            }
            Event::Changed(m) => return self.server_msg(ServerMsg::MessageUpdated { message: *m }),
            Event::Edited(m) => {
                if self.editing == Some(m.id) {
                    (self.editing, self.edit_error) = (None, String::new());
                }
                return self.server_msg(ServerMsg::MessageUpdated { message: *m });
            }
            Event::ChangeFailed(e) => {
                if self.editing.is_some() {
                    self.edit_error = e;
                } else {
                    self.error = e;
                }
            }
            Event::Pins { room, pins } => match (&mut self.pins, pins) {
                (Some((r, list)), Ok(p)) if *r == room => *list = Some(p),
                (_, Ok(_)) => {}
                (_, Err(e)) => (self.pins, self.error) = (None, e),
            },
            Event::UploadProgress { key, progress } => self.tray.progress(key, progress),
            Event::Uploaded { key, attachment } => return self.tray.finished(key, attachment),
            Event::UploadFailed { key, fail } => {
                let max = self.upload_max();
                return self.tray.failed(key, fail, max);
            }
            // The view decodes nothing here; it keeps the pictures.
            Event::Image { .. } => {}
            Event::Downloaded { path, open } => {
                if !open {
                    self.saved = Some(path);
                }
            }
            Event::DownloadFailed(e) => self.error = e,
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
        match &m {
            ServerMsg::Typing { room_id, user_id } => {
                let until = Instant::now() + TYPING_FOR;
                self.typing
                    .entry(*room_id)
                    .or_default()
                    .insert(user_id.clone(), until);
            }
            ServerMsg::MessageNew { message } => {
                if let Some(t) = self.typing.get_mut(&message.room_id) {
                    t.remove(&message.author_id);
                }
            }
            // A pin or unpin, here or elsewhere: the open panel loads again.
            ServerMsg::MessageUpdated { message } => {
                if let Some((room, Some(list))) = &self.pins
                    && *room == message.room_id
                    && message.pinned != list.iter().any(|p| p.id == message.id)
                {
                    cmds.push(Cmd::LoadPins(*room));
                }
            }
            _ => {}
        }
        let shown = self.active.filter(|id| self.state.find_room(*id).is_some());
        if let ServerMsg::Hello { .. } = m {
            (self.online, self.notice) = (true, String::new());
            self.typing.clear();
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
            if let Some((room, since)) = self.call.rejoin.take() {
                if since.elapsed() < REJOIN_FOR {
                    cmds.extend(self.join_voice(room));
                } else {
                    self.call.error = CALL_DROPPED.into();
                }
            }
        } else if shown.is_none() && self.active.is_some() {
            // The shown room went away (a member list or an archive).
            self.active = None;
            if let Some(id) = self.state.rooms.first().map(|r| r.id) {
                cmds.extend(self.open_room(id));
            }
        }
        cmds.extend(self.call_msg(&m));
        if let Some(id) = self.call.watching.clone()
            && !self.sharers().iter().any(|(s, _)| *s == id)
        {
            let c = &mut self.call;
            c.share_notice = format!("{} stopped sharing.", self.state.name_of(&id));
            (c.watching, c.frame, c.frames) = (None, None, 0);
        }
        cmds.extend(self.sync_watches(Instant::now()));
        cmds.extend(self.maybe_read());
        cmds
    }

    /// The others sharing a screen in my call, as (user id, name).
    pub fn sharers(&self) -> Vec<(String, String)> {
        let me = self.state.my_id();
        self.state
            .voice
            .iter()
            .filter(|p| Some(p.room_id) == self.call.room && p.share.is_some() && p.user_id != me)
            .map(|p| (p.user_id.clone(), p.name.clone()))
            .collect()
    }

    /// Watch `user`'s screen share, or stop (`None`). Choosing another share stops the one before.
    pub fn watch(&mut self, user: Option<String>, now: Instant) -> Vec<Cmd> {
        let c = &mut self.call;
        if c.watching != user {
            (c.watching, c.frame, c.frames) = (user, None, 0);
            c.share_notice.clear();
        }
        self.sync_watches(now)
    }

    /// The view tells whether the share view is on screen.
    pub fn set_share_visible(&mut self, visible: bool, now: Instant) -> Vec<Cmd> {
        if self.call.share_visible == visible {
            return vec![];
        }
        self.call.share_visible = visible;
        self.sync_watches(now)
    }

    /// Sends `watch` on or off for each sharer, like the web's `syncWatches`: on as soon as the share is chosen and
    /// its view is on screen, off only `WATCH_GRACE` after that stops. The engine is told too, so it decodes only
    /// what is watched. A share that ended keeps its gate, so the off still goes out.
    pub fn sync_watches(&mut self, now: Instant) -> Vec<Cmd> {
        let sharing: HashSet<String> = self.sharers().into_iter().map(|(id, _)| id).collect();
        let c = &mut self.call;
        let ids: HashSet<String> = sharing.iter().chain(c.gates.keys()).cloned().collect();
        let mut cmds = vec![];
        for id in ids {
            let want = c.phase == Phase::Joined
                && c.share_visible
                && c.watching.as_ref() == Some(&id)
                && sharing.contains(&id);
            let g = c.gates.entry(id.clone()).or_default();
            let on = if want && !g.on {
                true
            } else if !want && g.on && now >= *g.off_at.get_or_insert(now + WATCH_GRACE) {
                false
            } else {
                if want {
                    g.off_at = None;
                } else if !g.on {
                    c.gates.remove(&id);
                }
                continue;
            };
            if on {
                *g = Gate { on, off_at: None };
            } else {
                c.gates.remove(&id);
            }
            cmds.push(Cmd::Signal(ClientMsg::Watch {
                user_id: id.clone(),
                on,
            }));
            cmds.push(Cmd::Voice(VoiceCmd::Watch { user_id: id, on }));
        }
        cmds
    }

    /// What a socket message does to my call, like `callEvent` in `web/src/lib/call.ts`.
    fn call_msg(&mut self, m: &ServerMsg) -> Vec<Cmd> {
        let c = &mut self.call;
        match m {
            ServerMsg::Joined { answer, .. } if c.phase == Phase::Joining => {
                c.phase = Phase::Joined;
                let mut cmds = vec![Cmd::Voice(VoiceCmd::Answer(answer.clone()))];
                // A force mute that came while joining.
                if c.muted || c.deafened {
                    cmds.push(self.voice_state());
                }
                cmds
            }
            ServerMsg::Offer { sdp } if c.phase == Phase::Joined => {
                vec![Cmd::Voice(VoiceCmd::Offer(sdp.clone()))]
            }
            // My own id: another device took the call, a moderator removed me, or access was lost.
            ServerMsg::PeerLeft { user_id }
                if c.phase == Phase::Joined && *user_id == self.state.my_id() =>
            {
                self.end_call(CALL_ENDED, false)
            }
            ServerMsg::ForceMuted { muted } => {
                // Lifting a force mute does not unmute.
                (c.force_muted, c.muted) = (*muted, c.muted || *muted);
                if c.phase == Phase::Idle {
                    return vec![];
                }
                vec![self.sending()]
            }
            ServerMsg::ShareStopped => {
                c.share_notice = SHARE_STOPPED.into();
                vec![]
            }
            ServerMsg::RoomsChanged { rooms }
                if c.room
                    .is_some_and(|id| !rooms.iter().any(|r| r.id == id && !r.archived)) =>
            {
                self.end_call(ROOM_GONE, true)
            }
            ServerMsg::Error { code, .. } => match c.phase {
                Phase::Starting | Phase::Joining => self.end_call(signal_text(*code), true),
                Phase::Joined => {
                    c.error = signal_text(*code).into();
                    vec![]
                }
                Phase::Idle => vec![],
            },
            _ => vec![],
        }
    }

    fn voice_event(&mut self, call: u64, ev: VoiceEvent) -> Vec<Cmd> {
        let c = &mut self.call;
        if call != c.id || c.phase == Phase::Idle {
            return vec![];
        }
        match ev {
            VoiceEvent::Offer(offer) if c.phase == Phase::Starting => {
                c.phase = Phase::Joining;
                let room_id = c.room.expect("a call has a room");
                vec![Cmd::Signal(ClientMsg::Join { room_id, offer })]
            }
            VoiceEvent::Answer(sdp) if c.phase == Phase::Joined => {
                vec![Cmd::Signal(ClientMsg::Answer { sdp })]
            }
            VoiceEvent::Connected => {
                c.connected = true;
                vec![]
            }
            VoiceEvent::Speaking { me, peers } => {
                (c.me_speaking, c.speaking) = (me, peers);
                vec![]
            }
            VoiceEvent::Filter(text) => {
                c.filter = text;
                vec![]
            }
            VoiceEvent::EchoOff(text) => {
                c.echo_off = text;
                vec![]
            }
            VoiceEvent::Devices(text) => {
                c.devices = text;
                vec![]
            }
            VoiceEvent::Frame(f) => {
                // A late picture of a share no longer watched is dropped.
                if c.watching.as_ref() == Some(&f.user_id) {
                    (c.frame, c.frames) = (Some(f), c.frames + 1);
                }
                vec![]
            }
            VoiceEvent::Struggling { user_id, on } => {
                if on {
                    c.struggling.insert(user_id);
                } else {
                    c.struggling.remove(&user_id);
                }
                vec![]
            }
            VoiceEvent::Failed(e) => self.end_call(&e, true),
            _ => vec![],
        }
    }

    /// Ends my call, or a pending rejoin. `leave` tells the server, when it knows of the call.
    fn end_call(&mut self, error: &str, leave: bool) -> Vec<Cmd> {
        let phase = self.call.phase;
        if phase == Phase::Idle && self.call.rejoin.is_none() {
            return vec![];
        }
        self.call.end(error);
        let mut cmds = vec![];
        if leave && matches!(phase, Phase::Joining | Phase::Joined) {
            cmds.push(Cmd::Signal(ClientMsg::Leave));
        }
        if phase != Phase::Idle {
            cmds.push(Cmd::Voice(VoiceCmd::Stop));
        }
        cmds
    }

    fn voice_state(&self) -> Cmd {
        Cmd::Signal(ClientMsg::SetVoiceState {
            muted: self.call.muted,
            deafened: self.call.deafened,
        })
    }

    /// Whether the mic sends: not muted, and with push to talk on, the key held. Like the web's `trackLive`.
    pub fn live(&self) -> bool {
        !self.call.muted && (!self.settings.push_to_talk || self.ptt_open)
    }

    /// The engine's side of `live`.
    fn sending(&self) -> Cmd {
        Cmd::Voice(VoiceCmd::Muted(!self.live()))
    }

    /// The push-to-talk key opened or closed the mic.
    pub fn set_ptt(&mut self, open: bool) -> Vec<Cmd> {
        let was = self.live();
        self.ptt_open = open;
        if self.call.phase == Phase::Idle || was == self.live() {
            return vec![];
        }
        vec![self.sending()]
    }

    /// New settings. Push to talk applies at once; the rest at the next call.
    pub fn set_settings(&mut self, s: Settings) -> Vec<Cmd> {
        let was = self.live();
        if !s.push_to_talk {
            self.ptt_open = false;
        }
        self.settings = s.clamped();
        if self.call.phase == Phase::Idle || was == self.live() {
            return vec![];
        }
        vec![self.sending()]
    }

    /// Join voice in `room`. A call in another room ends first.
    pub fn join_voice(&mut self, room: i64) -> Vec<Cmd> {
        if !self.online {
            self.call.error = NOT_ONLINE.into();
            return vec![];
        }
        if self.call.phase != Phase::Idle && self.call.room == Some(room) {
            return vec![];
        }
        let mut cmds = self.end_call("", true);
        self.call.end(""); // a new call starts unmuted; the server sends a force mute again if it holds
        let c = &mut self.call;
        (c.phase, c.room, c.id) = (Phase::Starting, Some(room), c.id + 1);
        let s = &self.settings;
        if !s.echo_cancel && !c.headphones_seen {
            (c.headphones, c.headphones_seen) = (true, true);
        }
        cmds.push(Cmd::Voice(VoiceCmd::Start {
            call: c.id,
            setup: Setup {
                bitrate_kbps: s.bitrate_kbps.clamp(32, 510),
                volumes: s.volumes.clone(),
                denoise: s.denoise,
                echo: s.echo_cancel,
                input: s.input_device.clone(),
                output: s.output_device.clone(),
            },
        }));
        if !self.live() {
            cmds.push(self.sending()); // push to talk: silent until the key
        }
        cmds
    }

    pub fn leave_voice(&mut self) -> Vec<Cmd> {
        self.end_call("", true)
    }

    /// Mute or unmute. Not while a moderator holds a force mute.
    pub fn toggle_mute(&mut self) -> Vec<Cmd> {
        let c = &mut self.call;
        if c.phase == Phase::Idle || c.force_muted {
            return vec![];
        }
        c.muted = !c.muted;
        vec![self.sending(), self.voice_state()]
    }

    /// Deafen silences everyone; it does not mute the microphone, as in the web app.
    pub fn toggle_deafen(&mut self) -> Vec<Cmd> {
        let c = &mut self.call;
        if c.phase == Phase::Idle {
            return vec![];
        }
        c.deafened = !c.deafened;
        vec![
            Cmd::Voice(VoiceCmd::Deafened(c.deafened)),
            self.voice_state(),
        ]
    }

    /// One person's volume in percent, 100 when not set.
    pub fn volume(&self, user_id: &str) -> f32 {
        self.settings.volumes.get(user_id).copied().unwrap_or(100.0)
    }

    /// One person's volume, 0 to 400 percent. Kept in the settings, which the view saves.
    pub fn set_volume(&mut self, user_id: &str, percent: f32) -> Vec<Cmd> {
        let percent = percent.clamp(0.0, crate::voice::mix::MAX_VOLUME);
        self.settings.volumes.insert(user_id.into(), percent);
        if self.call.phase == Phase::Idle {
            return vec![];
        }
        vec![Cmd::Voice(VoiceCmd::Volume {
            user_id: user_id.into(),
            percent,
        })]
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
        // What belongs to the room left. Its uploads go on and wait there.
        (
            self.reply_to,
            self.editing,
            self.confirm_delete,
            self.picking,
            self.pins,
        ) = (None, None, None, None, None);
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

    /// Sends the composer's text to the shown room, with the finished files and the reply. Blank text and no files
    /// sends nothing; files still uploading wait.
    pub fn send(&mut self, body: &str) -> Vec<Cmd> {
        let (Some(room), body) = (self.active, body.trim()) else {
            return vec![];
        };
        if self.tray.busy(room) {
            self.error = UPLOADS_BUSY.into();
            return vec![];
        }
        let attachments = self.tray.done(room);
        // A body is required: a message of files only is sent with their names, as the web does.
        let body = if body.is_empty() {
            let names: Vec<&str> = self
                .tray
                .of(room)
                .filter(|i| i.attachment.is_some())
                .map(|i| i.name.as_str())
                .collect();
            names.join(", ")
        } else {
            body.into()
        };
        if body.is_empty() {
            return vec![];
        }
        self.error.clear();
        vec![Cmd::Send {
            room,
            body,
            reply_to: self.reply_to.as_ref().map(|r| r.id),
            attachments,
        }]
    }

    /// The composer's text changed: `typing` goes out at most every `TYPING_EVERY`, only for real text.
    pub fn typed(&mut self, text: &str, now: Instant) -> Vec<Cmd> {
        let Some(room) = self.active else {
            return vec![];
        };
        if text.trim().is_empty()
            || !self.online
            || self
                .last_typing
                .is_some_and(|t| now.duration_since(t) < TYPING_EVERY)
        {
            return vec![];
        }
        self.last_typing = Some(now);
        vec![Cmd::Signal(ClientMsg::Typing { room_id: room })]
    }

    /// The names of the others typing in `room` at `now`, sorted.
    pub fn typers(&self, room: i64, now: Instant) -> Vec<String> {
        let me = self.state.my_id();
        let mut v: Vec<String> = self
            .typing
            .get(&room)
            .into_iter()
            .flatten()
            .filter(|(id, until)| **until > now && *id != me)
            .map(|(id, _)| self.state.name_of(id))
            .collect();
        v.sort();
        v
    }

    /// When the soonest "is typing" line still shown ends, for the view's timer.
    pub fn typing_deadline(&self, now: Instant) -> Option<Instant> {
        self.typing
            .values()
            .flat_map(|t| t.values())
            .filter(|t| **t > now)
            .min()
            .copied()
    }

    pub fn is_mod(&self) -> bool {
        self.state
            .me
            .as_ref()
            .is_some_and(|m| matches!(m.user.role, Role::Owner | Role::Moderator))
    }

    pub fn mine(&self, m: &Message) -> bool {
        m.author_id == self.state.my_id()
    }

    /// The author deletes, and a moderator or the owner deletes anyone's, as the server allows.
    pub fn can_delete(&self, m: &Message) -> bool {
        !m.deleted && (self.mine(m) || self.is_mod())
    }

    /// Anyone who sees a room pins; a DM has no pins.
    pub fn can_pin(&self) -> bool {
        self.active
            .and_then(|id| self.state.find_room(id))
            .is_some_and(|r| r.kind == RoomKind::Room)
    }

    pub fn upload_max(&self) -> i64 {
        self.state.me.as_ref().map_or(UPLOAD_MAX, |m| m.upload_max)
    }

    fn find(&self, id: i64) -> Option<&Message> {
        self.log()?.messages.iter().find(|m| m.id == id)
    }

    pub fn reply(&mut self, id: i64) {
        self.reply_to = self.find(id).filter(|m| !m.deleted).cloned();
    }

    pub fn cancel_reply(&mut self) {
        self.reply_to = None;
    }

    /// Opens my message `id` for editing; the body to put in the edit box.
    pub fn start_edit(&mut self, id: i64) -> Option<String> {
        let m = self.find(id).filter(|m| self.mine(m) && !m.deleted)?;
        let body = m.body.clone();
        (self.editing, self.edit_error, self.confirm_delete) = (Some(id), String::new(), None);
        Some(body)
    }

    /// Up in an empty composer: my newest loaded message that is not deleted.
    pub fn edit_last(&mut self) -> Option<(i64, String)> {
        let me = self.state.my_id();
        let id = self
            .log()?
            .messages
            .iter()
            .rev()
            .find(|m| m.author_id == me && !m.deleted)?
            .id;
        Some((id, self.start_edit(id)?))
    }

    pub fn save_edit(&mut self, body: &str) -> Vec<Cmd> {
        let (Some(id), body) = (self.editing, body.trim()) else {
            return vec![];
        };
        if body.is_empty() {
            return vec![];
        }
        self.edit_error.clear();
        vec![Cmd::Edit {
            id,
            body: body.into(),
        }]
    }

    pub fn cancel_edit(&mut self) {
        (self.editing, self.edit_error) = (None, String::new());
    }

    /// Delete asks first; `confirmed` sends it.
    pub fn delete(&mut self, id: i64, confirmed: bool) -> Vec<Cmd> {
        if !self.find(id).is_some_and(|m| self.can_delete(m)) {
            return vec![];
        }
        if !confirmed {
            (self.confirm_delete, self.picking) = (Some(id), None);
            return vec![];
        }
        self.confirm_delete = None;
        vec![Cmd::Delete(id)]
    }

    pub fn cancel_delete(&mut self) {
        self.confirm_delete = None;
    }

    pub fn toggle_picker(&mut self, id: i64) {
        self.picking = if self.picking == Some(id) {
            None
        } else {
            Some(id)
        };
    }

    /// Adds my `emoji` to message `id`, or takes it off when it is there.
    pub fn react(&mut self, id: i64, emoji: &str) -> Vec<Cmd> {
        let me = self.state.my_id();
        let Some(m) = self.find(id).filter(|m| !m.deleted) else {
            return vec![];
        };
        let on = !m
            .reactions
            .iter()
            .any(|r| r.emoji == emoji && r.user_ids.iter().any(|u| u == me));
        self.picking = None;
        vec![Cmd::React {
            id,
            emoji: emoji.into(),
            on,
        }]
    }

    pub fn pin(&mut self, id: i64) -> Vec<Cmd> {
        match self.find(id).filter(|m| !m.deleted) {
            Some(m) if self.can_pin() => vec![Cmd::Pin { id, on: !m.pinned }],
            _ => vec![],
        }
    }

    /// Opens or closes the pins of the shown room.
    pub fn toggle_pins(&mut self) -> Vec<Cmd> {
        match (self.pins.take(), self.active) {
            (None, Some(room)) if self.can_pin() => {
                self.pins = Some((room, None));
                vec![Cmd::LoadPins(room)]
            }
            _ => vec![],
        }
    }

    /// Scrolls to a message when it is loaded in the shown room. Returns whether it is.
    pub fn jump_to(&mut self, id: i64) -> bool {
        let found = self.find(id).is_some();
        if found {
            self.jump = Some(id);
        }
        found
    }

    /// Files (path, size) for the shown room's tray.
    pub fn attach(&mut self, files: Vec<(PathBuf, u64)>) -> Vec<Cmd> {
        let Some(room) = self.active else {
            return vec![];
        };
        let max = self.upload_max();
        self.tray.add(room, files, max)
    }

    pub fn remove_upload(&mut self, key: u64) -> Vec<Cmd> {
        self.tray.remove(key)
    }

    pub fn retry_upload(&mut self, key: u64) -> Vec<Cmd> {
        self.tray.retry(key)
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

    fn voice(m: &mut Model, ev: VoiceEvent) -> Vec<Cmd> {
        let call = m.call.id;
        m.apply(Event::Voice { call, ev })
    }

    fn server(m: &mut Model, msg: ServerMsg) -> Vec<Cmd> {
        m.apply(Event::Server(Box::new(msg)))
    }

    /// Signed in, online, and in the call in room 1.
    fn in_call() -> Model {
        let mut m = signed_in();
        server(&mut m, hello());
        assert!(matches!(
            m.join_voice(1).as_slice(),
            [Cmd::Voice(VoiceCmd::Start {
                setup: Setup {
                    bitrate_kbps: 64,
                    ..
                },
                ..
            })]
        ));
        let cmds = voice(&mut m, VoiceEvent::Offer("o".into()));
        assert!(
            matches!(cmds.as_slice(), [Cmd::Signal(ClientMsg::Join { room_id: 1, offer })] if offer == "o")
        );
        let joined = ServerMsg::Joined {
            user_id: "a".into(),
            answer: "ans".into(),
            peers: vec![],
        };
        assert!(
            matches!(server(&mut m, joined).as_slice(), [Cmd::Voice(VoiceCmd::Answer(a))] if a == "ans")
        );
        assert_eq!(m.call.phase, Phase::Joined);
        m
    }

    #[test]
    fn join_offer_answer_and_renegotiation() {
        let mut m = signed_in();
        assert!(m.join_voice(1).is_empty(), "not before hello");
        assert_eq!(m.call.error, NOT_ONLINE);
        let mut m = in_call();
        assert!(!m.call.headphones, "echo cancellation is on");
        assert!(m.join_voice(1).is_empty(), "already in it");
        let cmds = server(&mut m, ServerMsg::Offer { sdp: "re".into() });
        assert!(matches!(cmds.as_slice(), [Cmd::Voice(VoiceCmd::Offer(s))] if s == "re"));
        let cmds = voice(&mut m, VoiceEvent::Answer("ra".into()));
        assert!(matches!(cmds.as_slice(), [Cmd::Signal(ClientMsg::Answer { sdp })] if sdp == "ra"));
        voice(&mut m, VoiceEvent::Connected);
        voice(
            &mut m,
            VoiceEvent::Speaking {
                me: true,
                peers: vec!["b".into()],
            },
        );
        assert!(m.call.connected && m.call.me_speaking && m.call.speaking == ["b"]);
        // An event of an older call is dropped.
        assert!(
            m.apply(Event::Voice {
                call: 0,
                ev: VoiceEvent::Failed("old".into())
            })
            .is_empty()
        );
        assert_eq!(m.call.phase, Phase::Joined);
        // Another room: leave this call, start the next.
        let cmds = m.join_voice(3);
        assert!(matches!(
            cmds.as_slice(),
            [
                Cmd::Signal(ClientMsg::Leave),
                Cmd::Voice(VoiceCmd::Stop),
                Cmd::Voice(VoiceCmd::Start { .. })
            ]
        ));
        assert_eq!((m.call.room, m.call.phase), (Some(3), Phase::Starting));
    }

    #[test]
    fn mute_deafen_volume_and_leave() {
        let mut m = in_call();
        let cmds = m.toggle_mute();
        assert!(matches!(
            cmds.as_slice(),
            [
                Cmd::Voice(VoiceCmd::Muted(true)),
                Cmd::Signal(ClientMsg::SetVoiceState {
                    muted: true,
                    deafened: false
                })
            ]
        ));
        let cmds = m.toggle_deafen();
        assert!(matches!(
            cmds.as_slice(),
            [
                Cmd::Voice(VoiceCmd::Deafened(true)),
                Cmd::Signal(ClientMsg::SetVoiceState {
                    muted: true,
                    deafened: true
                })
            ]
        ));
        let cmds = m.set_volume("b", 900.0);
        assert!(
            matches!(cmds.as_slice(), [Cmd::Voice(VoiceCmd::Volume { percent, .. })] if *percent == 400.0)
        );
        let cmds = m.leave_voice();
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::Signal(ClientMsg::Leave), Cmd::Voice(VoiceCmd::Stop)]
        ));
        assert_eq!(m.call.phase, Phase::Idle);
        assert_eq!(m.volume("b"), 400.0, "volumes outlive the call");
        assert_eq!(
            m.settings.volumes["b"], 400.0,
            "in the settings, which the view saves"
        );
        assert!(m.toggle_mute().is_empty());
        let cmds = m.join_voice(1);
        assert!(
            matches!(&cmds[..], [Cmd::Voice(VoiceCmd::Start { setup, .. })] if setup.volumes["b"] == 400.0)
        );
        assert!(
            !m.call.muted && !m.call.deafened,
            "a new call starts unmuted"
        );
    }

    #[test]
    fn push_to_talk_opens_the_mic_only_while_held_and_never_when_muted() {
        let muted = |cmds: &[Cmd]| match cmds {
            [Cmd::Voice(VoiceCmd::Muted(m))] => Some(*m),
            [] => None,
            other => panic!("{other:?}"),
        };
        let mut m = in_call();
        assert!(m.live(), "voice activity: open while not muted");
        let ptt = Settings {
            push_to_talk: true,
            ..Settings::default()
        };
        assert_eq!(
            muted(&m.set_settings(ptt.clone())),
            Some(true),
            "push to talk on closes it"
        );
        assert_eq!(muted(&m.set_ptt(true)), Some(false), "press");
        assert_eq!(muted(&m.set_ptt(true)), None);
        assert_eq!(muted(&m.set_ptt(false)), Some(true), "release");

        // Mute wins over the key, and the key does not unmute.
        m.set_ptt(true);
        assert!(matches!(
            m.toggle_mute().as_slice(),
            [Cmd::Voice(VoiceCmd::Muted(true)), _]
        ));
        assert_eq!(muted(&m.set_ptt(false)), None);
        assert_eq!(muted(&m.set_ptt(true)), None);
        assert!(!m.live());
        assert!(matches!(
            m.toggle_mute().as_slice(),
            [Cmd::Voice(VoiceCmd::Muted(false)), _]
        ));
        // A force mute closes it while the key is held.
        assert_eq!(
            muted(&server(&mut m, ServerMsg::ForceMuted { muted: true })),
            Some(true)
        );
        server(&mut m, ServerMsg::ForceMuted { muted: false });
        m.toggle_mute();
        assert!(m.live());

        // Turning push to talk off opens the mic again; a new call with it on starts closed.
        let off = Settings::default();
        m.set_ptt(false);
        assert_eq!(muted(&m.set_settings(off)), Some(false));
        m.set_settings(ptt);
        m.leave_voice();
        let cmds = m.join_voice(1);
        assert!(matches!(
            cmds.as_slice(),
            [
                Cmd::Voice(VoiceCmd::Start { .. }),
                Cmd::Voice(VoiceCmd::Muted(true))
            ]
        ));
        assert!(m.set_ptt(true).len() == 1 && m.live());
    }

    #[test]
    fn the_call_starts_with_the_settings_and_shows_the_filter() {
        let mut m = in_call();
        voice(&mut m, VoiceEvent::Filter("Noise filter: off.".into()));
        assert_eq!(m.call.filter, "Noise filter: off.");
        m.set_settings(Settings {
            denoise: crate::settings::Denoise::Gate,
            bitrate_kbps: 1000,
            input_device: Some("USB".into()),
            ..Settings::default()
        });
        m.leave_voice();
        assert!(m.call.filter.is_empty(), "cleared with the call");
        let cmds = m.join_voice(1);
        let [Cmd::Voice(VoiceCmd::Start { setup, .. })] = cmds.as_slice() else {
            panic!("{cmds:?}")
        };
        assert_eq!(
            (setup.bitrate_kbps, setup.denoise, setup.input.as_deref()),
            (510, crate::settings::Denoise::Gate, Some("USB"))
        );
        assert!(setup.echo && !m.call.headphones);
    }

    #[test]
    fn echo_off_shows_the_headphones_note_once_and_notices_clear_with_the_call() {
        let mut m = in_call();
        voice(
            &mut m,
            VoiceEvent::Devices("Your speakers were disconnected.".into()),
        );
        voice(&mut m, VoiceEvent::EchoOff("slow".into()));
        assert_eq!(
            (m.call.devices.as_str(), m.call.echo_off.as_str()),
            ("Your speakers were disconnected.", "slow")
        );
        m.set_settings(Settings {
            echo_cancel: false,
            ..Settings::default()
        });
        m.leave_voice();
        assert!(m.call.devices.is_empty() && m.call.echo_off.is_empty());
        let cmds = m.join_voice(1);
        assert!(matches!(&cmds[..], [Cmd::Voice(VoiceCmd::Start { setup, .. })] if !setup.echo));
        assert!(m.call.headphones);
        m.call.headphones = false;
        m.leave_voice();
        m.join_voice(1);
        assert!(!m.call.headphones, "once");
    }

    #[test]
    fn force_mute_holds_and_lifting_does_not_unmute() {
        let mut m = in_call();
        let cmds = server(&mut m, ServerMsg::ForceMuted { muted: true });
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::Voice(VoiceCmd::Muted(true))]
        ));
        assert!(m.toggle_mute().is_empty());
        server(&mut m, ServerMsg::ForceMuted { muted: false });
        assert!(m.call.muted && !m.call.force_muted);
        assert_eq!(m.toggle_mute().len(), 2);
    }

    #[test]
    fn the_call_ends_on_my_peer_left_a_lost_room_or_an_engine_failure() {
        let mut m = in_call();
        assert!(
            server(
                &mut m,
                ServerMsg::PeerLeft {
                    user_id: "b".into()
                }
            )
            .is_empty()
        );
        let cmds = server(
            &mut m,
            ServerMsg::PeerLeft {
                user_id: "a".into(),
            },
        );
        assert!(matches!(cmds.as_slice(), [Cmd::Voice(VoiceCmd::Stop)]));
        assert_eq!(m.call.error, CALL_ENDED);

        let mut m = in_call();
        let cmds = server(
            &mut m,
            ServerMsg::RoomsChanged {
                rooms: vec![crate::state::tests::room(
                    3,
                    "dm",
                    vesper_protocol::RoomKind::Dm,
                    &["a", "b"],
                )],
            },
        );
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::Signal(ClientMsg::Leave), Cmd::Voice(VoiceCmd::Stop)]
        ));
        assert_eq!(m.call.error, ROOM_GONE);

        let mut m = in_call();
        voice(
            &mut m,
            VoiceEvent::Failed("No microphone was found.".into()),
        );
        assert_eq!(m.call.error, "No microphone was found.");
        assert_eq!(m.call.phase, Phase::Idle);

        // Refused at join: the server's reason.
        let mut m = signed_in();
        server(&mut m, hello());
        m.join_voice(1);
        voice(&mut m, VoiceEvent::Offer("o".into()));
        server(
            &mut m,
            ServerMsg::Error {
                code: ErrorCode::RoomFull,
                message: String::new(),
            },
        );
        assert_eq!(
            (m.call.phase, m.call.error.as_str()),
            (Phase::Idle, "This room is full.")
        );
    }

    #[test]
    fn a_dropped_socket_rejoins_after_the_next_hello() {
        let mut m = in_call();
        let cmds = m.apply(Event::Offline {
            retry_in: std::time::Duration::from_secs(1),
        });
        assert!(matches!(cmds.as_slice(), [Cmd::Voice(VoiceCmd::Stop)]));
        assert!(m.call.rejoin.is_some_and(|(r, _)| r == 1));
        assert!(
            m.apply(Event::Offline {
                retry_in: std::time::Duration::from_secs(2)
            })
            .is_empty()
        );
        let cmds = server(&mut m, hello());
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Voice(VoiceCmd::Start { .. })))
        );
        assert_eq!((m.call.phase, m.call.room), (Phase::Starting, Some(1)));
        assert!(m.call.rejoin.is_none());

        // Too late: give up.
        let mut m = in_call();
        m.apply(Event::Offline {
            retry_in: std::time::Duration::from_secs(1),
        });
        m.call.rejoin = m.call.rejoin.map(|(r, t)| (r, t - REJOIN_FOR));
        let cmds = server(&mut m, hello());
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Voice(_))));
        assert_eq!(m.call.error, CALL_DROPPED);
    }

    #[test]
    fn send_trims_and_a_failed_send_gives_the_text_back() {
        let mut m = signed_in();
        m.apply(Event::Server(Box::new(hello())));
        assert!(m.send("  \n ").is_empty());
        assert!(
            matches!(m.send(" hi\n").as_slice(), [Cmd::Send { room: 1, body, .. }] if body == "hi")
        );
        m.apply(Event::SendFailed {
            room: 1,
            body: "hi".into(),
            error: "Too many tries.".into(),
        });
        assert_eq!(m.draft_back.as_deref(), Some("hi"));
    }

    fn share(m: &mut Model, user: &str, on: bool) -> Vec<Cmd> {
        let share = on.then_some(vesper_protocol::ShareInfo {
            fps: 60,
            bitrate_kbps: 8000,
        });
        let user_id = user.into();
        server(m, ServerMsg::PeerShare { user_id, share })
    }

    /// The `watch` message and the engine's `Watch` for each turn, in order.
    fn watches(cmds: &[Cmd]) -> Vec<(String, bool)> {
        let mut out = vec![];
        for c in cmds {
            match c {
                Cmd::Signal(ClientMsg::Watch { user_id, on }) => out.push((user_id.clone(), *on)),
                Cmd::Voice(VoiceCmd::Watch { user_id, on }) => {
                    assert_eq!(
                        out.last(),
                        Some(&(user_id.clone(), *on)),
                        "the engine follows"
                    )
                }
                _ => {}
            }
        }
        out
    }

    fn picture(user: &str) -> VoiceEvent {
        VoiceEvent::Frame(Frame {
            user_id: user.into(),
            width: 2,
            height: 1,
            bgra: std::sync::Arc::new(vec![0; 8]),
        })
    }

    #[test]
    fn watching_needs_the_view_on_screen_and_stops_after_the_grace() {
        let mut m = in_call();
        let t = Instant::now();
        assert!(m.sharers().is_empty());
        assert!(
            watches(&share(&mut m, "b", true)).is_empty(),
            "shown, not watched"
        );
        assert_eq!(m.sharers(), [("b".to_string(), "Bob".to_string())]);

        // Chosen, but the view is not on screen: nothing yet. On screen: watch on, once.
        assert!(m.watch(Some("b".into()), t).is_empty());
        assert_eq!(watches(&m.set_share_visible(true, t)), [("b".into(), true)]);
        assert!(m.sync_watches(t).is_empty());

        // Pictures of the watched share are kept; others are dropped.
        voice(&mut m, picture("b"));
        voice(&mut m, picture("c"));
        assert_eq!(m.call.frames, 1);
        assert!(m.call.take_frame().is_some_and(|f| f.user_id == "b"));
        assert!(m.call.take_frame().is_none());

        // Off screen: off only after the grace; back within it: nothing sent at all.
        assert!(m.set_share_visible(false, t).is_empty());
        assert_eq!(m.call.watch_deadline(), Some(t + WATCH_GRACE));
        assert!(m.set_share_visible(true, t + WATCH_GRACE / 2).is_empty());
        assert_eq!(m.call.watch_deadline(), None);
        assert!(m.watch(None, t).is_empty());
        assert!(m.sync_watches(t + WATCH_GRACE / 2).is_empty());
        assert_eq!(
            watches(&m.sync_watches(t + WATCH_GRACE)),
            [("b".into(), false)]
        );
        assert!(m.sync_watches(t + WATCH_GRACE * 2).is_empty(), "sent once");

        // Struggling, as the engine says.
        assert!(m.watch(Some("b".into()), t).len() == 2);
        voice(
            &mut m,
            VoiceEvent::Struggling {
                user_id: "b".into(),
                on: true,
            },
        );
        assert!(m.call.struggling.contains("b"));
        voice(
            &mut m,
            VoiceEvent::Struggling {
                user_id: "b".into(),
                on: false,
            },
        );
        assert!(m.call.struggling.is_empty());
    }

    #[test]
    fn a_share_ending_clears_the_view_and_a_call_ending_forgets_the_watches() {
        let mut m = in_call();
        share(&mut m, "b", true);
        let t = Instant::now();
        m.set_share_visible(true, t);
        assert_eq!(watches(&m.watch(Some("b".into()), t)), [("b".into(), true)]);
        voice(&mut m, picture("b"));

        // Bob stops: the picture goes at once with a note; the off goes after the grace.
        assert!(watches(&share(&mut m, "b", false)).is_empty());
        assert_eq!((m.call.watching.as_deref(), m.call.frames), (None, 0));
        assert!(m.call.frame.is_none());
        assert_eq!(m.call.share_notice, "Bob stopped sharing.");
        assert!(m.call.watch_deadline().is_some());
        let later = Instant::now() + WATCH_GRACE;
        assert_eq!(watches(&m.sync_watches(later)), [("b".into(), false)]);

        // Watching again, then the call ends: the server forgets watches with the session, so nothing is sent.
        share(&mut m, "b", true);
        m.watch(Some("b".into()), t);
        let cmds = m.leave_voice();
        assert!(watches(&cmds).is_empty());
        assert!(m.call.watching.is_none() && m.call.watch_deadline().is_none());
        assert!(m.call.share_visible, "the view's state stays");

        // share_stopped goes only to a sharer; its text is the web's.
        let mut m = in_call();
        server(&mut m, ServerMsg::ShareStopped);
        assert_eq!(m.call.share_notice, SHARE_STOPPED);
    }

    fn chat() -> Model {
        let mut m = signed_in();
        m.apply(Event::Server(Box::new(hello())));
        let mut mine = msg(11, 1, "a", "mine");
        mine.reactions = vec![vesper_protocol::Reaction {
            emoji: EMOJIS[0].into(),
            user_ids: vec!["a".into()],
        }];
        m.apply(Event::Latest {
            room: 1,
            page: MessagePage {
                messages: vec![msg(10, 1, "b", "theirs"), mine, msg(12, 1, "b", "last")],
                more_before: false,
                more_after: false,
            },
        });
        m
    }

    #[test]
    fn typing_goes_out_at_most_every_3_s_and_shows_for_5_s() {
        let mut m = chat();
        let t0 = Instant::now();
        assert!(m.typed("  ", t0).is_empty(), "blank text is not typing");
        let typing = |c: &[Cmd]| matches!(c, [Cmd::Signal(ClientMsg::Typing { room_id: 1 })]);
        assert!(typing(&m.typed("h", t0)));
        assert!(m.typed("hi", t0 + Duration::from_millis(2999)).is_empty());
        assert!(typing(&m.typed("hi!", t0 + TYPING_EVERY)));
        m.online = false;
        assert!(
            m.typed("hi!!", t0 + 2 * TYPING_EVERY).is_empty(),
            "not while offline"
        );

        let now = Instant::now();
        server(
            &mut m,
            ServerMsg::Typing {
                room_id: 1,
                user_id: "b".into(),
            },
        );
        server(
            &mut m,
            ServerMsg::Typing {
                room_id: 1,
                user_id: "a".into(),
            },
        );
        assert_eq!(m.typers(1, now), ["Bob"], "not me");
        assert_eq!(
            m.typers(1, now + TYPING_FOR + Duration::from_millis(10)),
            Vec::<String>::new()
        );
        assert!(m.typing_deadline(now).is_some());
        assert!(
            m.typing_deadline(now + TYPING_FOR + Duration::from_millis(10))
                .is_none()
        );
        server(
            &mut m,
            ServerMsg::MessageNew {
                message: msg(13, 1, "b", "sent"),
            },
        );
        assert!(m.typers(1, now).is_empty(), "a message ends it");
    }

    #[test]
    fn reply_edit_delete_react_and_pin() {
        let mut m = chat();
        m.reply(10);
        let cmds = m.send("ok");
        assert!(
            matches!(
                cmds.as_slice(),
                [Cmd::Send {
                    reply_to: Some(10),
                    ..
                }]
            ),
            "{cmds:?}"
        );
        let mut sent = msg(14, 1, "a", "ok");
        sent.reply_to = Some(vesper_protocol::ReplyPreview {
            id: 10,
            author_id: "b".into(),
            excerpt: "theirs".into(),
        });
        m.apply(Event::Sent {
            message: Box::new(sent),
            keys: vec![],
        });
        assert!(m.reply_to.is_none());

        // Up in an empty composer opens my newest message.
        assert_eq!(m.edit_last(), Some((14, "ok".into())));
        assert!(m.start_edit(10).is_none(), "not someone else's");
        assert!(
            matches!(m.save_edit(" fixed ").as_slice(), [Cmd::Edit { id: 14, body }] if body == "fixed")
        );
        m.apply(Event::ChangeFailed(
            "Too many tries. Wait a minute and try again.".into(),
        ));
        assert_eq!(m.editing, Some(14));
        assert!(!m.edit_error.is_empty());
        let mut edited = msg(14, 1, "a", "fixed");
        edited.updated_at = 99;
        m.apply(Event::Edited(Box::new(edited)));
        assert_eq!(
            (
                m.editing,
                m.log().unwrap().messages.last().unwrap().body.as_str()
            ),
            (None, "fixed")
        );

        // Delete asks first; a member cannot delete someone else's.
        assert!(m.delete(11, false).is_empty());
        assert_eq!(m.confirm_delete, Some(11));
        assert!(matches!(m.delete(11, true).as_slice(), [Cmd::Delete(11)]));
        assert!(m.delete(10, false).is_empty() && m.confirm_delete.is_none());
        m.state.me.as_mut().unwrap().user.role = Role::Moderator;
        assert!(
            matches!(m.delete(10, true).as_slice(), [Cmd::Delete(10)]),
            "a moderator can"
        );

        // My reaction comes off, a new one goes on.
        assert!(matches!(
            m.react(11, EMOJIS[0]).as_slice(),
            [Cmd::React {
                id: 11,
                on: false,
                ..
            }]
        ));
        assert!(matches!(
            m.react(11, EMOJIS[1]).as_slice(),
            [Cmd::React {
                id: 11,
                on: true,
                ..
            }]
        ));
        assert!(matches!(
            m.pin(10).as_slice(),
            [Cmd::Pin { id: 10, on: true }]
        ));
        assert!(matches!(m.toggle_pins().as_slice(), [Cmd::LoadPins(1)]));
        m.apply(Event::Pins {
            room: 1,
            pins: Ok(vec![]),
        });
        let mut pinned = msg(10, 1, "b", "theirs");
        (pinned.pinned, pinned.updated_at) = (true, 50);
        let cmds = m.apply(Event::Changed(Box::new(pinned)));
        assert!(
            matches!(cmds.as_slice(), [Cmd::LoadPins(1)]),
            "the open panel loads again: {cmds:?}"
        );
        assert!(m.jump_to(10) && m.jump == Some(10));
        assert!(!m.jump_to(999));
        m.open_room(3);
        assert!(m.pins.is_none() && !m.can_pin(), "a DM has no pins");
    }

    #[test]
    fn files_send_with_the_message_once_uploaded() {
        let mut m = chat();
        m.state.me.as_mut().unwrap().upload_max = 1 << 20;
        let cmds = m.attach(vec![("/tmp/a.txt".into(), 3)]);
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::Upload {
                key: 1,
                room: 1,
                ..
            }]
        ));
        assert!(m.send("x").is_empty());
        assert_eq!(m.error, UPLOADS_BUSY);
        let att = vesper_protocol::Attachment {
            id: "u1".into(),
            name: "a.txt".into(),
            mime: "application/octet-stream".into(),
            size: 3,
            kind: vesper_protocol::AttachmentKind::File,
        };
        m.apply(Event::Uploaded {
            key: 1,
            attachment: att,
        });
        let cmds = m.send("");
        assert!(
            matches!(cmds.as_slice(), [Cmd::Send { body, attachments, .. }] if body == "a.txt" && attachments == &[(1, "u1".to_string())]),
            "{cmds:?}"
        );
        m.apply(Event::Sent {
            message: Box::new(msg(20, 1, "a", "a.txt")),
            keys: vec![1],
        });
        assert!(m.tray.items.is_empty());
        // The server's upload_max is checked before sending.
        assert!(m.attach(vec![("/tmp/big".into(), 2 << 20)]).is_empty());
        assert_eq!(m.tray.items[0].error, "Files can be up to 1 MB.");
    }
}
