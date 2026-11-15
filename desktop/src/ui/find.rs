//! The layers over the shell: the Ctrl+K switcher (`Switcher.svelte`, `SwitcherField.svelte`), message search
//! (`SearchDialog.svelte`, `search.svelte.ts`) and the shortcuts list (`Shortcuts.svelte`, `ShortcutList.svelte`), plus
//! the app's keyboard shortcuts (`keys.ts`, read in `Shell.svelte`).
//!
//! The switcher and search are the web's `bare` dialog: near the top (96 px), 600 px wide (search 720), at most 520
//! px tall (search 640), over the backdrop, fading in with `pop` (GPUI cannot scale a box). Focus stays in the one
//! text field; Up, Down, Enter and the rest are read there.

use std::time::{Duration, Instant};

use gpui::Focusable;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, Context, Div, ElementId, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Task, Window,
    deferred, div, px, transparent_black,
};
use gpui_component::input::{Input, RopeExt};
use vesper_protocol::{SearchHit, SearchQuery};

use super::c::{self, Avatar, Badge, Ellipsis};
use super::icons::icon;
use super::motion::{self, POP};
use super::settings::{self, At};
use super::theme::{TEXT_LG, TEXT_SM, TEXT_XS, TextStepExt, Tokens, tokens};
use super::{Vesper, admin};
use crate::model::Phase;
use crate::net::{Cmd, Fail};
use crate::searchq::{self, Key, MAX_QUERY};
use crate::switcher::{self, Item, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Switcher,
    Search,
    Shortcuts,
}

/// The key context of the switcher's and search's field.
pub const FIND: &str = "Find";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Idle,
    Loading,
    Done,
    Busy,
    Error,
}

/// `Searcher` of search.svelte.ts: waits 300 ms after the last change, drops answers to an older query, on a 429
/// tries the same page once more a second later, and stops at `Busy` after a second 429 until Enter or Try again.
#[derive(Default)]
pub struct Searcher {
    pub hits: Vec<SearchHit>,
    pub next: Option<i64>,
    pub status: Status,
    pub error: String,
    pub paging: bool,
    q: Option<SearchQuery>,
    seq: u64,
    before: Option<i64>,
    again: bool,
    timer: Option<Task<()>>,
}

/// What is open, and its state.
#[derive(Default)]
pub struct Find {
    pub layer: Option<Layer>,
    since: Option<Instant>,
    /// The highlighted row of the switcher or of the search suggestions.
    active: usize,
    /// The switcher's `r `, `p ` or `> ` chip.
    scope: Option<Kind>,
    /// Rooms opened last, most recent first (in memory: the web keeps them in localStorage).
    pub recent: Vec<i64>,
    pub search: Searcher,
    /// The search result the keyboard is on (the web moves real focus into the results).
    hit: Option<usize>,
    hovered: Option<usize>,
    pub jump_error: String,
    scroll: ScrollHandle,
}

#[cfg(test)]
impl Find {
    pub fn scope_kind(&self) -> Option<Kind> {
        self.scope
    }
}

const BUSY: &str = "Search is busy. Wait a moment, then press Enter to try again.";

/// One row of `SHORTCUTS` in keys.ts: what it does, its keys and whether it works while typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Switcher,
    Mute,
    Deafen,
    Search,
    PrevRoom,
    NextRoom,
    NextUnread,
    Sidebar,
    Help,
    Escape,
}

struct Def {
    s: Shortcut,
    /// Ctrl, or Cmd on a Mac.
    cmd: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
    key: &'static str,
    typing: bool,
    label: &'static str,
}

const fn has(m: &[u8], c: u8) -> bool {
    let mut i = 0;
    while i < m.len() {
        if m[i] == c {
            return true;
        }
        i += 1;
    }
    false
}

const fn def(s: Shortcut, mods: &str, key: &'static str, typing: bool, label: &'static str) -> Def {
    let m = mods.as_bytes();
    Def {
        s,
        cmd: has(m, b'm'),
        ctrl: has(m, b'c'),
        alt: has(m, b'a'),
        shift: has(m, b's'),
        key,
        typing,
        label,
    }
}

/// `SHORTCUTS` of keys.ts. Mods: m Ctrl (Cmd on a Mac), c Ctrl, a Alt, s Shift.
const SHORTCUTS: [Def; 12] = [
    def(
        Shortcut::Switcher,
        "m",
        "k",
        true,
        "Switch to a room, a person or an action",
    ),
    def(Shortcut::Mute, "ms", "m", true, "Mute or unmute"),
    def(Shortcut::Deafen, "ms", "d", true, "Deafen or undeafen"),
    def(Shortcut::Search, "ms", "f", true, "Search messages"),
    def(Shortcut::PrevRoom, "a", "up", false, "Previous room"),
    def(Shortcut::NextRoom, "a", "down", false, "Next room"),
    def(
        Shortcut::PrevRoom,
        "ca",
        "up",
        true,
        "Previous room, also while typing",
    ),
    def(
        Shortcut::NextRoom,
        "ca",
        "down",
        true,
        "Next room, also while typing",
    ),
    def(
        Shortcut::NextUnread,
        "as",
        "down",
        false,
        "Next unread room",
    ),
    def(
        Shortcut::Sidebar,
        "c",
        "\\",
        true,
        "Hide or show the sidebar",
    ),
    def(
        Shortcut::Help,
        "s",
        "?",
        false,
        "Show the keyboard shortcuts",
    ),
    def(
        Shortcut::Escape,
        "",
        "escape",
        false,
        "Close a dialog or a menu",
    ),
];

const MAC: bool = cfg!(target_os = "macos");

/// The shortcut a key press is, or None (`shortcut()` of keys.ts). `?` is matched by the character it types.
pub fn shortcut(k: &gpui::Keystroke, typing: bool) -> Option<Shortcut> {
    let m = &k.modifiers;
    let key = if k.key_char.as_deref() == Some("?") {
        "?"
    } else {
        k.key.as_str()
    };
    SHORTCUTS.iter().find_map(|d| {
        let ctrl = d.ctrl || (d.cmd && !MAC);
        let meta = d.cmd && MAC;
        let hit = d.key == key
            && m.control == ctrl
            && m.platform == meta
            && m.shift == d.shift
            && m.alt == d.alt;
        (hit && (!typing || d.typing)).then_some(d.s)
    })
}

/// The key caps of a shortcut row (`keysFor`).
fn keys_for(d: &Def) -> Vec<&'static str> {
    let k = match d.key {
        "up" => "↑",
        "down" => "↓",
        "escape" => "Esc",
        "k" => "K",
        "m" => "M",
        "d" => "D",
        "f" => "F",
        k => k,
    };
    let mut v = vec![];
    if d.ctrl {
        v.push("Ctrl");
    }
    if d.cmd {
        v.push(if MAC { "⌘" } else { "Ctrl" });
    }
    if d.alt {
        v.push(if MAC { "⌥" } else { "Alt" });
    }
    if d.shift && d.key != "?" {
        v.push(if MAC { "⇧" } else { "Shift" });
    }
    v.push(k);
    v
}

fn keys_of(s: Shortcut) -> Vec<&'static str> {
    SHORTCUTS.iter().find(|d| d.s == s).map_or(vec![], keys_for)
}

