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
use crate::voice::capture::Source;
use crate::voice::{Frame, Setup, ShareSetup, VoiceCmd, VoiceEvent};

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
pub const INSECURE: &str = "This address uses plain http, so your password and messages cross the network unencrypted. Use https unless the server is on this computer or your home network.";
pub const NOT_ONLINE: &str = "Not connected to the server. Wait a moment and try again.";
pub const CALL_ENDED: &str =
    "Your call ended. You may have joined from another device, or a moderator removed you.";
pub const ROOM_GONE: &str = "You no longer have access to that room.";
pub const REJOINING: &str = "Connection lost. You rejoin the call as soon as it is back.";
pub const CALL_DROPPED: &str = "Disconnected from the server.";
/// The web reloads on a new server build; a desktop app cannot, so it says so.
pub const UPDATE_APP: &str =
    "This server runs a newer Vesper. Update the app to use everything it has.";
pub const TOO_NEW: &str =
    "This server runs a newer Vesper that this app cannot talk to. Update the app.";
/// How long a call whose socket dropped waits for it to come back, like the web's `REJOIN_MS`.
pub const REJOIN_FOR: Duration = Duration::from_secs(60);
/// A resume that takes longer than this shows "Reconnecting" in the call bar, as on the web.
pub const RECONNECTING_AFTER: Duration = Duration::from_secs(4);
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
/// The web's text for `share_stopped`. Only a sharer gets it.
pub const SHARE_STOPPED: &str = "A moderator stopped your screen share.";
/// The web's texts for a share the server cut or refused.
pub const SHARE_CUT: &str = "Your screen share sent more than 35 Mbps and was stopped.";
pub const SHARE_DM: &str = "Screen share is only in group rooms.";
/// The server lowered my share to fit the room's budget.
pub fn share_lowered(kbps: u32) -> String {
    format!(
        "Your screen share was lowered to {:.1} Mbps so every share in the room fits.",
        f64::from(kbps) / 1000.
    )
}

/// My own screen share, like the web's `shares.mine`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mine {
    pub fps: u8,
    pub kbps: u32,
    /// Keep text sharp, else motion smooth.
    pub sharp: bool,
    pub watchers: u32,
    /// The screen is open and `share_start` went; before that the picker may still be up.
    pub open: bool,
    /// What is sent: width, height, frame rate and kbps, from the encoder every 2 s.
    pub figures: Option<(u32, u32, f32, f32)>,
    /// The screen or window chosen in the share dialog.
    pub source: Option<Source>,
}

impl Mine {
    /// The engine's settings: it encodes only while someone watches.
    fn setup(&self) -> ShareSetup {
        ShareSetup {
            fps: u32::from(self.fps),
            kbps: self.kbps,
            sharp: self.sharp,
            active: self.watchers > 0,
            source: self.source,
        }
    }

    fn start_msg(&self) -> Cmd {
        Cmd::Signal(ClientMsg::ShareStart {
            fps: self.fps,
            bitrate_kbps: self.kbps,
        })
    }
}

/// A point at a share shows this long (`MARKER_MS`).
pub const MARKER_FOR: Duration = Duration::from_secs(3);

/// Someone pointed at a spot of `share`'s screen: `x` and `y` from 0 to 1, shown until `until`.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    pub share: String,
    pub from: String,
    pub x: f32,
    pub y: f32,
    pub until: Instant,
}

/// Whether the server and the engine have been told to watch one share, like the web's `WatchGate`.
#[derive(Debug, Default)]
struct Gate {
    on: bool,
    /// When `watch off` goes, once the share is no longer wanted.
    off_at: Option<Instant>,
}

/// The server's socket error codes in plain words, as `SIGNAL_MESSAGES` in `web/src/lib/signaling.ts`.
/// How many older pages a search hit's jump loads at most (50 messages each).
pub const SEEK_PAGES: u8 = 40;

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

/// Whether release `theirs` (`1.9.0`) is newer than `ours`. A pre-release tag is ignored; what does not parse is not newer.
pub fn newer(theirs: &str, ours: &str) -> bool {
    let nums = |v: &str| -> Option<Vec<u64>> {
        let core = v.split(['-', '+']).next()?;
        core.split('.').map(|n| n.parse().ok()).collect()
    };
    matches!((nums(theirs), nums(ours)), (Some(t), Some(o)) if t > o)
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
    /// The server's number for this call, from `joined`, for `resume` (0 from an older server: no resume).
    pub session: u64,
    /// The socket dropped mid-call, and since when: the call and its engine go on, and the next socket takes it back.
    pub resuming: Option<Instant>,
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
    /// The share panel is open (`shares.panelOpen` on the web): every share in it not hidden is watched.
    pub panel_open: bool,
    /// Shares hidden one by one (`shares.hidden`).
    pub hidden: HashSet<String>,
    /// The big one in Focus (`shares.focus`); the first shown one when it is not shown.
    pub focus: Option<String>,
    /// Chat beside the screens in Grid and Focus (`shares.chatOpen`).
    pub chat_open: bool,
    /// Points at a share, each shown until its time (`shares.markers`).
    pub markers: Vec<Marker>,
    /// The share view is on screen: the call's room is shown, not Settings. Set by the view.
    pub share_visible: bool,
    gates: HashMap<String, Gate>,
    /// The newest picture of each watched share. The view takes the pixels to draw; the next picture comes after.
    pub frames: HashMap<String, Frame>,
    /// Pictures of watched shares so far.
    pub pictures: u64,
    /// Watched shares that lose packets or frames.
    pub struggling: HashSet<String>,
    /// Each watched share's size, frame rate and kbps received, for its tile's figures line.
    pub seen: HashMap<String, (u32, u32, f32, f32)>,
    /// The share shown in its own window (`shares.popped`): watched while it is out, panel or not.
    pub popped: Option<String>,
    /// Why the watched share went away.
    pub share_notice: String,
    /// My own share, from Start sharing until it ends.
    pub mine: Option<Mine>,
    /// My share before the last `share_start` (`Some(None)`: it started one), to put back if the server refuses it.
    before: Option<Option<Mine>>,
    /// When the server last cut my share: its `rate_limited` error comes after.
    cut_at: Option<Instant>,
    /// The call's last round trip and loss, for the call bar's quality line; none before the first stats.
    pub quality: Option<crate::link::Sample>,
    /// Each other person's connection grade, by user id.
    pub links: HashMap<String, crate::link::Link>,
    /// Your own row's grade, from `quality`.
    pub own_link: crate::link::Link,
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
            session: 0,
            resuming: None,
            error: String::new(),
            filter: String::new(),
            echo_off: String::new(),
            devices: String::new(),
            headphones: false,
            headphones_seen: false,
            panel_open: true,
            hidden: HashSet::new(),
            focus: None,
            chat_open: false,
            markers: vec![],
            share_visible: false,
            gates: HashMap::new(),
            frames: HashMap::new(),
            pictures: 0,
            struggling: HashSet::new(),
            seen: HashMap::new(),
            popped: None,
            share_notice: String::new(),
            mine: None,
            before: None,
            cut_at: None,
            quality: None,
            links: HashMap::new(),
            own_link: crate::link::Link::default(),
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

    /// The pixels of the newest pictures, for the view to draw. Taking them lets the next ones come.
    pub fn take_frames(&mut self) -> HashMap<String, Frame> {
        std::mem::take(&mut self.frames)
    }

    /// `id`'s share has a tile in the open panel, so it is watched while the view is on screen.
    pub fn shown(&self, id: &str) -> bool {
        self.panel_open && !self.hidden.contains(id)
    }

    /// `id`'s share is drawn somewhere: a tile in the open panel, or its own window.
    /// A resume has taken longer than `RECONNECTING_AFTER`: the call bar says "Reconnecting". A shorter gap shows nothing.
    pub fn reconnecting(&self, now: Instant) -> bool {
        self.resuming
            .is_some_and(|t| now.saturating_duration_since(t) >= RECONNECTING_AFTER)
    }

    pub fn wanted(&self, id: &str) -> bool {
        self.shown(id) || self.popped.as_deref() == Some(id)
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
    /// Something covers the shown room (the Settings view): it is not on screen.
    covered: bool,
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
    /// A search hit to scroll to once loaded: room, message id, older pages it may still load.
    seeking: Option<(i64, i64, u8)>,
    /// Where the last saved file went.
    pub saved: Option<PathBuf>,
    /// When the socket tries again, while offline.
    pub retry_at: Option<Instant>,
    /// Incoming DM calls (room, caller), oldest first: the first one shows.
    pub rings: Vec<(i64, String)>,
    /// A person card's request runs (`dm`, `force-mute`, ...), and why the last one failed.
    pub person_busy: &'static str,
    pub person_error: String,
    /// A DM the person card asked for is there: the view opens it.
    pub dm_opened: Option<i64>,
    /// Settings > Voice's input level and mic test.
    pub mic: MicCheck,
    /// A newer release, offered in a bar above the room (`update.rs`).
    pub update: Update,
    /// A `vesper://room` link's room, opened once the rooms are known.
    pub want_room: Option<i64>,
    /// A `vesper://invite` link's code: the sign in screen offers to create an account with it.
    pub invite: Option<vesper_protocol::Secret>,
}

/// Where an update offer is. `Failed` says why and offers it again.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Update {
    #[default]
    None,
    Offered(crate::update::Offer),
    Installing(crate::update::Offer),
    Failed(crate::update::Offer, String),
}