/// `Kbd.svelte`: a 20 px key cap, at least 20 wide, 4 px sides, `--radius-sm`, a `--line` border with a
/// `--line-strong` bottom, `--bg-element`, 12/16 `--fg-2`.
pub fn kbd(text: impl Into<SharedString>, t: &Tokens) -> Div {
    div()
        .relative()
        .flex()
        .flex_none()
        .h(px(20.))
        .min_w(px(20.))
        .items_center()
        .justify_center()
        .px(px(4.))
        .rounded(t.radius_sm)
        .border_1()
        .border_color(t.line)
        .bg(t.bg_element)
        .text_step(TEXT_XS)
        .text_color(t.fg_2)
        .font_family(t.font_sans.clone())
        .child(text.into())
        .child(
            div()
                .absolute()
                .left(px(1.))
                .right(px(1.))
                .bottom(px(-1.))
                .h(px(1.))
                .bg(t.line_strong),
        )
}

/// Action rows: an icon by the first part of their id.
fn action_icon(id: &str) -> &'static str {
    match id.split('-').next().unwrap_or("") {
        "join" => "headphones",
        "mute" => "micOff",
        "deafen" => "headphonesOff",
        "leave" => "phoneOff",
        "search" => "search",
        "next" => "arrowDown",
        "settings" => "settings",
        "admin" => "shield",
        "sidebar" => "sidebar",
        "shortcuts" => "command",
        "logout" => "logOut",
        _ => "chevronRight",
    }
}

fn row_hover(i: usize) -> ElementId {
    ElementId::Name(format!("find-row-{i}").into())
}

impl Vesper {
    /// Opens a layer (the same one again closes the switcher, as Ctrl+K toggles it). Each open starts empty.
    pub(super) fn open_layer(&mut self, l: Layer, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.layer == Some(l) && l == Layer::Switcher {
            return self.close_layer(window, cx);
        }
        let f = &mut self.find;
        (f.layer, f.since, f.active, f.scope, f.hit, f.hovered) =
            (Some(l), Some(Instant::now()), 0, None, None, None);
        f.jump_error.clear();
        self.person = None;
        let placeholder = match l {
            Layer::Search => "Search messages. Try from:alice in:general",
            _ => "Rooms, people and actions",
        };
        self.find_field.update(cx, |i, cx| {
            i.set_value("", window, cx);
            i.set_placeholder(placeholder, window, cx);
            i.focus(window, cx);
        });
        if l == Layer::Shortcuts {
            self.dialog_focus.focus(window);
        }
        self.search_changed(cx);
        cx.notify();
    }

    pub(super) fn close_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find.layer = None;
        self.search_changed(cx);
        self.find_field
            .update(cx, |i, cx| i.set_value("", window, cx));
        self.composer.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    /// The field's text changed: the switcher's chip, the search query.
    pub(super) fn find_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let q = self.find_field.read(cx).value().to_string();
        if self.find.layer == Some(Layer::Switcher)
            && self.find.scope.is_none()
            && let Some(k) = switcher::scope_of(&q)
        {
            self.find.scope = Some(k);
            let rest = q[2..].to_string();
            self.find_field
                .update(cx, |i, cx| i.set_value(rest, window, cx));
        }
        if self.find.layer == Some(Layer::Search) && q.chars().count() > MAX_QUERY {
            let cut: String = q.chars().take(MAX_QUERY).collect();
            self.find_field
                .update(cx, |i, cx| i.set_value(cut, window, cx));
        }
        (self.find.active, self.find.hit) = (0, None);
        self.search_changed(cx);
        cx.notify();
    }

    fn find_text(&self, cx: &App) -> String {
        self.find_field.read(cx).value().to_string()
    }

    fn parsed(&self, q: &str) -> searchq::Parsed {
        let s = &self.model.state;
        let today = chrono::Local::now().date_naive();
        searchq::parse_search(q, s.users.values(), &s.rooms, today, &chrono::Local)
    }

    /// The query the server gets now, or None (closed, a hint shows, one short word).
    fn search_ask(&self, cx: &App) -> Option<SearchQuery> {
        if self.find.layer != Some(Layer::Search) {
            return None;
        }
        let p = self.parsed(&self.find_text(cx));
        let words = p.q.trim();
        let one_short = !words.contains(' ') && searchq::short_last(words);
        (p.unknown.is_empty() && !searchq::nothing_to_search(words) && !one_short).then(|| {
            SearchQuery {
                q: words.to_string(),
                room_id: p.room_id,
                author_id: p.author_id,
                from: p.from,
                to: p.to,
                before: None,
            }
        })
    }

    /// `Searcher.set`: a new query (or none) clears or restarts the search; the same one keeps what is shown.
    fn search_changed(&mut self, cx: &mut Context<Self>) {
        let q = self.search_ask(cx);
        let s = &mut self.find.search;
        if q == s.q {
            return;
        }
        (s.timer, s.q, s.paging) = (None, q.clone(), false);
        s.seq += 1;
        s.error.clear();
        if q.is_none() {
            (s.hits, s.next, s.status) = (vec![], None, Status::Idle);
            return;
        }
        s.status = Status::Loading; // the old hits stay until the answer, so the list does not blink
        self.search_later(Duration::from_millis(300), None, false, cx);
    }

    fn search_later(
        &mut self,
        wait: Duration,
        before: Option<i64>,
        again: bool,
        cx: &mut Context<Self>,
    ) {
        self.find.search.timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |v, _| v.search_fetch(before, again));
        }));
    }

    fn search_fetch(&mut self, before: Option<i64>, again: bool) {
        let s = &mut self.find.search;
        let Some(mut q) = s.q.clone() else { return };
        s.seq += 1;
        (s.before, s.again, q.before) = (before, again, before);
        let seq = s.seq;
        self.run(vec![Cmd::Search { seq, q }]);
    }

    fn search_more(&mut self) {
        let s = &mut self.find.search;
        if s.next.is_none() || s.paging || s.q.is_none() {
            return;
        }
        s.paging = true;
        let next = s.next;
        self.search_fetch(next, false);
    }

    fn search_retry(&mut self) {
        let s = &mut self.find.search;
        if s.status != Status::Busy || s.q.is_none() {
            return;
        }
        s.paging = s.before.is_some();
        s.status = if s.paging {
            Status::Done
        } else {
            Status::Loading
        };
        let before = s.before;
        self.search_fetch(before, false);
    }

    /// An answer from `GET /api/search`; one for an older query is dropped.
    pub(super) fn searched(
        &mut self,
        seq: u64,
        page: Result<vesper_protocol::SearchPage, Fail>,
        cx: &mut Context<Self>,
    ) {
        let s = &mut self.find.search;
        if seq != s.seq {
            return;
        }
        match page {
            Ok(p) => {
                if s.before.is_none() {
                    s.hits = p.hits;
                } else {
                    s.hits.extend(p.hits);
                }
                (s.next, s.status, s.paging) = (p.next_before, Status::Done, false);
            }
            Err(Fail::Api { status: 429, .. }) if s.again => {
                (s.status, s.paging) = (Status::Busy, false); // never a loop: the person asks again
            }
            Err(Fail::Api { status: 429, .. }) => {
                let before = s.before;
                self.search_later(Duration::from_secs(1), before, true, cx);
            }
            Err(f) => (s.status, s.error, s.paging) = (Status::Error, f.text().into(), false),
        }
    }

    /// Jumps to a hit: its room opens and the timeline scrolls to it.
    fn go_to_hit(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(h) = self.find.search.hits.get(i) else {
            return;
        };
        let (room, id) = (h.message.room_id, h.message.id);
        self.close_layer(window, cx);
        if self.model.active != Some(room) || self.settings_open {
            self.open_room(room, window, cx);
        }
        let cmds = self.model.seek(room, id);
        self.run(cmds);
        cx.notify();
    }

    /// The switcher's items, in the web's order before ranking.
    fn switcher_items(&self) -> Vec<Item> {
        let m = &self.model;
        let s = &m.state;
        let me = s.my_id();
        let room_name = |id: i64| s.find_room(id).map_or("a room".into(), |r| r.name.clone());
        let joined = m.call.phase == Phase::Joined;
        let where_ = |uid: &str| {
            let r = if uid == me {
                m.call.room.filter(|_| joined)
            } else {
                s.voice.iter().find(|p| p.user_id == uid).map(|p| p.room_id)
            };
            r.map(|r| format!("In voice in {}", room_name(r)))
        };
        let act = |id: &str, label: String, hint: Option<&str>| Item {
            kind: Kind::Action,
            id: id.into(),
            label,
            hint: hint.map(String::from),
        };
        let mut rooms: Vec<_> = s.rooms.iter().collect();
        rooms.sort_by_key(|r| r.position);
        let mut people: Vec<_> = s
            .users
            .values()
            .filter(|u| !u.banned && u.user_id != me)
            .map(|u| (where_(&u.user_id), u))
            .collect();
        people.sort_by(|a, b| {
            a.0.is_none().cmp(&b.0.is_none()).then_with(|| {
                a.1.display_name
                    .to_lowercase()
                    .cmp(&b.1.display_name.to_lowercase())
            })
        });
        let current = m.active.filter(|_| !self.settings_open);
        let room = current.and_then(|id| rooms.iter().find(|r| r.id == id));
        let mut items: Vec<Item> = rooms
            .iter()
            .map(|r| Item {
                kind: Kind::Room,
                id: r.id.to_string(),
                label: r.name.clone(),
                hint: None,
            })
            .collect();
        items.extend(people.into_iter().map(|(w, u)| Item {
            kind: Kind::Person,
            id: u.user_id.clone(),
            label: u.display_name.clone(),
            hint: w,
        }));
        if let Some(r) = room
            && !(joined && m.call.room == Some(r.id))
        {
            items.push(act("join", format!("Join voice in {}", r.name), None));
        }
        if joined {
            let c = &m.call;
            items.push(act(
                "mute",
                (if c.muted { "Unmute" } else { "Mute" }).into(),
                Some("mute"),
            ));
            items.push(act(
                "deafen",
                (if c.deafened { "Undeafen" } else { "Deafen" }).into(),
                Some("deafen"),
            ));
            items.push(act("leave", "Leave voice".into(), None));
        }
        items.push(act("search", "Search messages".into(), Some("search")));
        if let Some(n) = switcher::next_unread(&self.room_order(), current, &s.unread) {
            items.push(act(
                &format!("next-unread-{n}"),
                format!("Next unread room: {}", room_name(n)),
                Some("next-unread"),
            ));
        }
        // The web's list less Diagnostics, which the desktop does not have.
        for sec in settings::Section::ALL {
            items.push(act(
                &format!("settings-{}", sec.id()),
                format!("Settings: {}", sec.name()),
                None,
            ));
        }
        // Staff only; a moderator without the owner's sections (`ADMIN_OWNER_ONLY`).
        for sec in admin::sections_for(self.my_role()) {
            items.push(act(
                &format!("admin-{}", sec.id()),
                format!("Admin: {}", sec.name()),
                None,
            ));
        }
        let collapsed = m.settings.sidebar_collapsed;
        items.push(act(
            "sidebar",
            (if collapsed {
                "Show sidebar"
            } else {
                "Hide sidebar"
            })
            .into(),
            Some("sidebar"),
        ));
        items.push(act("shortcuts", "Keyboard shortcuts".into(), None));
        items.push(act("logout", "Log out".into(), None));
        items
    }

    /// The room ids in sidebar order: rooms by position, then DMs (`roomOrder`).
    pub(super) fn room_order(&self) -> Vec<i64> {
        let s = &self.model.state;
        let mut rooms: Vec<_> = s.rooms.iter().collect();
        rooms.sort_by_key(|r| r.position);
        rooms
            .iter()
            .chain(s.dms.iter().collect::<Vec<_>>().iter())
            .map(|r| r.id)
            .collect()
    }

    pub(super) fn switcher_results(&self, cx: &App) -> Vec<Item> {
        let prefix = match self.find.scope {
            Some(Kind::Room) => "r ",
            Some(Kind::Person) => "p ",
            Some(Kind::Action) => "> ",
            None => "",
        };
        let current = self.model.active;
        let recent: Vec<i64> = self
            .find
            .recent
            .iter()
            .copied()
            .filter(|&r| Some(r) != current)
            .collect();
        let q = format!("{prefix}{}", self.find_text(cx));
        switcher::rank(
            &q,
            &self.switcher_items(),
            &self.model.state.unread,
            &recent,
        )
    }

    /// Runs a switcher row, after the dialog closed.
    fn choose(&mut self, it: Item, window: &mut Window, cx: &mut Context<Self>) {
        self.close_layer(window, cx);
        match it.kind {
            Kind::Room => {
                if let Ok(id) = it.id.parse() {
                    self.open_room(id, window, cx);
                }
            }
            Kind::Person => self.open_person(it.id, String::new(), false, cx),
            Kind::Action => self.run_action(&it.id, window, cx),
        }
    }

    fn run_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.model.active;
        match id {
            "join" => {
                if let Some(room) = current {
                    self.act(
                        |m| {
                            let mut cmds = if m.call.phase != Phase::Idle {
                                m.leave_voice()
                            } else {
                                vec![]
                            };
                            cmds.extend(m.join_voice(room));
                            cmds
                        },
                        cx,
                    );
                }
            }
            "mute" => self.act(|m| m.toggle_mute(), cx),
            "deafen" => self.act(|m| m.toggle_deafen(), cx),
            "leave" => self.act(|m| m.leave_voice(), cx),
            "search" => self.open_layer(Layer::Search, window, cx),
            "sidebar" => {
                let on = !self.model.settings.sidebar_collapsed;
                self.set_collapsed(on, cx);
            }
            "shortcuts" => self.open_layer(Layer::Shortcuts, window, cx),
            "logout" => self.sign_out(window, cx),
            id => {
                if let Some(room) = id.strip_prefix("next-unread-").and_then(|n| n.parse().ok()) {
                    self.open_room(room, window, cx);
                } else if let Some(sec) = id
                    .strip_prefix("settings-")
                    .and_then(settings::Section::from_id)
                {
                    self.open_page(At::Settings(sec), window, cx);
                } else if let Some(sec) =
                    id.strip_prefix("admin-").and_then(admin::Section::from_id)
                {
                    self.open_page(At::Admin(sec), window, cx);
                }
            }
        }
    }

    /// A key press anywhere in the main screen, before the focused element: the app's shortcuts (`Shell.svelte`).
    pub(super) fn shortcut_key(
        &mut self,
        e: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model.screen != crate::model::Screen::Main || self.capturing_key {
            return;
        }
        let set = &self.model.settings;
        let ptt = set.push_to_talk
            && crate::voice::ptt::key_from_window(&e.keystroke.key).is_ok_and(|n| n == set.ptt_key);
        if ptt || self.model.ptt_open {
            return;
        }
        let Some(s) = shortcut(&e.keystroke, self.typing(window, cx)) else {
            return;
        };
        let ids = self.room_order();
        let current = self.model.active.filter(|_| !self.settings_open);
        let to = match s {
            Shortcut::Escape => return,
            Shortcut::Switcher => {
                self.open_layer(Layer::Switcher, window, cx);
                None
            }
            Shortcut::Search => {
                self.open_layer(Layer::Search, window, cx);
                None
            }
            Shortcut::Help => {
                self.open_layer(Layer::Shortcuts, window, cx);
                None
            }
            Shortcut::Mute | Shortcut::Deafen => {
                if self.model.call.phase != Phase::Joined {
                    return; // not in a call: the keys are left alone
                }
                if s == Shortcut::Mute {
                    self.act(|m| m.toggle_mute(), cx);
                } else {
                    self.act(|m| m.toggle_deafen(), cx);
                }
                None
            }
            Shortcut::Sidebar => {
                let on = !self.model.settings.sidebar_collapsed;
                self.set_collapsed(on, cx);
                None
            }
            Shortcut::PrevRoom => switcher::step_room(&ids, current, -1),
            Shortcut::NextRoom => switcher::step_room(&ids, current, 1),
            Shortcut::NextUnread => switcher::next_unread(&ids, current, &self.model.state.unread),
        };
        if let Some(room) = to {
            self.close_layer(window, cx);
            self.open_room(room, window, cx);
        }
        cx.stop_propagation();
    }

    /// A text input has focus.
    pub(super) fn typing(&self, window: &Window, cx: &App) -> bool {
        [
            &self.server,
            &self.username,
            &self.password,
            &self.composer,
            &self.edit,
            &self.find_field,
            &self.after_field,
            &self.before_field,
        ]
        .iter()
        .any(|i| i.read(cx).focus_handle(cx).is_focused(window))
    }

    /// Up or Down in the field.
    pub(super) fn find_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let step = |i: usize, n: usize| if down { (i + 1) % n } else { (i + n - 1) % n };
        match self.find.layer {
            Some(Layer::Switcher) => {
                let n = self.switcher_results(cx).len();
                if n > 0 {
                    self.find.active = step(self.find.active, n);
                    self.scroll_to_active(cx);
                }
            }
            Some(Layer::Search) => {
                let n = self.suggestions(cx).len();
                if n > 0 {
                    self.find.active = step(self.find.active, n);
                } else {
                    let hits = self.find.search.hits.len();
                    self.find.hit = match (self.find.hit, down) {
                        (None, true) if hits > 0 => Some(0),
                        (Some(i), true) => Some((i + 1).min(hits.saturating_sub(1))),
                        (Some(0), false) => None,
                        (Some(i), false) => Some(i - 1),
                        (h, _) => h,
                    };
                    if let Some(i) = self.find.hit {
                        self.find.scroll.scroll_to_item(self.hit_child(i));
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Home or End in the switcher: the first or last row. Elsewhere the field's.
    pub(super) fn find_edge(&mut self, end: bool, cx: &mut Context<Self>) {
        if self.find.layer != Some(Layer::Switcher) {
            return cx.propagate();
        }
        let n = self.switcher_results(cx).len();
        self.find.active = if end { n.saturating_sub(1) } else { 0 };
        self.scroll_to_active(cx);
        cx.notify();
    }

    /// Backspace at the start of the field removes the switcher's chip.
    pub(super) fn find_back(&mut self, cx: &mut Context<Self>) {
        if self.find.scope.is_some() && self.find_field.read(cx).cursor() == 0 {
            (self.find.scope, self.find.active) = (None, 0);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    /// Enter (or Tab on a suggestion).
    pub(super) fn find_enter(&mut self, tab: bool, window: &mut Window, cx: &mut Context<Self>) {
        match self.find.layer {
            Some(Layer::Switcher) if !tab => {
                if let Some(it) = self.switcher_results(cx).get(self.find.active).cloned() {
                    self.choose(it, window, cx);
                }
            }
            Some(Layer::Search) => {
                if !self.suggestions(cx).is_empty() {
                    return self.pick(self.find.active, window, cx);
                }
                if tab {
                    return cx.propagate();
                }
                let s = &self.find.search;
                if let Some(i) = self.find.hit {
                    self.go_to_hit(i, window, cx);
                } else if s.status == Status::Busy {
                    self.search_retry();
                } else if s.status == Status::Done && !s.hits.is_empty() {
                    self.go_to_hit(0, window, cx);
                }
                cx.notify();
            }
            _ => cx.propagate(),
        }
    }

    fn scroll_to_active(&self, cx: &App) {
        // The list's children are the group headings and the rows: find the active row's place among them.
        let results = self.switcher_results(cx);
        let mut child = 0;
        for (i, it) in results.iter().enumerate() {
            if i == 0 || results[i - 1].kind != it.kind {
                child += 1;
            }
            if i == self.find.active {
                break;
            }
            child += 1;
        }
        self.find.scroll.scroll_to_item(child);
    }

    /// The child index of hit `i` in the results list (day headings in between).
    fn hit_child(&self, i: usize) -> usize {
        let days = self.hit_days();
        let mut child = 1; // the count row
        for (k, d) in days.iter().enumerate() {
            if k == 0 || days[k - 1] != *d {
                child += 1;
            }
            if k == i {
                break;
            }
            child += 1;
        }
        child
    }

    fn hit_days(&self) -> Vec<String> {
        let now = chrono::Utc::now().timestamp_millis();
        let loc = crate::timeline::Locale::system();
        self.find
            .search
            .hits
            .iter()
            .map(|h| crate::timeline::day_label(h.message.created_at, now, &chrono::Local, loc))
            .collect()
    }

    /// The people or rooms for the `from:` or `in:` word at the caret: (value, label, user id).
    fn suggestions(&self, cx: &App) -> Vec<(String, String, Option<String>)> {
        if self.find.layer != Some(Layer::Search) {
            return vec![];
        }
        let field = self.find_field.read(cx);
        let Some(at) = searchq::key_at(&field.value(), field.cursor()) else {
            return vec![];
        };
        let t = at.q.to_lowercase();
        let has = |x: &str| x.to_lowercase().contains(&t);
        let s = &self.model.state;
        if at.key == Key::From {
            let mut users: Vec<_> = s
                .users
                .values()
                .filter(|u| !u.banned && (has(&u.username) || has(&u.display_name)))
                .collect();
            users.sort_by(|a, b| a.display_name.cmp(&b.display_name));
            users
                .into_iter()
                .take(6)
                .map(|u| {
                    (
                        u.username.clone(),
                        u.display_name.clone(),
                        Some(u.user_id.clone()),
                    )
                })
                .collect()
        } else {
            let mut rooms: Vec<_> = s.rooms.iter().filter(|r| has(&r.name)).collect();
            rooms.sort_by_key(|r| r.position);
            rooms
                .into_iter()
                .take(6)
                .map(|r| (r.name.clone(), r.name.clone(), None))
                .collect()
        }
    }

    fn pick(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let sugs = self.suggestions(cx);
        let field = self.find_field.read(cx);
        let (text, caret) = (field.value().to_string(), field.cursor());
        let (Some(at), Some(s)) = (searchq::key_at(&text, caret), sugs.get(i)) else {
            return;
        };
        let (text, caret) = searchq::complete(&text, &at, &s.0);
        self.set_find_text(text, Some(caret), window, cx);
    }

    fn set_find_text(
        &mut self,
        text: String,
        caret: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_field.update(cx, |i, cx| {
            let end = caret.unwrap_or(text.len());
            i.set_value(text, window, cx);
            let pos = i.text().offset_to_position(end);
            i.set_cursor_position(pos, window, cx);
            i.focus(window, cx);
        });
        self.find.active = 0;
        self.search_changed(cx);
        cx.notify();
    }

    /// A date field changed while typed in: its key in the box follows.
    pub(super) fn date_changed(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) {
        let field = if key == Key::After {
            &self.after_field
        } else {
            &self.before_field
        };
        if !field.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        let v = field.read(cx).value().trim().to_string();
        let valid = v.is_empty() || chrono::NaiveDate::parse_from_str(&v, "%Y-%m-%d").is_ok();
        if !valid {
            return;
        }
        let q = searchq::set_key(
            &self.find_text(cx),
            key,
            Some(v.as_str()).filter(|v| !v.is_empty()),
        );
        self.find_field
            .update(cx, |i, cx| i.set_value(q, window, cx));
        self.search_changed(cx);
        cx.notify();
    }

    /// What is open over the shell, or None.
    pub(super) fn render_layer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let layer = self.find.layer?;
        let t = tokens(cx).clone();
        let pose = self
            .find
            .since
            .and_then(|at| motion::pose(&t.motion, POP, false, at, Instant::now()));
        if pose.is_some() {
            window.request_animation_frame();
        }
        let opacity = pose.map_or(1., |p| p.opacity);
        if layer == Layer::Shortcuts {
            let close = cx.listener(|this, _: &gpui::ClickEvent, w, cx| this.close_layer(w, cx));
            return Some(
                deferred(
                    div()
                        .absolute()
                        .inset_0()
                        .track_focus(&self.dialog_focus)
                        .debug_selector(|| "shortcuts".into())
                        .opacity(opacity)
                        .child(
                            c::Dialog::new("Keyboard shortcuts")
                                .child(shortcut_list(&t))
                                .on_close(close),
                        ),
                )
                .with_priority(3)
                .into_any_element(),
            );
        }
        let search = layer == Layer::Search;
        let vh = window.viewport_size().height;
        let max_h = (vh - px(128.)).min(px(if search { 640. } else { 520. }));
        let body: Vec<AnyElement> = if search {
            self.search_body(&t, window, cx)
        } else {
            self.switcher_body(&t, cx)
        };
        let mut backdrop: Hsla = t.bg_app.into();
        backdrop.a = 0.72;
        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .occlude()
                    .flex()
                    .flex_col()
                    .items_center()
                    .pt(px(96.))
                    .bg(backdrop)
                    .opacity(opacity)
                    .child(
                        div()
                            .id("find-dialog")
                            .debug_selector(move || {
                                (if search { "search" } else { "switcher" }).into()
                            })
                            .key_context(FIND)
                            .flex()
                            .flex_col()
                            .w(px(if search { 720. } else { 600. }))
                            .max_h(max_h)
                            .overflow_hidden()
                            .border_1()
                            .border_color(t.line_subtle)
                            .rounded(t.radius_lg)
                            .bg(t.bg_raised)
                            .shadow(t.shadow_float.clone())
                            .text_step(TEXT_SM)
                            .text_color(t.fg_2)
                            .on_mouse_down_out(
                                cx.listener(|this, _, w, cx| this.close_layer(w, cx)),
                            )
                            .children(body),
                    ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }

    /// `SwitcherField.svelte`: 48 px, 16 px sides, 12 px gap, the search icon (20, `--fg-3`), an optional chip, the
    /// field (16/24). Its rule is `--accent-line`: it always has focus.
    fn find_field_row(&self, chip: Option<&'static str>, t: &Tokens) -> Div {
        div()
            .flex()
            .flex_none()
            .h(px(48.))
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .border_b_1()
            .border_color(t.accent_line)
            .child(icon("search").size(px(20.)).text_color(t.fg_3))
            .children(chip.map(|c| {
                div()
                    .flex()
                    .flex_none()
                    .h(px(24.))
                    .items_center()
                    .px(px(8.))
                    .rounded(t.radius_sm)
                    .bg(t.accent_subtle)
                    .text_color(t.accent_fg)
                    .text_size(px(14.))
                    .font_weight(t.weight_ui)
                    .child(c)
            }))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.find_field)
                        .appearance(false)
                        .p_0()
                        .text_step(TEXT_LG)
                        .text_color(t.fg),
                ),
            )
    }

    /// The footer: 36 px, `--line-subtle` above, 12/16 `--fg-3`, 16 px gaps.
    fn find_footer(t: &Tokens, enter: &'static str, right: Div) -> Div {
        let pair = |keys: &[&'static str], what: &'static str| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .children(keys.iter().map(|k| kbd(*k, t)))
                .child(what)
        };
        div()
            .flex()
            .flex_none()
            .h(px(36.))
            .items_center()
            .gap(px(16.))
            .px(px(16.))
            .border_t_1()
            .border_color(t.line_subtle)
            .text_step(TEXT_XS)
            .text_color(t.fg_3)
            .child(pair(&["↑", "↓"], "move"))
            .child(pair(&["Enter"], enter))
            .child(pair(&["Esc"], "close"))
            .child(div().flex_1().flex().justify_end().child(right))
    }

    fn switcher_body(&mut self, t: &Tokens, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let scope = self.find.scope.map(|k| match k {
            Kind::Room => "Rooms",
            Kind::Person => "People",
            Kind::Action => "Actions",
        });
        let results = self.switcher_results(cx);
        self.find.active = self.find.active.min(results.len().saturating_sub(1));
        let field = self.find_field_row(scope, t);
        let s = &self.model.state;
        let mut rows: Vec<AnyElement> = vec![];
        for (i, it) in results.iter().enumerate() {
            if i == 0 || results[i - 1].kind != it.kind {
                let name = match it.kind {
                    Kind::Room => "Rooms",
                    Kind::Person => "People",
                    Kind::Action => "Actions",
                };
                rows.push(
                    div()
                        .flex()
                        .h(px(28.))
                        .items_center()
                        .px(px(8.))
                        .when(i > 0, |d| d.mt(px(8.)))
                        .text_step(TEXT_XS)
                        .text_color(t.fg_3)
                        .child(name)
                        .into_any_element(),
                );
            }
            let on = i == self.find.active;
            let lead_c = if on { t.fg_2 } else { t.fg_3 };
            let lead: AnyElement = match (it.kind, s.users.get(&it.id)) {
                (Kind::Person, Some(u)) => Avatar::new(
                    u.user_id.clone(),
                    u.display_name.clone(),
                    u.accent.clone(),
                    20,
                )
                .into_any_element(),
                (k, _) => div()
                    .flex()
                    .flex_none()
                    .w(px(20.))
                    .justify_center()
                    .child(
                        icon(if k == Kind::Room {
                            "room"
                        } else {
                            action_icon(&it.id)
                        })
                        .text_color(lead_c),
                    )
                    .into_any_element(),
            };
            let un = (it.kind == Kind::Room)
                .then(|| it.id.parse::<i64>().ok().and_then(|id| s.unread.get(&id)))
                .flatten();
            let mut right = div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .pl(px(12.));
            if let Some(u) = un {
                if u.unread > 0 {
                    right = right.child(Badge::new(false, u.unread.into()));
                }
                if u.mentions > 0 {
                    right = right.child(Badge::new(true, u.mentions.into()));
                }
            }
            if let Some(h) = &it.hint {
                right = match it.kind {
                    Kind::Person => {
                        right.child(div().text_step(TEXT_XS).text_color(lead_c).child(h.clone()))
                    }
                    Kind::Action => {
                        let s = match h.as_str() {
                            "mute" => Shortcut::Mute,
                            "deafen" => Shortcut::Deafen,
                            "search" => Shortcut::Search,
                            "sidebar" => Shortcut::Sidebar,
                            _ => Shortcut::NextUnread,
                        };
                        right.child(
                            div()
                                .flex()
                                .gap(px(4.))
                                .children(keys_of(s).into_iter().map(|k| kbd(k, t))),
                        )
                    }
                    Kind::Room => right,
                };
            }
            let item = it.clone();
            rows.push(
                div()
                    .id(row_hover(i))
                    .debug_selector(move || format!("switcher-row-{i}"))
                    .flex()
                    .h(px(36.))
                    .items_center()
                    .gap(px(8.))
                    .px(px(8.))
                    .rounded(t.radius)
                    .when(on, |d| d.bg(t.line))
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.find.active != i {
                            this.find.active = i;
                            cx.notify();
                        }
                    }))
                    .on_click(cx.listener(move |this, _, w, cx| this.choose(item.clone(), w, cx)))
                    .child(lead)
                    .child(
                        div()
                            .min_w_0()
                            .ellipsis()
                            .text_color(t.fg)
                            .child(it.label.clone()),
                    )
                    .child(div().flex_1())
                    .child(right)
                    .into_any_element(),
            );
        }
        let q = self.find_text(cx);
        let mut out = vec![field.into_any_element()];
        if results.is_empty() {
            let text = if q.trim().is_empty() {
                "Nothing to show here.".to_string()
            } else {
                format!("Nothing matches “{}”.", q.trim())
            };
            out.push(
                div()
                    .px(px(16.))
                    .py(px(24.))
                    .text_center()
                    .text_color(t.fg_2)
                    .child(text)
                    .into_any_element(),
            );
        } else {
            out.push(
                div()
                    .id("switcher-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.find.scroll)
                    .p(px(8.))
                    .children(rows)
                    .into_any_element(),
            );
        }
        let filters = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child("Filter:")
            .child(kbd("r", t))
            .child("rooms")
            .child(kbd("p", t))
            .child("people")
            .child(kbd(">", t))
            .child("actions, then")
            .child(kbd("Space", t));
        out.push(Self::find_footer(t, "open", filters).into_any_element());
        out
    }

    fn search_body(
        &mut self,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let q = self.find_text(cx);
        let parsed = self.parsed(&q);
        let words = parsed.q.trim().to_string();
        let one_short = !words.contains(' ') && searchq::short_last(&words);
        // The date fields show the box's dates while they are not being typed in.
        for (key, f) in [
            (Key::After, self.after_field.clone()),
            (Key::Before, self.before_field.clone()),
        ] {
            let v = searchq::key_value(&q, key);
            if !f.read(cx).focus_handle(cx).is_focused(window) && f.read(cx).value() != v {
                f.update(cx, |i, cx| i.set_value(v, window, cx));
            }
        }
        let sugs = self.suggestions(cx);
        self.find.active = self.find.active.min(sugs.len().saturating_sub(1));
        let current = self.model.active.filter(|_| !self.settings_open);
        let cur_name = current
            .and_then(|id| self.model.state.find_room(id))
            .map(|r| r.name.clone());
        let mut out = vec![self.find_field_row(None, t).into_any_element()];

        // The filter row.
        let mut filters = div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .px(px(16.))
            .py(px(8.))
            .border_b_1()
            .border_color(t.line_subtle)
            .text_step(TEXT_XS)
            .text_color(t.fg_2);
        if let Some(name) = cur_name.clone() {
            // The picked scope's fill slides to the other (`slides` in motion.ts).
            let slide = motion::Slide::new(
                "search-scope",
                usize::from(parsed.room_id.is_some()),
                window,
                cx,
            );
            let gliding = slide.gliding();
            let seg = |id: &'static str,
                       label: &'static str,
                       on: bool,
                       value: Option<String>,
                       cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .flex()
                    .h(px(24.))
                    .items_center()
                    .px(px(8.))
                    .rounded(t.radius_sm)
                    .text_color(if on { t.fg } else { t.fg_2 })
                    .relative()
                    .when(on && !gliding, |d| d.bg(t.bg_selected))
                    .cursor_pointer()
                    .child(slide.track(usize::from(value.is_some())))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        let q = searchq::set_key(&this.find_text(cx), Key::In, value.as_deref());
                        this.set_find_text(q, None, w, cx);
                    }))
                    .child(label)
            };
            let group = div().flex().p(px(2.)).rounded(t.radius).bg(t.bg_element);
            filters = filters.child(
                slide
                    .under(group, t.radius_sm, t.bg_selected, window, cx)
                    .child(seg(
                        "search-all",
                        "All rooms",
                        parsed.room_id.is_none(),
                        None,
                        cx,
                    ))
                    .child(seg(
                        "search-here",
                        "This room",
                        parsed.room_id == current,
                        Some(name),
                        cx,
                    )),
            );
        }
        for (key, text) in parsed.chips.clone() {
            let label = if key == Key::From { "From" } else { "In" };
            filters = filters.child(
                div()
                    .flex()
                    .h(px(28.))
                    .items_center()
                    .gap(px(4.))
                    .pl(px(8.))
                    .pr(px(2.))
                    .rounded(t.radius_sm)
                    .bg(t.accent_subtle)
                    .text_color(t.accent_fg)
                    .child(label)
                    .child(
                        div()
                            .font_weight(t.weight_strong)
                            .text_color(t.fg)
                            .child(text),
                    )
                    .child(x_button(
                        if key == Key::From {
                            "search-x-from"
                        } else {
                            "search-x-in"
                        },
                        t,
                        cx.listener(move |this, _, w, cx| {
                            let q = searchq::set_key(&this.find_text(cx), key, None);
                            this.set_find_text(q, None, w, cx);
                        }),
                    )),
            );
        }
        let mut dates = div()
            .flex_1()
            .flex()
            .justify_end()
            .items_center()
            .gap(px(8.));
        for (key, label, field) in [
            (Key::After, "After", self.after_field.clone()),
            (Key::Before, "Before", self.before_field.clone()),
        ] {
            let set = !searchq::key_value(&q, key).is_empty();
            dates = dates
                .child(
                    div().flex().items_center().gap(px(4.)).child(label).child(
                        div().w(px(118.)).h(px(28.)).overflow_hidden().child(
                            Input::new(&field)
                                .h(px(28.))
                                .px(px(6.))
                                .py_0()
                                .bg(t.bg_element)
                                .border_1()
                                .border_color(t.line)
                                .rounded(t.radius)
                                .text_step(TEXT_XS)
                                .text_color(t.fg),
                        ),
                    ),
                )
                .when(set, |d| {
                    d.child(div().ml(px(-4.)).child(x_button(
                        if key == Key::After {
                            "search-x-after"
                        } else {
                            "search-x-before"
                        },
                        t,
                        cx.listener(move |this, _, w, cx| {
                            let q = searchq::set_key(&this.find_text(cx), key, None);
                            this.set_find_text(q, None, w, cx);
                        }),
                    )))
                });
        }
        out.push(filters.child(dates).into_any_element());

        let centred = |text: String, c: gpui::Rgba| {
            div()
                .px(px(16.))
                .py(px(24.))
                .text_center()
                .text_color(c)
                .child(text)
        };
        let s = &self.find.search;
        let body: AnyElement = if !sugs.is_empty() {
            let mut list = div()
                .id("search-sugs")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(8.));
            for (i, (value, label, uid)) in sugs.iter().enumerate() {
                let on = i == self.find.active;
                let lead: AnyElement =
                    match uid.as_ref().and_then(|u| self.model.state.users.get(u)) {
                        Some(u) => Avatar::new(
                            u.user_id.clone(),
                            u.display_name.clone(),
                            u.accent.clone(),
                            20,
                        )
                        .into_any_element(),
                        None => div()
                            .flex()
                            .w(px(20.))
                            .justify_center()
                            .child(icon("room").text_color(t.fg_3))
                            .into_any_element(),
                    };
                list = list.child(
                    div()
                        .id(ElementId::Name(format!("search-sug-{i}").into()))
                        .flex()
                        .h(px(36.))
                        .items_center()
                        .gap(px(8.))
                        .px(px(8.))
                        .rounded(t.radius)
                        .when(on, |d| d.bg(t.line))
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if this.find.active != i {
                                this.find.active = i;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(move |this, _, w, cx| this.pick(i, w, cx)))
                        .child(lead)
                        .child(
                            div()
                                .min_w_0()
                                .ellipsis()
                                .text_color(t.fg)
                                .child(label.clone()),
                        )
                        .when(uid.is_some(), |d| {
                            d.child(
                                div()
                                    .ellipsis()
                                    .text_step(TEXT_XS)
                                    .text_color(t.fg_3)
                                    .child(format!("@{value}")),
                            )
                        }),
                );
            }
            list.into_any_element()
        } else if !parsed.unknown.is_empty() {
            let mut d = div()
                .px(px(16.))
                .py(px(24.))
                .text_center()
                .text_color(t.fg_2);
            for u in &parsed.unknown {
                d = d.child(div().child(u.clone()));
            }
            if parsed.unknown.iter().any(|u| u.starts_with("No ")) {
                d = d.child(
                    div()
                        .mt(px(4.))
                        .text_step(TEXT_XS)
                        .text_color(t.fg_3)
                        .child("Type from: or in: and pick a name from the list."),
                );
            }
            d.into_any_element()
        } else if q.trim().is_empty() {
            let row = |code: &'static str, what: &'static str| {
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        div()
                            .w(px(130.))
                            .flex_none()
                            .font_family(t.font_mono.clone())
                            .text_size(px(12.))
                            .text_color(t.fg_2)
                            .child(code),
                    )
                    .child(div().text_color(t.fg_3).child(what))
            };
            div()
                .px(px(16.))
                .py(px(24.))
                .text_color(t.fg_2)
                .child(
                    div()
                        .mb(px(12.))
                        .text_color(t.fg)
                        .child("Find messages in the rooms you can see, newest first."),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(row(
                            "from:alice",
                            "by one person (quote a name with spaces)",
                        ))
                        .child(row("in:general", "in one room"))
                        .child(row("after:2026-10-01", "on or after that day"))
                        .child(row("before:2026-10-31", "on or before that day")),
                )
                .into_any_element()
        } else if words.is_empty() {
            centred("Add a word to search for.".into(), t.fg_2).into_any_element()
        } else if one_short {
            centred(
                "Type at least 2 letters to search as you type.".into(),
                t.fg_2,
            )
            .into_any_element()
        } else if searchq::nothing_to_search(&words)
            || (s.status == Status::Done && s.hits.is_empty())
        {
            div()
                .px(px(16.))
                .py(px(24.))
                .text_center()
                .text_color(t.fg_2)
                .child("No messages found.")
                .child(
                    div()
                        .mt(px(4.))
                        .text_step(TEXT_XS)
                        .text_color(t.fg_3)
                        .child("Try other words, or fewer filters."),
                )
                .into_any_element()
        } else if s.status == Status::Error {
            centred(s.error.clone(), t.danger_fg).into_any_element()
        } else if s.status == Status::Busy && s.hits.is_empty() {
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .py(px(24.))
                .text_center()
                .text_color(t.fg_2)
                .child(BUSY)
                .child(
                    c::Button::new("search-retry", "Try again")
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.search_retry();
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else if s.hits.is_empty() {
            centred("Searching...".into(), t.fg_2).into_any_element()
        } else {
            self.hits_list(&words, t, cx)
        };
        out.push(body);
        let enter = if !sugs.is_empty() {
            "pick"
        } else if self.find.search.status == Status::Busy {
            "try again"
        } else {
            "jump"
        };
        out.push(
            Self::find_footer(
                t,
                enter,
                div().child("Dates are whole days in your time zone."),
            )
            .into_any_element(),
        );
        out
    }

    fn hits_list(&mut self, words: &str, t: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let s = &self.find.search;
        let n = s.hits.len();
        let count = format!(
            "{n}{} {}",
            if s.next.is_some() { "+" } else { "" },
            if n == 1 && s.next.is_none() {
                "message"
            } else {
                "messages"
            }
        );
        let head = match s.status {
            Status::Done => count,
            Status::Busy => BUSY.into(),
            _ => "Searching...".into(),
        };
        let mut list = div()
            .id("search-results")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.find.scroll)
            .child(
                div()
                    .flex()
                    .h(px(32.))
                    .items_center()
                    .gap(px(12.))
                    .px(px(16.))
                    .text_step(TEXT_XS)
                    .text_color(t.fg_3)
                    .child(head)
                    .when(s.status == Status::Busy, |d| {
                        d.child(
                            div()
                                .id("search-retry-inline")
                                .font_weight(t.weight_strong)
                                .text_color(t.fg_2)
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.search_retry();
                                    cx.notify();
                                }))
                                .child("Try again"),
                        )
                    })
                    .when(searchq::short_last(words), |d| {
                        d.child("Type at least 2 letters to search as you type.")
                    }),
            );
        if !self.find.jump_error.is_empty() {
            list = list.child(
                div()
                    .px(px(16.))
                    .pb(px(8.))
                    .text_step(TEXT_XS)
                    .text_color(t.danger_fg)
                    .child(self.find.jump_error.clone()),
            );
        }
        let days = self.hit_days();
        let st = &self.model.state;
        let loc = crate::timeline::Locale::system();
        let mut rows = div().px(px(8.)).pb(px(8.));
        for (i, h) in self.find.search.hits.iter().enumerate() {
            if i == 0 || days[i - 1] != days[i] {
                rows = rows.child(
                    div()
                        .flex()
                        .h(px(28.))
                        .items_center()
                        .px(px(8.))
                        .text_step(TEXT_XS)
                        .text_color(t.fg_3)
                        .child(days[i].clone()),
                );
            }
            let m = &h.message;
            let (name, accent) = st
                .users
                .get(&m.author_id)
                .map_or(("Someone".to_string(), String::new()), |u| {
                    (u.display_name.clone(), u.accent.clone())
                });
            let room = st.find_room(m.room_id).map(|r| r.name.clone());
            let lit = self.find.hit == Some(i) || self.find.hovered == Some(i);
            let mut snip = div().mt(px(2.)).text_color(t.fg_2).flex().flex_wrap();
            // shortcut: each run is its own box, so a long run wraps as a block; fine for one-line snippets.
            for (text, mark) in searchq::marks(&h.snippet) {
                snip = snip.child(
                    div()
                        .when(mark, |d| {
                            d.rounded(px(2.))
                                .bg(t.accent_subtle)
                                .text_color(t.accent_fg)
                        })
                        .child(text.replace(' ', "\u{a0}")),
                );
            }
            rows = rows.child(
                div()
                    .id(ElementId::Name(format!("search-hit-{i}").into()))
                    .debug_selector(move || format!("search-hit-{i}"))
                    .flex()
                    .w_full()
                    .gap(px(12.))
                    .p(px(8.))
                    .rounded(t.radius)
                    .when(lit, |d| d.bg(t.bg_hover))
                    .cursor_pointer()
                    .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                        this.find.hovered = if *on {
                            Some(i)
                        } else {
                            this.find.hovered.filter(|&h| h != i)
                        };
                        cx.notify();
                    }))
                    .on_click(cx.listener(move |this, _, w, cx| this.go_to_hit(i, w, cx)))
                    .child(Avatar::new(m.author_id.clone(), name.clone(), accent, 32))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .ellipsis()
                                            .font_weight(t.weight_strong)
                                            .text_color(t.fg)
                                            .child(name),
                                    )
                                    .children(room.map(|r| {
                                        div()
                                            .ellipsis()
                                            .text_step(TEXT_XS)
                                            .text_color(t.fg_3)
                                            .child(format!("in {r}"))
                                    }))
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_step(TEXT_XS)
                                            .text_color(t.fg_3)
                                            .child(crate::timeline::clock(
                                                m.created_at,
                                                loc,
                                                false,
                                            )),
                                    )
                                    .child(div().flex_1())
                                    .when(lit, |d| {
                                        d.child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(px(2.))
                                                .text_step(TEXT_XS)
                                                .text_color(t.fg_2)
                                                .child("Jump")
                                                .child(icon("chevronRight").text_color(t.fg_2)),
                                        )
                                    }),
                            )
                            .child(snip),
                    ),
            );
        }
        if self.find.search.next.is_some() {
            rows = rows.child(
                div().flex().justify_center().pt(px(8.)).child(
                    c::Button::new("search-more", "More results")
                        .small()
                        .loading(self.find.search.paging)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.search_more();
                            cx.notify();
                        })),
                ),
            );
        }
        list.child(rows).into_any_element()
    }
}