/// `levelHint` of devices.ts over the last 30 levels (3 s).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hint {
    #[default]
    Ok,
    Clipping,
    TooQuiet,
}

/// `MicCheck.svelte`: the input level, metered from the mic test or the call while Settings > Voice is open, and the
/// mic test (hear yourself). In a call both use the call's own capture; outside one the test opens the mic itself.
#[derive(Debug)]
pub struct MicCheck {
    /// Settings > Voice is open. Set by the view.
    pub open: bool,
    pub testing: bool,
    /// The level in dBFS, -100 for silence.
    pub db: f32,
    peaks: std::collections::VecDeque<f32>,
    pub hint: Hint,
    /// Why the test stopped or did not start.
    pub error: String,
}

impl Default for MicCheck {
    fn default() -> Self {
        MicCheck {
            open: false,
            testing: false,
            db: -100.0,
            peaks: Default::default(),
            hint: Hint::Ok,
            error: String::new(),
        }
    }
}

impl MicCheck {
    fn level(&mut self, db: f32) {
        self.db = db;
        self.peaks.push_back(db);
        if self.peaks.len() > 30 {
            self.peaks.pop_front();
        }
        let max = self.peaks.iter().copied().fold(-100f32, f32::max);
        self.hint = if self.peaks.len() < 30 {
            Hint::Ok
        } else if max >= -1.0 {
            Hint::Clipping
        } else if max < -40.0 {
            Hint::TooQuiet
        } else {
            Hint::Ok
        };
    }

    fn quiet(&mut self) {
        (self.db, self.hint) = (-100.0, Hint::Ok);
        self.peaks.clear();
    }
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
            covered: false,
            loading_older: false,
            seeking: None,
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
            retry_at: None,
            rings: vec![],
            person_busy: "",
            person_error: String::new(),
            dm_opened: None,
            mic: MicCheck::default(),
            update: Update::None,
            want_room: None,
            invite: None,
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
            update: std::mem::take(&mut self.update),
            want_room: self.want_room.take(),
            // An invite is for signing up: gone once signed in.
            invite: self.invite.take().filter(|_| screen == Screen::SignIn),
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
                | Event::UpdateOffered(_)
                | Event::UpdateInstalled(_)
                | Event::UpdateFailed(_)
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
            Event::UpdateOffered(o) => {
                if !matches!(self.update, Update::Installing(_)) {
                    self.update = Update::Offered(o);
                }
            }
            // Here only when the new app did not start (the view starts it and quits).
            Event::UpdateInstalled(_) | Event::UpdateFailed(_) => {
                if let Update::Installing(o) = std::mem::take(&mut self.update) {
                    let why = match ev {
                        Event::UpdateFailed(e) => e,
                        _ => "Vesper was updated. Quit and start it again.".into(),
                    };
                    self.update = Update::Failed(o, why);
                }
            }
            Event::SessionEnded => {
                self.reset(Screen::SignIn, SESSION_ENDED);
                return vec![Cmd::Forget];
            }
            Event::Disconnected => {
                (self.online, self.notice) = (false, DISCONNECTED.into());
                return self.end_call(CALL_DROPPED, false);
            }
            Event::Offline { retry_in } => {
                self.online = false;
                if self.notice == UPDATE_APP || self.notice == TOO_NEW {
                    self.notice.clear(); // the next hello says it again
                }
                self.retry_at = Some(Instant::now() + retry_in);
                // The server keeps the call for a while and the media (UDP) runs on its own: the engine goes on, and
                // the next socket takes the call back. Past REJOIN_FOR (checked at each try) it is over.
                let c = &mut self.call;
                if c.phase == Phase::Joined && c.session > 0 {
                    let since = *c.resuming.get_or_insert_with(Instant::now);
                    if since.elapsed() < REJOIN_FOR {
                        return vec![];
                    }
                    return self.end_call(CALL_DROPPED, false);
                }
                // Without a call number the media goes with the socket; the call is joined again once a new one is up.
                let Some(room) = self.call.room else {
                    return vec![];
                };
                let cmds = self.end_call("", false);
                self.call.rejoin = Some((room, Instant::now()));
                return cmds;
            }
            Event::Voice { call, ev } => return self.voice_event(call, ev),
            Event::MicTest(ev) if self.mic.testing && self.call.phase == Phase::Idle => match ev {
                VoiceEvent::Level(db) => self.mic.level(db),
                VoiceEvent::Failed(why) => {
                    self.mic.testing = false;
                    self.mic.error = why;
                    self.mic.quiet();
                }
                _ => {}
            },
            Event::MicTest(_) => {} // a late one, after the test stopped
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
            Event::ChangeFailed(e) => self.error = e,
            Event::EditFailed(e) => {
                if self.editing.is_some() {
                    self.edit_error = e;
                } else {
                    self.error = e;
                }
            }
            Event::Incompatible => self.notice = TOO_NEW.into(),
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
            Event::Image { .. }
            | Event::Full { .. }
            | Event::Avatar { .. }
            | Event::Searched { .. } => {}
            Event::Downloaded { path, open } => {
                if !open {
                    self.saved = Some(path);
                }
            }
            Event::DownloadFailed(e) => self.error = e,
            Event::SendFailed {
                room,
                body,
                error,
                keys,
            } => {
                self.tray.set_sending(&keys, false);
                self.error = error;
                if self.active == Some(room) {
                    self.draft_back = Some(body);
                }
            }
            Event::ReadFailed { room } => {
                self.read_sent.remove(&room);
            }
            Event::Moderated(r) => {
                self.person_busy = "";
                self.person_error = r.err().unwrap_or_default();
            }
            // The pages' own requests: the view takes their answers.
            Event::Http { .. } => {}
            Event::DmOpened(room) => {
                self.person_busy = "";
                if !self.state.dms.iter().any(|d| d.id == room.id) {
                    self.state.apply(&ServerMsg::RoomsChanged {
                        rooms: self
                            .state
                            .rooms
                            .iter()
                            .chain(&self.state.dms)
                            .cloned()
                            .chain([*room.clone()])
                            .collect(),
                    });
                }
                self.dm_opened = Some(room.id);
            }
        }
        let mut cmds = self.seek_step();
        cmds.extend(self.maybe_read());
        cmds
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
            (self.online, self.notice, self.retry_at) = (true, String::new(), None);
            let theirs = self.state.me.as_ref().map(|m| m.server.version.as_str());
            if theirs.is_some_and(|v| newer(v, vesper_protocol::VERSION)) {
                self.notice = UPDATE_APP.into();
            }
            self.typing.clear();
            // A ring_stop may have been lost while the socket was down (the web's ring store does the same).
            self.rings.clear();
            // Like resync(): events may have been missed while the socket was down, so drop the other rooms and
            // fetch the shown one's newest page again, merged into what is held.
            let link = self
                .want_room
                .take()
                .filter(|id| self.state.find_room(*id).is_some());
            let keep = shown
                .or(link)
                .or_else(|| self.state.rooms.first().map(|r| r.id));
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
            if let Some(since) = self.call.resuming {
                let c = &self.call;
                if since.elapsed() < REJOIN_FOR {
                    cmds.push(Cmd::Signal(ClientMsg::Resume {
                        room_id: c.room.expect("a call has a room"),
                        session: c.session,
                        muted: c.muted,
                        deafened: c.deafened,
                    }));
                } else {
                    cmds.extend(self.end_call(CALL_DROPPED, false));
                }
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
        // A share that ended goes from the panel at once (its `watch off` waits for the grace).
        let sharing: HashSet<String> = self.sharers().into_iter().map(|(id, _)| id).collect();
        self.call.frames.retain(|id, _| sharing.contains(id));
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

    /// Hide one share (`hide` in ShareTile.svelte) or show it again: a hidden share is not watched.
    pub fn hide_share(&mut self, user: &str, hide: bool, now: Instant) -> Vec<Cmd> {
        let c = &mut self.call;
        if hide {
            c.hidden.insert(user.into());
            c.frames.remove(user);
        } else {
            c.hidden.remove(user);
        }
        self.sync_watches(now)
    }

    /// Pop a share out into its own window, or back in (`None`): out, it is watched even with the panel closed.
    pub fn pop_out(&mut self, user: Option<String>, now: Instant) -> Vec<Cmd> {
        self.call.popped = user;
        self.sync_watches(now)
    }

    /// Open the share panel, or close it (Hide screens): closed, no share is watched but the one popped out.
    pub fn set_panel_open(&mut self, open: bool, now: Instant) -> Vec<Cmd> {
        let c = &mut self.call;
        c.panel_open = open;
        if !open {
            c.frames.retain(|id, _| c.popped.as_ref() == Some(id));
        }
        self.sync_watches(now)
    }

    /// Point at a spot of `user`'s share, `x` and `y` from 0 to 1 (`pointAt`).
    pub fn point_at(&self, user: &str, x: f32, y: f32) -> Vec<Cmd> {
        let user_id = user.into();
        vec![Cmd::Signal(ClientMsg::Point { user_id, x, y })]
    }

    /// The view tells whether the share view is on screen.
    pub fn set_share_visible(&mut self, visible: bool, now: Instant) -> Vec<Cmd> {
        if self.call.share_visible == visible {
            return vec![];
        }
        self.call.share_visible = visible;
        self.sync_watches(now)
    }

    /// Sends `watch` on or off for each sharer, like the web's `syncWatches`: on as soon as the share has a tile in
    /// the open panel and the view is on screen, or is popped out, off only `WATCH_GRACE` after that stops. The engine is told too, so it decodes only
    /// what is watched. A share that ended keeps its gate, so the off still goes out.
    pub fn sync_watches(&mut self, now: Instant) -> Vec<Cmd> {
        let sharing: HashSet<String> = self.sharers().into_iter().map(|(id, _)| id).collect();
        let c = &mut self.call;
        if c.popped.as_ref().is_some_and(|p| !sharing.contains(p)) {
            c.popped = None; // that share ended: its window closes
        }
        let ids: HashSet<String> = sharing.iter().chain(c.gates.keys()).cloned().collect();
        let mut cmds = vec![];
        for id in ids {
            let popped = c.popped.as_ref() == Some(&id);
            let want = c.phase == Phase::Joined
                && (popped || c.share_visible && c.shown(&id))
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
                c.seen.remove(&id);
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
            ServerMsg::Joined {
                answer, session, ..
            } if c.phase == Phase::Joining => {
                (c.phase, c.session) = (Phase::Joined, *session);
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
            ServerMsg::Resumed { .. } if c.resuming.is_some() => {
                c.resuming = None;
                vec![]
            }
            // The server ended the call meanwhile: end it here and join the room again on this socket.
            ServerMsg::ResumeFailed if c.resuming.is_some() => {
                let room = c.room.expect("a call has a room");
                let mut cmds = self.end_call("", false);
                cmds.extend(self.join_voice(room));
                cmds
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
                c.error = SHARE_STOPPED.into();
                vec![]
            }
            ServerMsg::Watchers { count } => match c.mine.as_mut() {
                Some(m) => {
                    m.watchers = *count;
                    vec![Cmd::Voice(VoiceCmd::Share(Some(m.setup())))]
                }
                None => vec![],
            },
            // The server ended my share: a moderator, or it sent too much (its error follows).
            ServerMsg::PeerShare {
                user_id,
                share: None,
            } if *user_id == self.state.my_id() && c.mine.is_some_and(|m| m.open) => {
                (c.mine, c.before, c.cut_at) = (None, None, Some(Instant::now()));
                vec![Cmd::Voice(VoiceCmd::Share(None))]
            }
            // The server gave my share less than I asked for (the room's budget): send at that rate.
            ServerMsg::PeerShare {
                user_id,
                share: Some(info),
            } if *user_id == self.state.my_id()
                && c.mine.is_some_and(|m| m.open && info.bitrate_kbps < m.kbps) =>
            {
                let m = c.mine.as_mut().expect("checked");
                m.kbps = info.bitrate_kbps;
                c.error = share_lowered(m.kbps);
                vec![Cmd::Voice(VoiceCmd::Share(Some(m.setup())))]
            }
            ServerMsg::Error {
                code: code @ (ErrorCode::ShareCap | ErrorCode::Forbidden),
                ..
            } if c.phase == Phase::Joined && c.mine.is_some() && c.before.is_some() => {
                // The server refused my last share_start: a new share ends, new settings go back.
                c.error = if *code == ErrorCode::Forbidden {
                    SHARE_DM.into()
                } else {
                    signal_text(*code).into()
                };
                let now = c.mine.expect("checked");
                c.mine = c.before.take().flatten().map(|prev| Mine {
                    watchers: now.watchers,
                    figures: now.figures,
                    ..prev
                });
                vec![Cmd::Voice(VoiceCmd::Share(c.mine.map(|m| m.setup())))]
            }
            ServerMsg::Error {
                code: ErrorCode::RateLimited,
                ..
            } if c
                .cut_at
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5)) =>
            {
                c.error = SHARE_CUT.into();
                vec![]
            }
            ServerMsg::Point {
                user_id,
                from,
                x,
                y,
            } if c.phase == Phase::Joined => {
                c.markers.push(Marker {
                    share: user_id.clone(),
                    from: from.clone(),
                    x: *x,
                    y: *y,
                    until: Instant::now() + MARKER_FOR,
                });
                vec![]
            }
            ServerMsg::RingStart { room_id, user_id } => {
                if !self.rings.iter().any(|r| r.0 == *room_id) {
                    self.rings.push((*room_id, user_id.clone()));
                }
                vec![]
            }
            ServerMsg::RingStop { room_id } => {
                self.rings.retain(|r| r.0 != *room_id);
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
            VoiceEvent::Level(db) => {
                self.mic.level(db);
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
            VoiceEvent::ShareSeen {
                user_id,
                width,
                height,
                fps,
                kbps,
            } => {
                if c.gates.contains_key(&user_id) {
                    c.seen.insert(user_id, (width, height, fps, kbps));
                }
                vec![]
            }
            VoiceEvent::Frame(f) => {
                // A late picture of a share no longer drawn is dropped.
                if c.wanted(&f.user_id) {
                    c.pictures += 1;
                    c.frames.insert(f.user_id.clone(), f);
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
            VoiceEvent::Stats { own, links } => {
                c.quality = Some(own);
                c.own_link = c.own_link.step(Some(own), false);
                let voice = &self.state.voice;
                for (user, s) in links {
                    let muted = voice
                        .iter()
                        .any(|p| p.user_id == user && (p.muted || p.force_muted));
                    let l = c.links.entry(user).or_default();
                    *l = l.step(Some(s), muted);
                }
                vec![]
            }
            VoiceEvent::ShareOpen => match c.mine.as_mut().filter(|m| !m.open) {
                Some(m) => {
                    m.open = true;
                    c.before = Some(None);
                    vec![m.start_msg()]
                }
                None => vec![],
            },
            VoiceEvent::ShareFigures {
                width,
                height,
                fps,
                kbps,
            } => {
                if let Some(m) = c.mine.as_mut() {
                    m.figures = Some((width, height, fps, kbps));
                }
                vec![]
            }
            VoiceEvent::ShareEnded(why) => {
                let Some(m) = c.mine.take() else {
                    return vec![];
                };
                c.before = None;
                if !why.is_empty() {
                    c.error = why;
                }
                if m.open {
                    vec![Cmd::Signal(ClientMsg::ShareStop)]
                } else {
                    vec![]
                }
            }
            VoiceEvent::Failed(e) => self.end_call(&e, true),
            _ => vec![],
        }
    }

    /// Share my screen (the share dialog's Start sharing), or change the running share's settings (its Save).
    /// The first opens the screen; `share_start` goes once it is open.
    pub fn share(&mut self, fps: u8, kbps: u32, sharp: bool, source: Option<Source>) -> Vec<Cmd> {
        let c = &mut self.call;
        if c.phase != Phase::Joined {
            return vec![];
        }
        match c.mine {
            None => {
                let m = Mine {
                    fps,
                    kbps,
                    sharp,
                    watchers: 0,
                    open: false,
                    figures: None,
                    source,
                };
                (c.mine, c.error) = (Some(m), String::new());
                vec![Cmd::Voice(VoiceCmd::Share(Some(m.setup())))]
            }
            Some(m) if !m.open => vec![], // the picker is up
            Some(m) => {
                c.before = Some(Some(m));
                let m = Mine {
                    fps,
                    kbps,
                    sharp,
                    ..m
                };
                c.mine = Some(m);
                vec![m.start_msg(), Cmd::Voice(VoiceCmd::Share(Some(m.setup())))]
            }
        }
    }

    /// Stop sharing my screen.
    pub fn stop_share(&mut self) -> Vec<Cmd> {
        let Some(m) = self.call.mine.take() else {
            return vec![];
        };
        self.call.before = None;
        let mut cmds = vec![Cmd::Voice(VoiceCmd::Share(None))];
        if m.open {
            cmds.insert(0, Cmd::Signal(ClientMsg::ShareStop));
        }
        cmds
    }

    /// Ends my call, or a pending rejoin. `leave` tells the server, when it knows of the call.
    fn end_call(&mut self, error: &str, leave: bool) -> Vec<Cmd> {
        let phase = self.call.phase;
        if phase == Phase::Idle && self.call.rejoin.is_none() {
            return vec![];
        }
        self.call.end(error);
        let mut cmds = vec![];
        if self.mic.testing {
            self.mic.testing = false; // the test was the call's: it ends with it, as on the web
        }
        self.mic.quiet();
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
        self.rings.retain(|r| r.0 != room);
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
                gate_db: Some(s.gate_db as f32),
            },
        }));
        if !self.live() {
            cmds.push(self.sending()); // push to talk: silent until the key
        }
        if self.mic.testing {
            self.mic.testing = false; // the test outside the call: the call takes the mic (net ends the test)
            self.mic.quiet();
        }
        if self.mic.open {
            cmds.push(Cmd::Voice(VoiceCmd::Meter(true)));
        }
        cmds
    }

    /// The view shows Settings > Voice, or stopped showing it: the call meters while it shows, and the mic test ends
    /// when it goes (as `MicCheck.svelte` stops its test when it is unmounted).
    pub fn set_mic_open(&mut self, open: bool) -> Vec<Cmd> {
        if self.mic.open == open {
            return vec![];
        }
        self.mic.open = open;
        let mut cmds = vec![];
        if !open && self.mic.testing {
            cmds = self.mic_test();
        }
        self.mic.quiet();
        if self.call.phase != Phase::Idle {
            cmds.push(Cmd::Voice(VoiceCmd::Meter(open)));
        }
        cmds
    }

    /// Mic test (hear yourself) or Stop mic test: in a call, on the call's capture; else on its own.
    pub fn mic_test(&mut self) -> Vec<Cmd> {
        let m = &mut self.mic;
        m.testing = !m.testing;
        m.error.clear();
        if !m.testing {
            m.quiet();
        }
        if self.call.phase != Phase::Idle {
            return vec![Cmd::Voice(VoiceCmd::Hear(self.mic.testing))];
        }
        let s = &self.settings;
        let setup = self.mic.testing.then(|| Setup {
            denoise: s.denoise,
            input: s.input_device.clone(),
            output: s.output_device.clone(),
            gate_db: Some(s.gate_db as f32),
            ..Setup::default()
        });
        vec![Cmd::MicTest(setup)]
    }

    pub fn leave_voice(&mut self) -> Vec<Cmd> {
        self.end_call("", true)
    }

    /// Accept the first ring: leave any call, and join the DM's. The view opens the room.
    pub fn accept_ring(&mut self, room: i64) -> Vec<Cmd> {
        if !self.online {
            self.call.error = NOT_ONLINE.into(); // the current call stays
            return vec![];
        }
        self.rings.retain(|r| r.0 != room);
        let mut cmds = self.leave_voice();
        cmds.extend(self.join_voice(room));
        cmds
    }

    /// Decline a ring: it stops here; the caller stays in the call.
    pub fn decline_ring(&mut self, room: i64) -> Vec<Cmd> {
        self.rings.retain(|r| r.0 != room);
        vec![Cmd::Signal(ClientMsg::DeclineRing { room_id: room })]
    }

    /// A person card's moderator action (`force-mute`, `stop-share`, `kick`, `disconnect`) or `dm`. One at a time.
    pub fn person_act(
        &mut self,
        user_id: &str,
        what: &'static str,
        muted: Option<bool>,
    ) -> Vec<Cmd> {
        if !self.person_busy.is_empty() {
            return vec![];
        }
        (self.person_busy, self.person_error) = (what, String::new());
        vec![if what == "dm" {
            Cmd::OpenDm(user_id.into())
        } else {
            Cmd::Moderate {
                user_id: user_id.into(),
                action: what,
                muted,
            }
        }]
    }

    /// The server's voice_target rule (`canModerate`): a moderator acts on members, the owner on anyone lower.
    pub fn outranks(&self, user_id: &str) -> bool {
        let rank = |r: vesper_protocol::Role| match r {
            vesper_protocol::Role::Member => 0,
            vesper_protocol::Role::Moderator => 1,
            vesper_protocol::Role::Owner => 2,
        };
        let me = self.state.me.as_ref().map(|m| rank(m.user.role));
        let them = self.state.users.get(user_id).map(|u| rank(u.role));
        matches!((me, them), (Some(a), Some(b)) if a > b)
    }

    /// Mute or unmute. Not while a moderator holds a force mute.
    pub fn toggle_mute(&mut self) -> Vec<Cmd> {
        let c = &mut self.call;
        if c.phase == Phase::Idle || c.force_muted {
            return vec![];
        }
        c.muted = !c.muted;
        let mut cmds = vec![self.sending()];
        cmds.extend(self.voice_state_once_joining());
        cmds
    }

    /// The server answers a voice state before `join` with an error that ends the call; `Joined` sends it then.
    fn voice_state_once_joining(&self) -> Option<Cmd> {
        (self.call.phase != Phase::Starting).then(|| self.voice_state())
    }

    /// Deafen silences everyone; it does not mute the microphone, as in the web app.
    pub fn toggle_deafen(&mut self) -> Vec<Cmd> {
        let c = &mut self.call;
        if c.phase == Phase::Idle {
            return vec![];
        }
        c.deafened = !c.deafened;
        let mut cmds = vec![Cmd::Voice(VoiceCmd::Deafened(c.deafened))];
        cmds.extend(self.voice_state_once_joining());
        cmds
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
        if self.screen != Screen::Main || !self.focused || self.covered || !self.at_bottom {
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

    /// "Update and restart" was clicked.
    pub fn install_update(&mut self) -> Vec<Cmd> {
        match std::mem::take(&mut self.update) {
            Update::Offered(o) | Update::Failed(o, _) => {
                self.update = Update::Installing(o.clone());
                vec![Cmd::InstallUpdate(o)]
            }
            u => {
                self.update = u;
                vec![]
            }
        }
    }

    /// Create your account, with the invite of a `vesper://invite` link.
    pub fn sign_up(
        &mut self,
        server: &str,
        username: &str,
        display_name: &str,
        password: &str,
    ) -> Vec<Cmd> {
        let Some(invite) = self.invite.clone().filter(|_| !self.busy) else {
            return vec![];
        };
        if [server, username, display_name]
            .iter()
            .any(|f| f.trim().is_empty())
            || password.is_empty()
        {
            self.error =
                "Fill in the server address, a username, a display name and a password.".into();
            return vec![];
        }
        (self.busy, self.error, self.notice) = (true, String::new(), String::new());
        self.server = server.trim().into();
        vec![Cmd::Signup {
            server: self.server.clone(),
            invite,
            username: username.trim().into(),
            display_name: display_name.trim().into(),
            password: password.into(),
        }]
    }

    /// A `vesper://` link. Returns a room to show now; else it waits for the rooms, or for the sign in screen.
    pub fn open_link(&mut self, link: crate::single::Link) -> Option<i64> {
        use crate::single::Link;
        match link {
            Link::Room(id) if self.screen == Screen::Main && self.state.find_room(id).is_some() => {
                return Some(id);
            }
            Link::Room(id) => self.want_room = Some(id),
            // Signed in already: an invite is for someone new, so the link only shows the window.
            Link::Invite { .. } if self.screen == Screen::Main => {}
            Link::Invite { code, server } => {
                self.invite = Some(code);
                if let Some(s) = server {
                    self.server = s;
                }
                self.error.clear();
            }
        }
        None
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

    pub fn set_covered(&mut self, covered: bool) -> Vec<Cmd> {
        self.covered = covered;
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
        // A message of files has no text: the body stays empty.
        if body.is_empty() && attachments.is_empty() {
            return vec![];
        }
        let body = body.to_string();
        self.error.clear();
        let keys: Vec<u64> = attachments.iter().map(|a| a.0).collect();
        self.tray.set_sending(&keys, true);
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
        if confirmed {
            self.confirm_delete = None; // also when it was deleted elsewhere meanwhile
        }
        if !self.find(id).is_some_and(|m| self.can_delete(m)) {
            return vec![];
        }
        if !confirmed {
            (self.confirm_delete, self.picking) = (Some(id), None);
            return vec![];
        }
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

    /// A search hit: opens its room and scrolls to it, loading older pages until it is there (at most `SEEK_PAGES`;
    /// the web loads the page around it instead, which this timeline cannot hold yet).
    pub fn seek(&mut self, room: i64, id: i64) -> Vec<Cmd> {
        let mut cmds = self.open_room(room);
        self.seeking = Some((room, id, SEEK_PAGES));
        cmds.extend(self.seek_step());
        cmds
    }

    /// The next step of `seek`: scroll there when loaded, else the next older page.
    fn seek_step(&mut self) -> Vec<Cmd> {
        let Some((room, id, left)) = self.seeking else {
            return vec![];
        };
        let Some(log) = self
            .chat
            .rooms
            .get(&room)
            .filter(|_| self.active == Some(room))
        else {
            self.seeking = None;
            return vec![];
        };
        if log.messages.iter().any(|m| m.id == id) {
            (self.jump, self.seeking) = (Some(id), None);
            return vec![];
        }
        let Some(first) = log.messages.first().map(|m| m.id) else {
            return vec![]; // the first page is on its way
        };
        if !log.more_before || left == 0 || first < id {
            self.seeking = None;
            return vec![];
        }
        if self.loading_older {
            return vec![]; // the page in flight comes back here
        }
        (self.seeking, self.loading_older) = (Some((room, id, left - 1)), true);
        vec![Cmd::LoadOlder {
            room,
            before: first,
        }]
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
    fn a_room_link_waits_for_the_rooms_and_an_invite_for_the_sign_in_screen() {
        use crate::single::Link;
        let mut m = Model::default();
        assert_eq!(m.open_link(Link::Room(3)), None);
        let code: vesper_protocol::Secret = "c".repeat(64).into();
        m.open_link(Link::Invite {
            code: code.clone(),
            server: Some("https://v.example".into()),
        });
        m.apply(Event::SignedOut {
            server: Some("https://old.example".into()),
            notice: String::new(),
        });
        assert_eq!(m.invite, Some(code.clone()), "kept on the sign in screen");
        assert!(m.sign_up("https://v.example", "ann", "", "pw").is_empty());
        let cmds = m.sign_up("https://v.example", "ann", "Ann", "test-password-1");
        assert!(matches!(&cmds[..], [Cmd::Signup { invite, .. }] if *invite == code));
        assert!(
            m.sign_up("https://v.example", "ann", "Ann", "test-password-1")
                .is_empty(),
            "busy"
        );
        assert!(
            !format!("{cmds:?}").contains(&*code),
            "the code is never printed"
        );
        m.apply(Event::SignedIn {
            me: Box::new(match crate::state::tests::hello() {
                ServerMsg::Hello { me, .. } => me,
                _ => unreachable!(),
            }),
            server: "https://v.example".into(),
        });
        assert_eq!(m.invite, None, "gone once signed in");
        m.apply(Event::Server(Box::new(crate::state::tests::hello())));
        assert_eq!(m.active, Some(3), "the link's room, not the first");
        // Signed in: a room link opens at once, an invite does nothing.
        assert_eq!(m.open_link(Link::Room(1)), Some(1));
        m.open_link(Link::Invite { code, server: None });
        assert_eq!(m.invite, None);
    }

    #[test]
    fn an_update_installs_only_on_the_click_and_a_failure_offers_it_again() {
        let names = [crate::update::asset_name(false), crate::update::SUMS];
        let assets: Vec<_> = names
            .iter()
            .map(|n| serde_json::json!({"name": n, "browser_download_url": format!("https://x/{n}")}))
            .collect();
        let json = serde_json::json!({"tag_name": "v99.0.0", "assets": assets}).to_string();
        let o = crate::update::offer(&json, "2.2.0", false).unwrap();
        let mut m = signed_in();
        assert!(m.install_update().is_empty(), "nothing offered");
        assert!(m.apply(Event::UpdateOffered(o.clone())).is_empty());
        assert_eq!(m.update, Update::Offered(o.clone()));
        m.sign_out();
        assert_eq!(m.update, Update::Offered(o.clone()), "kept over a sign out");
        assert!(matches!(
            m.install_update().as_slice(),
            [Cmd::InstallUpdate(_)]
        ));
        assert!(m.install_update().is_empty(), "once");
        m.apply(Event::UpdateOffered(o.clone()));
        assert_eq!(
            m.update,
            Update::Installing(o.clone()),
            "a check does not stop it"
        );
        m.apply(Event::UpdateFailed("The download failed.".into()));
        assert_eq!(m.update, Update::Failed(o, "The download failed.".into()));
        assert!(matches!(
            m.install_update().as_slice(),
            [Cmd::InstallUpdate(_)]
        ));
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

    /// The mic test runs on its own outside a call and on the call's capture in one; the meter follows Settings >
    /// Voice; a call ends the test that was its own.
    #[test]
    fn the_mic_test_and_meter() {
        let mut m = signed_in();
        server(&mut m, hello());
        assert!(m.set_mic_open(true).is_empty(), "no call: nothing to meter");
        let cmds = m.mic_test();
        assert!(
            matches!(cmds.as_slice(), [Cmd::MicTest(Some(Setup { gate_db: Some(g), .. }))] if *g == -50.0),
            "{cmds:?}"
        );
        for _ in 0..30 {
            m.apply(Event::MicTest(VoiceEvent::Level(-60.0)));
        }
        assert_eq!((m.mic.db, m.mic.hint), (-60.0, Hint::TooQuiet));
        m.apply(Event::MicTest(VoiceEvent::Level(-0.5)));
        assert_eq!(m.mic.hint, Hint::Clipping);
        m.apply(Event::MicTest(VoiceEvent::Failed(
            "No microphone was found.".into(),
        )));
        assert!(!m.mic.testing);
        assert_eq!(
            (m.mic.db, m.mic.error.as_str()),
            (-100.0, "No microphone was found.")
        );

        assert!(matches!(m.mic_test().as_slice(), [Cmd::MicTest(Some(_))]));
        let cmds = m.join_voice(1);
        assert!(!m.mic.testing, "the call takes the mic");
        assert!(
            matches!(cmds.last(), Some(Cmd::Voice(VoiceCmd::Meter(true)))),
            "{cmds:?}"
        );
        assert!(matches!(
            m.mic_test().as_slice(),
            [Cmd::Voice(VoiceCmd::Hear(true))]
        ));
        voice(&mut m, VoiceEvent::Level(-20.0));
        assert_eq!(m.mic.db, -20.0);
        let cmds = m.set_mic_open(false);
        assert!(
            matches!(
                cmds.as_slice(),
                [
                    Cmd::Voice(VoiceCmd::Hear(false)),
                    Cmd::Voice(VoiceCmd::Meter(false))
                ]
            ),
            "leaving Settings > Voice stops the test: {cmds:?}"
        );
        m.set_mic_open(true);
        m.mic_test();
        m.leave_voice();
        assert!(!m.mic.testing, "the call's test ends with it");
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
            session: 0,
        };
        assert!(
            matches!(server(&mut m, joined).as_slice(), [Cmd::Voice(VoiceCmd::Answer(a))] if a == "ans")
        );
        assert_eq!(m.call.phase, Phase::Joined);
        m
    }

    #[test]
    fn stats_grade_the_call_and_each_person_and_end_with_it() {
        use crate::link::{Grade, Sample};
        let mut m = in_call();
        let own = Sample {
            rtt_ms: Some(420.),
            ..Sample::default()
        };
        let bob = Sample {
            loss_pct: Some(0.),
            jitter_ms: Some(4.),
            rtt_ms: None,
            packets: Some(50),
        };
        for _ in 0..2 {
            let links = vec![("b".to_string(), bob)];
            voice(&mut m, VoiceEvent::Stats { own, links });
        }
        assert_eq!(
            crate::link::quality(m.call.quality.as_ref()),
            Some(Grade::Poor)
        );
        assert_eq!(m.call.links["b"].grade, Some(Grade::Good));
        assert_eq!(m.call.own_link.grade, Some(Grade::Poor));
        m.leave_voice();
        assert!(
            m.call.quality.is_none() && m.call.links.is_empty(),
            "a new call measures again"
        );
    }

    #[test]
    fn rings_come_and_go_and_moderation_waits_for_its_answer() {
        let mut m = signed_in();
        server(&mut m, hello());
        let ring = |room| ServerMsg::RingStart {
            room_id: room,
            user_id: "b".into(),
        };
        server(&mut m, ring(3));
        server(&mut m, ring(3));
        assert_eq!(m.rings, [(3, "b".to_string())], "one per room");
        server(&mut m, ServerMsg::RingStop { room_id: 3 });
        assert!(m.rings.is_empty());

        assert_eq!(m.person_act("b", "kick", None).len(), 1);
        assert!(m.person_act("b", "kick", None).is_empty(), "one at a time");
        m.apply(Event::Moderated(Err("Not allowed.".into())));
        assert_eq!(
            (m.person_busy, m.person_error.as_str()),
            ("", "Not allowed.")
        );
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

    /// In a call the server numbered (session 5), with the socket just dropped.
    fn resuming() -> Model {
        let mut m = in_call();
        m.call.session = 5;
        let cmds = m.apply(Event::Offline {
            retry_in: std::time::Duration::from_secs(1),
        });
        assert!(cmds.is_empty(), "the engine goes on: {cmds:?}");
        m
    }

    #[test]
    fn a_dropped_socket_resumes_the_call_quietly() {
        let mut m = resuming();
        // The call goes on as it was: no stop, no error, no rejoin, no banner, and no "Reconnecting" yet.
        assert_eq!((m.call.phase, m.call.room), (Phase::Joined, Some(1)));
        assert!(m.call.error.is_empty() && m.call.rejoin.is_none());
        let at = m.call.resuming.expect("resuming");
        assert!(!m.call.reconnecting(at + Duration::from_secs(1)));
        assert!(m.call.reconnecting(at + RECONNECTING_AFTER));
        // Further tries change nothing.
        assert!(
            m.apply(Event::Offline {
                retry_in: std::time::Duration::from_secs(2)
            })
            .is_empty()
        );
        m.toggle_mute(); // while away: the resume carries it
        let cmds = server(&mut m, hello());
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                Cmd::Signal(ClientMsg::Resume {
                    room_id: 1,
                    session: 5,
                    muted: true,
                    deafened: false
                })
            )),
            "{cmds:?}"
        );
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Voice(_))), "{cmds:?}");
        let cmds = server(&mut m, ServerMsg::Resumed { peers: vec![] });
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Voice(_))), "{cmds:?}");
        assert!(m.call.resuming.is_none());
        assert_eq!(m.call.phase, Phase::Joined);
        // Offers on the new socket reach the same engine.
        let cmds = server(&mut m, ServerMsg::Offer { sdp: "o2".into() });
        assert!(matches!(cmds.as_slice(), [Cmd::Voice(VoiceCmd::Offer(s))] if s == "o2"));
    }

    #[test]
    fn a_failed_resume_ends_the_call_and_joins_again() {
        let mut m = resuming();
        server(&mut m, hello());
        let cmds = server(&mut m, ServerMsg::ResumeFailed);
        assert!(
            cmds.iter().any(|c| matches!(c, Cmd::Voice(VoiceCmd::Stop))),
            "{cmds:?}"
        );
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Voice(VoiceCmd::Start { .. }))),
            "{cmds:?}"
        );
        assert_eq!((m.call.phase, m.call.room), (Phase::Starting, Some(1)));
        assert!(m.call.resuming.is_none());
    }

    #[test]
    fn a_resume_too_late_ends_the_call() {
        let mut m = resuming();
        m.call.resuming = m.call.resuming.map(|t| t - REJOIN_FOR);
        let cmds = server(&mut m, hello());
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Voice(VoiceCmd::Stop))));
        assert!(!cmds.iter().any(|c| matches!(c, Cmd::Signal(_))));
        assert_eq!(m.call.error, CALL_DROPPED);
        // The same at a later try while still offline.
        let mut m = resuming();
        m.call.resuming = m.call.resuming.map(|t| t - REJOIN_FOR);
        let cmds = m.apply(Event::Offline {
            retry_in: std::time::Duration::from_secs(2),
        });
        assert!(cmds.iter().any(|c| matches!(c, Cmd::Voice(VoiceCmd::Stop))));
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
            keys: vec![],
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

    /// Cy joins my call (room 1).
    fn join_cy(m: &mut Model) {
        let peer = serde_json::from_value(
            serde_json::json!({ "user_id": "c", "name": "Cy", "room_id": 1,
            "muted": false, "deafened": false, "force_muted": false, "share": null }),
        )
        .unwrap();
        server(m, ServerMsg::PeerJoined { peer });
    }

    fn sorted(mut v: Vec<(String, bool)>) -> Vec<(String, bool)> {
        v.sort();
        v
    }

    /// A popped out share is watched with the panel closed and the window off screen, like the web's `wantsWatch`,
    /// keeps its pictures and figures, and goes back in when the share ends.
    #[test]
    fn a_popped_out_share_is_watched_anywhere_until_it_ends() {
        let mut m = in_call();
        let t = Instant::now();
        share(&mut m, "b", true);
        m.set_panel_open(false, t);
        assert_eq!(
            watches(&m.pop_out(Some("b".into()), t)),
            [("b".into(), true)]
        );
        voice(&mut m, picture("b"));
        assert!(m.call.frames.contains_key("b"), "its pictures come");
        let seen = VoiceEvent::ShareSeen {
            user_id: "b".into(),
            width: 1920,
            height: 1080,
            fps: 30.,
            kbps: 4000.,
        };
        voice(&mut m, seen.clone());
        assert_eq!(m.call.seen.get("b"), Some(&(1920, 1080, 30., 4000.)));

        // Back in, with the panel closed: off after the grace, and its figures go.
        assert!(m.pop_out(None, t).is_empty());
        assert_eq!(
            watches(&m.sync_watches(t + WATCH_GRACE)),
            [("b".into(), false)]
        );
        assert!(m.call.seen.is_empty());
        voice(&mut m, seen);
        assert!(m.call.seen.is_empty(), "figures of a share not watched");

        // Out again, then the share ends: it is no longer out.
        m.pop_out(Some("b".into()), t);
        share(&mut m, "b", false);
        assert_eq!(m.call.popped, None);
    }

    #[test]
    fn every_shown_share_is_watched_while_the_view_is_on_screen() {
        let mut m = in_call();
        let t = Instant::now();
        assert!(m.sharers().is_empty());
        assert!(
            watches(&share(&mut m, "b", true)).is_empty(),
            "the view is not on screen yet"
        );
        assert_eq!(m.sharers(), [("b".to_string(), "Bob".to_string())]);

        // On screen: watch on, once. A second share is watched as soon as it starts: every tile is.
        assert_eq!(watches(&m.set_share_visible(true, t)), [("b".into(), true)]);
        assert!(m.sync_watches(t).is_empty());
        join_cy(&mut m);
        assert_eq!(watches(&share(&mut m, "c", true)), [("c".into(), true)]);

        // The newest picture of each is kept for the view.
        voice(&mut m, picture("b"));
        voice(&mut m, picture("c"));
        voice(&mut m, picture("b"));
        assert_eq!(m.call.pictures, 3);
        let mut got: Vec<String> = m.call.take_frames().into_keys().collect();
        got.sort();
        assert_eq!(got, ["b", "c"]);
        assert!(m.call.take_frames().is_empty());

        // Hidden: its pictures are dropped, and its off goes after the grace. Shown again: on at once.
        assert!(m.hide_share("c", true, t).is_empty());
        voice(&mut m, picture("c"));
        assert!(m.call.frames.is_empty());
        assert_eq!(
            watches(&m.sync_watches(t + WATCH_GRACE)),
            [("c".into(), false)]
        );
        assert_eq!(watches(&m.hide_share("c", false, t)), [("c".into(), true)]);

        // Off screen: off only after the grace; back within it: nothing sent at all.
        assert!(m.set_share_visible(false, t).is_empty());
        assert_eq!(m.call.watch_deadline(), Some(t + WATCH_GRACE));
        assert!(m.set_share_visible(true, t + WATCH_GRACE / 2).is_empty());
        assert_eq!(m.call.watch_deadline(), None);

        // Hide screens: every share goes off after the grace; Show: all on again.
        assert!(m.set_panel_open(false, t).is_empty());
        assert!(m.sync_watches(t + WATCH_GRACE / 2).is_empty());
        assert_eq!(
            sorted(watches(&m.sync_watches(t + WATCH_GRACE))),
            [("b".into(), false), ("c".into(), false)]
        );
        assert!(m.sync_watches(t + WATCH_GRACE * 2).is_empty(), "sent once");
        assert_eq!(
            sorted(watches(&m.set_panel_open(true, t))),
            [("b".into(), true), ("c".into(), true)]
        );

        // Struggling, as the engine says.
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

    /// My own share, like the web's `startShare`, `onShareMsg` and `shareRefused`.
    #[test]
    fn my_share_starts_once_open_follows_its_watchers_and_ends() {
        // Cmd has no PartialEq: compare what it prints.
        macro_rules! same {
            ($a:expr, $b:expr $(,)?) => {
                assert_eq!(format!("{:?}", $a), format!("{:?}", $b))
            };
        }
        let mut m = in_call();
        let me = m.state.my_id().to_string();
        let setup = |active| ShareSetup {
            fps: 30,
            kbps: 4000,
            sharp: true,
            active,
            source: Some(Source::Window(7)),
        };
        let start = ClientMsg::ShareStart {
            fps: 30,
            bitrate_kbps: 4000,
        };
        // The screen opens first (the picker); only then does share_start go.
        same!(
            m.share(30, 4000, true, Some(Source::Window(7))),
            [Cmd::Voice(VoiceCmd::Share(Some(setup(false))))]
        );
        assert!(m.share(60, 4000, true, None).is_empty(), "the picker is up");
        same!(
            voice(&mut m, VoiceEvent::ShareOpen),
            [Cmd::Signal(start.clone())]
        );
        // Watched: the encoder runs; nobody left: it stops.
        let w = |n| ServerMsg::Watchers { count: n };
        same!(
            server(&mut m, w(2)),
            [Cmd::Voice(VoiceCmd::Share(Some(setup(true))))]
        );
        // The server lowered it to fit the room: sent at that rate, and said.
        let granted = ServerMsg::PeerShare {
            user_id: me.clone(),
            share: Some(vesper_protocol::ShareInfo {
                fps: 30,
                bitrate_kbps: 2500,
            }),
        };
        let lowered = ShareSetup {
            kbps: 2500,
            ..setup(true)
        };
        same!(
            server(&mut m, granted),
            [Cmd::Voice(VoiceCmd::Share(Some(lowered)))]
        );
        assert_eq!(m.call.error, share_lowered(2500));
        assert!(m.call.error.contains("2.5 Mbps"));
        // New settings refused (share_cap): the old ones come back, the share goes on.
        m.share(60, 9000, false, None);
        let refused = ServerMsg::Error {
            code: ErrorCode::ShareCap,
            message: String::new(),
        };
        same!(
            server(&mut m, refused),
            [Cmd::Voice(VoiceCmd::Share(Some(lowered)))]
        );
        assert_eq!(m.call.mine.map(|x| (x.fps, x.kbps)), Some((30, 2500)));
        // Stop: share_stop, then the engine.
        same!(
            m.stop_share(),
            [
                Cmd::Signal(ClientMsg::ShareStop),
                Cmd::Voice(VoiceCmd::Share(None))
            ]
        );
        assert!(m.call.mine.is_none());

        // A cancelled picker ends it quietly; a failed capture says so.
        m.share(30, 4000, true, None);
        assert!(voice(&mut m, VoiceEvent::ShareEnded(String::new())).is_empty());
        assert!(m.call.mine.is_none());
        m.share(30, 4000, true, None);
        voice(&mut m, VoiceEvent::ShareOpen);
        let cmds = voice(
            &mut m,
            VoiceEvent::ShareEnded("The screen capture stopped.".into()),
        );
        same!(cmds, [Cmd::Signal(ClientMsg::ShareStop)]);
        assert_eq!(m.call.error, "The screen capture stopped.");

        // A new share refused in a DM room ends here with the web's text.
        m.share(30, 4000, true, None);
        voice(&mut m, VoiceEvent::ShareOpen);
        let refused = ServerMsg::Error {
            code: ErrorCode::Forbidden,
            message: String::new(),
        };
        same!(server(&mut m, refused), [Cmd::Voice(VoiceCmd::Share(None))]);
        assert_eq!((m.call.mine, m.call.error.as_str()), (None, SHARE_DM));

        // The server cut it: it ends here, and its rate_limited error reads as the cut.
        m.share(30, 4000, true, None);
        voice(&mut m, VoiceEvent::ShareOpen);
        let cut = ServerMsg::PeerShare {
            user_id: me,
            share: None,
        };
        let cmds = server(&mut m, cut);
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Cmd::Voice(VoiceCmd::Share(None))))
        );
        let limited = ServerMsg::Error {
            code: ErrorCode::RateLimited,
            message: String::new(),
        };
        server(&mut m, limited);
        assert_eq!((m.call.mine, m.call.error.as_str()), (None, SHARE_CUT));
    }

    #[test]
    fn a_share_ending_clears_its_picture_and_a_call_ending_forgets_the_watches() {
        let mut m = in_call();
        share(&mut m, "b", true);
        let t = Instant::now();
        assert_eq!(watches(&m.set_share_visible(true, t)), [("b".into(), true)]);
        voice(&mut m, picture("b"));

        // Bob stops: the picture goes at once; the off goes after the grace.
        assert!(watches(&share(&mut m, "b", false)).is_empty());
        assert!(m.call.frames.is_empty());
        assert!(m.call.watch_deadline().is_some());
        let later = Instant::now() + WATCH_GRACE;
        assert_eq!(watches(&m.sync_watches(later)), [("b".into(), false)]);

        // Watching again, one hidden, then the call ends: the server forgets watches with the session, so nothing is
        // sent, and the panel's state goes with the call.
        share(&mut m, "b", true);
        join_cy(&mut m);
        share(&mut m, "c", true);
        m.hide_share("c", true, t);
        m.call.chat_open = true;
        let cmds = m.leave_voice();
        assert!(watches(&cmds).is_empty());
        let c = &m.call;
        assert!(
            c.hidden.is_empty() && c.panel_open && !c.chat_open && c.watch_deadline().is_none()
        );
        assert!(m.call.share_visible, "the view's state stays");

        // share_stopped goes only to a sharer; its text is the web's.
        let mut m = in_call();
        server(&mut m, ServerMsg::ShareStopped);
        assert_eq!(m.call.share_notice, SHARE_STOPPED);

        // A point goes to the server; one at a share shows for `MARKER_FOR`.
        let cmds = m.point_at("b", 0.5, 0.25);
        assert!(
            matches!(cmds.as_slice(), [Cmd::Signal(ClientMsg::Point { user_id, x, y })] if user_id == "b" && *x == 0.5 && *y == 0.25),
            "{cmds:?}"
        );
        let before = Instant::now();
        let (user_id, from) = ("b".to_string(), "c".to_string());
        server(
            &mut m,
            ServerMsg::Point {
                user_id,
                from,
                x: 0.1,
                y: 0.9,
            },
        );
        let mk = &m.call.markers[0];
        assert_eq!(
            (mk.share.as_str(), mk.from.as_str(), mk.x, mk.y),
            ("b", "c", 0.1, 0.9)
        );
        assert!(mk.until >= before + MARKER_FOR);
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
        m.apply(Event::ChangeFailed("A reaction failed.".into()));
        assert_eq!(
            (m.edit_error.as_str(), m.error.as_str()),
            ("", "A reaction failed."),
            "a failed reaction is not the edit's error"
        );
        m.apply(Event::EditFailed(
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
            matches!(cmds.as_slice(), [Cmd::Send { body, attachments, .. }] if body.is_empty() && attachments == &[(1, "u1".to_string())]),
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

    #[test]
    fn a_newer_server_is_named_and_an_unreadable_hello_too() {
        assert!(newer("1.9.0", "1.8.3") && newer("2.0.0", "1.8.3"));
        assert!(!newer("1.8.3", "1.8.3") && !newer("1.8.2", "1.8.3"));
        assert!(!newer("1.8.4-rc1", "1.8.4") && !newer("dev", "1.8.3"));

        let mut m = signed_in();
        server(&mut m, hello());
        assert!(m.notice.is_empty(), "the test server is older");
        let mut hi = hello();
        if let ServerMsg::Hello { me, .. } = &mut hi {
            me.server.version = "99.0.0".into();
        }
        server(&mut m, hi);
        assert_eq!((m.online, m.notice.as_str()), (true, UPDATE_APP));
        m.apply(Event::Offline {
            retry_in: Duration::from_secs(1),
        });
        assert!(
            m.notice.is_empty(),
            "the banner says the connection is lost"
        );
        m.apply(Event::Incompatible);
        assert_eq!(m.notice, TOO_NEW);
    }

    #[test]
    fn a_reconnect_and_a_join_drop_old_rings() {
        let mut m = signed_in();
        server(&mut m, hello());
        let ring = |room| ServerMsg::RingStart {
            room_id: room,
            user_id: "b".into(),
        };
        server(&mut m, ring(3));
        server(&mut m, hello());
        assert!(m.rings.is_empty(), "its ring_stop may have been lost");
        server(&mut m, ring(3));
        server(&mut m, ring(4));
        m.join_voice(3);
        assert_eq!(m.rings, [(4, "b".to_string())]);
    }

    #[test]
    fn accepting_a_ring_offline_keeps_the_call() {
        let mut m = in_call();
        server(
            &mut m,
            ServerMsg::RingStart {
                room_id: 3,
                user_id: "b".into(),
            },
        );
        m.online = false;
        assert!(m.accept_ring(3).is_empty());
        assert_eq!((m.call.phase, m.call.room), (Phase::Joined, Some(1)));
        assert_eq!(m.call.error, NOT_ONLINE);
    }

    #[test]
    fn mute_before_the_join_waits_for_joined() {
        let mut m = signed_in();
        server(&mut m, hello());
        m.join_voice(1);
        assert_eq!(m.call.phase, Phase::Starting);
        let sent = |c: &[Cmd]| {
            c.iter()
                .any(|c| matches!(c, Cmd::Signal(ClientMsg::SetVoiceState { .. })))
        };
        assert!(
            !sent(&m.toggle_mute()),
            "the server would answer join first"
        );
        assert!(!sent(&m.toggle_deafen()));
        voice(&mut m, VoiceEvent::Offer("o".into()));
        let joined = ServerMsg::Joined {
            user_id: "a".into(),
            answer: "ans".into(),
            peers: vec![],
            session: 0,
        };
        assert!(sent(&server(&mut m, joined)), "Joined sends it");
        assert!(sent(&m.toggle_mute()));
    }

    #[test]
    fn a_delete_confirm_closes_when_the_message_went_elsewhere() {
        let mut m = chat();
        m.delete(11, false);
        assert_eq!(m.confirm_delete, Some(11));
        let mut gone = msg(11, 1, "a", "");
        gone.deleted = true;
        server(&mut m, ServerMsg::MessageUpdated { message: gone });
        assert!(m.delete(11, true).is_empty());
        assert_eq!(m.confirm_delete, None);
    }

    #[test]
    fn a_second_send_does_not_attach_the_same_files() {
        let mut m = chat();
        m.state.me.as_mut().unwrap().upload_max = 1 << 20;
        m.attach(vec![("/tmp/a.txt".into(), 3)]);
        let att = vesper_protocol::Attachment {
            id: "u1".into(),
            name: "a.txt".into(),
            mime: "text/plain".into(),
            size: 3,
            kind: vesper_protocol::AttachmentKind::File,
        };
        m.apply(Event::Uploaded {
            key: 1,
            attachment: att,
        });
        assert!(
            matches!(m.send("one").as_slice(), [Cmd::Send { attachments, .. }] if attachments.len() == 1)
        );
        assert!(
            matches!(m.send("two").as_slice(), [Cmd::Send { attachments, .. }] if attachments.is_empty()),
            "the files are in the first send"
        );
        assert!(m.send("").is_empty(), "no files left to send alone");
        m.apply(Event::SendFailed {
            room: 1,
            body: "one".into(),
            error: "x".into(),
            keys: vec![1],
        });
        assert!(
            matches!(m.send("again").as_slice(), [Cmd::Send { attachments, .. }] if attachments.len() == 1),
            "a failed send gives them back"
        );
    }
}