/// The search dialog's `.x`: 4 px around a 16 px icon, `--radius-sm`, `--bg-hover` on hover.
fn x_button(
    id: &'static str,
    t: &Tokens,
    f: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let hover = t.bg_hover;
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .p(px(4.))
        .rounded(t.radius_sm)
        .bg(transparent_black())
        .hover(move |s| s.bg(hover))
        .cursor_pointer()
        .on_click(f)
        .child(icon("x").text_color(t.fg_2))
}

/// `ShortcutList.svelte`: every shortcut with its keys, then the switcher's filters.
fn shortcut_list(t: &Tokens) -> Div {
    let row = |label: &'static str, keys: Vec<&'static str>| {
        div()
            .flex()
            .h(px(32.))
            .items_center()
            .gap(px(16.))
            .child(div().flex_1().min_w_0().text_color(t.fg_2).child(label))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap(px(4.))
                    .children(keys.into_iter().map(|k| kbd(k, t))),
            )
    };
    let heading = |text: &'static str| {
        div()
            .mt(px(16.))
            .flex()
            .h(px(28.))
            .items_center()
            .text_step(TEXT_XS)
            .text_color(t.fg_3)
            .child(text)
    };
    div()
        .flex()
        .flex_col()
        .children(SHORTCUTS.iter().map(|d| row(d.label, keys_for(d))))
        .child(heading("In the switcher, start with"))
        .child(row("Only rooms", vec!["r", "Space"]))
        .child(row("Only people", vec!["p", "Space"]))
        .child(row("Only actions", vec![">", "Space"]))
        .child(
            div()
                .mt(px(16.))
                .text_step(TEXT_XS)
                .text_color(t.fg_3)
                .child("Mute and deafen keys work only in a call."),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str, ctrl: bool, alt: bool, shift: bool) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers: gpui::Modifiers {
                control: ctrl,
                alt,
                shift,
                ..Default::default()
            },
            key: k.into(),
            key_char: None,
        }
    }

    #[test]
    fn shortcuts_follow_keys_ts() {
        assert_eq!(
            shortcut(&key("k", !MAC, false, false), true),
            Some(Shortcut::Switcher)
        );
        assert_eq!(
            shortcut(&key("f", !MAC, false, true), false),
            Some(Shortcut::Search)
        );
        assert_eq!(
            shortcut(&key("up", false, true, false), false),
            Some(Shortcut::PrevRoom)
        );
        assert_eq!(
            shortcut(&key("up", false, true, false), true),
            None,
            "Alt+Up is the field's"
        );
        assert_eq!(
            shortcut(&key("up", true, true, false), true),
            Some(Shortcut::PrevRoom)
        );
        assert_eq!(
            shortcut(&key("down", false, true, true), false),
            Some(Shortcut::NextUnread)
        );
        let mut q = key("/", false, false, true);
        q.key_char = Some("?".into());
        assert_eq!(shortcut(&q, false), Some(Shortcut::Help));
        assert_eq!(shortcut(&q, true), None, "? types in a field");
        assert_eq!(
            keys_of(Shortcut::Mute),
            if MAC {
                vec!["⌘", "⇧", "M"]
            } else {
                vec!["Ctrl", "Shift", "M"]
            }
        );
        assert_eq!(keys_of(Shortcut::Help), ["?"]);
    }
}
