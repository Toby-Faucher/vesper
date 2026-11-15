//! The window: one view, `Vesper`, that owns the `Model` and draws the sign in screen or the main screen. Events from
//! the network thread come in through `listen`; what the model asks for goes out on `cmds`.

mod banner;
pub mod c;
mod call;
mod chat;
#[cfg(debug_assertions)]
mod dev;
pub mod icons;
mod md;
pub mod motion;
mod person;
mod room;
pub mod settings;
mod share;
mod sidebar;
mod sign_in;
pub mod theme;
mod tray;

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use gpui::{
    App, AppContext, Application, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyBinding, KeyDownEvent, KeyUpEvent, ListAlignment, ListState, ParentElement, Render,
    RenderImage, Styled, Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowOptions,
    actions, div, px, rgb, size,
};
use gpui_component::Root;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::slider::SliderState;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::model::{Model, Phase, Screen};
use crate::net::{self, Cmd, Event};
use crate::store::Keyring;
use crate::voice::Cpal;
use crate::voice::ptt::{self, KeyMsg};

actions!(
    vesper,
    [
        SendMessage,
        NewLine,
        SaveEdit,
        CancelEdit,
        ComposerUp,
        ComposerDown,
        ComposerTab,
        ComposerEscape,
        PasteFile
    ]
);

/// The key context of the composer.
const COMPOSER: &str = "Composer";
/// The key context of a message's edit box.
const EDIT_BOX: &str = "EditBox";

/// Keys for the app, bound after gpui-component's so they win over its multi-line input.
pub fn init(cx: &mut App) {
    gpui_component::init(cx);
    theme::apply(cx);
    let input = Some("Composer > Input");
    let edit = Some("EditBox > Input");
    // A handler that does not use its key (Up with text in the box) lets it through to the input.
    cx.bind_keys([
        KeyBinding::new("enter", SendMessage, input),
        KeyBinding::new("shift-enter", NewLine, input),
        KeyBinding::new("up", ComposerUp, input),
        KeyBinding::new("down", ComposerDown, input),
        KeyBinding::new("tab", ComposerTab, input),
        KeyBinding::new("escape", ComposerEscape, input),
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-v", PasteFile, input),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-v", PasteFile, input),
        KeyBinding::new("enter", SaveEdit, edit),
        KeyBinding::new("shift-enter", NewLine, edit),
        KeyBinding::new("escape", CancelEdit, edit),
    ]);
}

/// Starts the network and key threads and opens the window. Returns when the app quits.
pub fn run() {
    let audio: Arc<dyn crate::voice::AudioDevices> = Arc::new(Cpal);
    // The parity harness's calls run on fake devices (dev.rs).
    #[cfg(debug_assertions)]
    let audio = dev::audio().unwrap_or(audio);
    let (cmds, events) = net::start(Arc::new(Keyring), audio);
    let file = crate::settings::path();
    let saved = file
        .as_deref()
        .map(crate::settings::Settings::load)
        .unwrap_or_default();
    let (open_tx, open_rx) = tokio::sync::mpsc::unbounded_channel();
    let keys = ptt::spawn(ptt::global_source, move |open| {
        let _ = open_tx.send(open);
    });
    let app = Application::new().with_assets(icons::Assets);
    app.run(move |cx: &mut App| {
        init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("Vesper".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let handle = cx
            .open_window(options, |window, cx| {
                let view = cx.new(|cx| {
                    let mut v = Vesper::new(cmds, window, cx);
                    v.listen(events, window, cx);
                    v.listen_ptt(open_rx, window, cx);
                    v.model.settings = saved;
                    (v.settings_file, v.ptt_tx) = (file, Some(keys));
                    v.ptt_note = ptt::global_note();
                    v.watch_key();
                    v
                });
                // Closing the window during a call keeps the call: the window goes to the tray (minimised: GPUI
                // 0.2.2 cannot hide a window on Windows). Leave, or Quit in the tray, ends it.
                let weak = view.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    let in_call = weak
                        .upgrade()
                        .is_some_and(|v| v.read(cx).model.call.phase != Phase::Idle);
                    if in_call {
                        window.minimize_window();
                    }
                    !in_call
                });
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("the window opens");
        if let Some(view) = handle
            .read(cx)
            .ok()
            .and_then(|r| r.view().clone().downcast::<Vesper>().ok())
        {
            let tray = tray::start(cx, view.downgrade(), handle.into());
            view.update(cx, |v, _| v._tray = tray);
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

pub struct Vesper {
    pub model: Model,
    cmds: UnboundedSender<Cmd>,
    server: Entity<InputState>,
    username: Entity<InputState>,
    password: Entity<InputState>,
    /// The password is shown in clear (the Show toggle).
    password_shown: bool,
    composer: Entity<InputState>,
    /// The edit box of the message being edited.
    edit: Entity<InputState>,
    list: ListState,
    /// The list is top aligned: its messages fit, and start at the top as the web's do. Else it is bottom aligned,
    /// which keeps the newest message in view as more come.
    list_top: bool,
    /// What the list shows: the room and each row's id and what its look depends on, to tell the list which rows
    /// changed.
    shown: (Option<i64>, Vec<(i64, RowKey)>),
    /// Each shown message's day row and grouping, made before the list draws.
    heads: Vec<crate::timeline::RowHead>,
    /// Inline pictures by upload id, and the order they came in.
    images: HashMap<String, chat::Img>,
    image_order: VecDeque<String>,
    /// The `@` being typed (its byte offset and the text after it), the highlighted person, and an `@` whose list
    /// Esc closed.
    mention: Option<(usize, String)>,
    mention_ix: usize,
    mention_dismissed: Option<usize>,
    /// Draws again when the soonest "is typing" line ends.
    typing_timer: Option<(Instant, Task<()>)>,
    /// Volume sliders of the people in calls, by user id.
    sliders: HashMap<String, (Entity<SliderState>, Subscription)>,
    /// The Settings view is shown instead of the room.
    pub settings_open: bool,
    /// The settings file. Set in the real app only: then changes are saved, devices are listed and start with the
    /// computer is set. Tests leave it empty.
    pub settings_file: Option<PathBuf>,
    /// Input and output device names for the Settings view.
    pub devices: (Vec<String>, Vec<String>),
    /// Choosing the push-to-talk key: the next key pressed is it.
    pub capturing_key: bool,
    key_problem: &'static str,
    key_focus: FocusHandle,
    settings_error: String,
    bitrate: Option<(Entity<SliderState>, Subscription)>,
    /// To the key thread (none in tests).
    pub ptt_tx: Option<Sender<KeyMsg>>,
    /// Why push to talk works only while the window has focus.
    pub ptt_note: Option<&'static str>,
    /// The watched screen's newest picture, by sharer.
    screen: Option<(String, Arc<RenderImage>)>,
    /// Sends the next `watch off` when it is due.
    watch_timer: Option<(Instant, Task<()>)>,
    _tray: Option<tray_icon::TrayIcon>,
    /// The sidebar's menu, folds and motion.
    side: sidebar::Side,
    /// The timeline's motion, New row and open menus.
    tl: chat::Tl,
    /// The room whose name the composer's placeholder has.
    placeholder_room: Option<i64>,
    /// Since when the call bar shows, for its rise.
    bar_since: Option<Instant>,
    /// The open person card, the opener a press outside closed it on (and when), and where openers were drawn.
    person: Option<person::Card>,
    person_closed: Option<(String, Instant)>,
    anchors: person::Anchors,
    /// The connection banner and the ring prompt's motion.
    banner: banner::Banner,
    /// Debug builds: what the parity harness asked for (`VESPER_DEV_*`).
    #[cfg(debug_assertions)]
    dev: dev::Dev,
    _subs: Vec<Subscription>,
    _pump: Option<Task<()>>,
}

impl Vesper {
    pub fn new(cmds: UnboundedSender<Cmd>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let server =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://vesper.example.com"));
        let username = cx.new(|cx| InputState::new(window, cx));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(1, 10)
                .placeholder("Message")
        });
        // A fixed height, so the row in the list keeps its size while the text changes.
        let edit = cx.new(|cx| InputState::new(window, cx).multi_line(true).rows(3));
        let enter = |this: &mut Self,
                     _: &Entity<InputState>,
                     e: &InputEvent,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            if matches!(e, InputEvent::PressEnter { .. }) {
                this.submit_sign_in(window, cx);
            }
        };
        let subs = vec![
            cx.subscribe_in(&server, window, enter),
            cx.subscribe_in(&username, window, enter),
            cx.subscribe_in(&password, window, enter),
            cx.subscribe_in(&composer, window, |this, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    this.composer_changed(cx);
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                if !active {
                    // The key-up never comes once focus is gone: a window-held key would leave the mic open.
                    this.window_key(None, false);
                }
                let cmds = this.model.set_focused(active);
                this.run(cmds);
                cx.notify();
            }),
        ];
        let list = Self::new_list(false, cx);
        let _ = cmds.send(Cmd::Restore); // the first thing the app asks for: the stored session
        Vesper {
            model: Model::default(),
            cmds,
            server,
            username,
            password,
            password_shown: false,
            composer,
            edit,
            list,
            list_top: false,
            shown: (None, vec![]),
            heads: vec![],
            images: HashMap::new(),
            image_order: VecDeque::new(),
            mention: None,
            mention_ix: 0,
            mention_dismissed: None,
            typing_timer: None,
            sliders: HashMap::new(),
            settings_open: false,
            settings_file: None,
            devices: (vec![], vec![]),
            capturing_key: false,
            key_problem: "",
            key_focus: cx.focus_handle(),
            settings_error: String::new(),
            bitrate: None,
            ptt_tx: None,
            ptt_note: None,
            screen: None,
            watch_timer: None,
            _tray: None,
            side: sidebar::Side::default(),
            tl: chat::Tl::default(),
            placeholder_room: None,
            bar_since: None,
            person: None,
            person_closed: None,
            anchors: Default::default(),
            banner: Default::default(),
            #[cfg(debug_assertions)]
            dev: dev::Dev::from_env(),
            _subs: subs,
            _pump: None,
        }
    }

    /// Applies events from the network thread as they come, until the thread stops.
    pub fn listen(
        &mut self,
        mut events: UnboundedReceiver<Event>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._pump = Some(cx.spawn_in(window, async move |this, cx| {
            while let Some(ev) = events.recv().await {
                if this
                    .update_in(cx, |v, window, cx| v.apply(ev, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// The key thread's push to talk, as it opens and closes.
    pub fn listen_ptt(
        &mut self,
        mut open: UnboundedReceiver<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(o) = open.recv().await {
                if this
                    .update(cx, |v, cx| v.act(|m| m.set_ptt(o), cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// A key down or up in the window: the push-to-talk key goes to the key thread (all there is on Wayland).
    /// `None` releases it whatever it was.
    fn window_key(&self, key: Option<&str>, down: bool) {
        let s = &self.model.settings;
        let ours = key.is_none_or(|k| ptt::key_from_window(k).is_ok_and(|n| n == s.ptt_key));
        if s.push_to_talk
            && ours
            && let Some(tx) = &self.ptt_tx
        {
            let _ = tx.send(KeyMsg::Window(down));
        }
    }

    /// One event into the model; what it asks for goes to the network.
    pub fn apply(&mut self, ev: Event, window: &mut Window, cx: &mut Context<Self>) {
        let signed_out = matches!(ev, Event::SignedOut { .. });
        #[cfg(debug_assertions)]
        let latest = matches!(ev, Event::Latest { .. });
        match &ev {
            Event::Image { id, picture } => self.image_loaded(id, picture.clone(), window),
            Event::Downloaded { path, open: true } => cx.open_with_system(path),
            Event::Latest { room, .. } => drop(self.tl.loaded.insert(*room)),
            _ => {}
        }
        let editing = self.model.editing;
        let cmds = self.model.apply(ev);
        self.run(cmds);
        if editing.is_some() && self.model.editing.is_none() {
            // The edit was saved: back to the composer.
            self.composer.update(cx, |i, cx| i.focus(window, cx));
        }
        if signed_out && self.server.read(cx).value().is_empty() && !self.model.server.is_empty() {
            let s = self.model.server.clone();
            self.server.update(cx, |i, cx| i.set_value(s, window, cx));
        }
        if let Some(room) = self.model.dm_opened.take() {
            self.person = None;
            self.open_room(room, window, cx);
        }
        if let Some(text) = self.model.draft_back.take()
            && self.composer.read(cx).value().is_empty()
        {
            self.composer
                .update(cx, |i, cx| i.set_value(text, window, cx));
        }
        #[cfg(debug_assertions)]
        self.dev_step(latest, window, cx);
        cx.notify();
    }

    fn run(&self, cmds: Vec<Cmd>) {
        for c in cmds {
            let _ = self.cmds.send(c);
        }
    }

    fn submit_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (server, user, pass) = (
            self.server.read(cx).value(),
            self.username.read(cx).value(),
            self.password.read(cx).value(),
        );
        let cmds = self.model.sign_in(&server, &user, &pass);
        if !cmds.is_empty() {
            // The password is not kept in the field once it is on its way, and a retry is never typed in clear.
            self.password_shown = false;
            self.password.update(cx, |i, cx| {
                i.set_value("", window, cx);
                i.set_masked(true, window, cx);
            });
        }
        self.run(cmds);
        cx.notify();
    }

    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cmds = self.model.sign_out();
        self.run(cmds);
        self.side = sidebar::Side::default();
        self.banner = Default::default();
        self.person = None;
        self.tl = chat::Tl::default();
        for (_, i) in self.images.drain() {
            if let chat::Img::Ready(i) = i {
                let _ = window.drop_image(i);
            }
        }
        self.image_order.clear();
        self.composer
            .update(cx, |i, cx| i.set_value("", window, cx));
        cx.notify();
    }

    fn open_room(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let was = self.model.active.filter(|_| !self.settings_open);
        self.settings_open = false;
        if was.is_some_and(|w| w != id) {
            self.side.glide_from(was, theme::tokens(cx).motion.reduced);
        }
        let cmds = self.model.open_room(id);
        self.run(cmds);
        self.composer.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    fn send(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        if self.pick_active(window, cx) {
            return;
        }
        let cmds = self.model.send(&self.composer.read(cx).value());
        if !cmds.is_empty() {
            self.composer
                .update(cx, |i, cx| i.set_value("", window, cx));
        }
        self.run(cmds);
        cx.notify();
    }

    fn new_line(&mut self, _: &NewLine, window: &mut Window, cx: &mut Context<Self>) {
        self.composer.update(cx, |i, cx| i.insert("\n", window, cx));
    }

    /// The message list, aligned at the top (all of it fits) or the bottom.
    fn new_list(top: bool, cx: &mut Context<Self>) -> ListState {
        let align = if top {
            ListAlignment::Top
        } else {
            ListAlignment::Bottom
        };
        let list = ListState::new(0, align, px(400.));
        let this = cx.weak_entity();
        list.set_scroll_handler(move |e, _, cx| {
            // The list is borrowed while this runs: only the model changes here, the rows update on the next render.
            // A list that fits is always at its bottom.
            let (start, bottom) = (e.visible_range.start == 0, top || !e.is_scrolled);
            let _ = this.update(cx, |v, cx| {
                let cmds = v.model.scrolled(start, bottom);
                v.run(cmds);
                cx.notify();
            });
        });
        list
    }

    /// Short rooms start at the top, as on the web: the list turns top aligned when the last frame's rows fit, and
    /// bottom aligned again when they no longer do (one frame late, like a scroll).
    fn fit_list(&mut self, cx: &mut Context<Self>) {
        let n = self.list.item_count();
        if n == 0 || self.model.active != self.shown.0 {
            return;
        }
        // The measured rows against the viewport. (bounds_for_item cannot tell: it ignores the bottom alignment.)
        let fits = self.list.max_offset_for_scrollbar().height <= px(0.5)
            && self.list.viewport_bounds().size.height > px(0.);
        let flip = fits != self.list_top;
        if flip {
            self.list_top = !self.list_top;
            self.list = Self::new_list(self.list_top, cx);
            self.list.reset(n);
        }
    }

    /// Tells the list which rows were added or changed since the last render, so it keeps its scroll position, and
    /// scrolls to a message asked for (a reply's quote, a pin) and flashes it. Also moves the timeline's motion on.
    fn sync_list(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let calm = self.side.calm(now);
        self.fit_list(cx);
        self.sync_timeline(now, calm, cx);
        let new_after = self.new_row_after(cx);
        let room = self.model.active;
        let m = &self.model;
        let msgs = m.log().map_or(&[][..], |l| &l.messages[..]);
        self.heads = crate::timeline::heads(
            msgs,
            chrono::Utc::now().timestamp_millis(),
            &chrono::Local,
            crate::timeline::Locale::system(),
            new_after.map(|at| (at, m.state.my_id())),
        );
        let images = &self.images;
        let more_before = m.log().is_some_and(|l| l.more_before);
        let last = msgs.len().saturating_sub(1);
        let rows: Vec<(i64, RowKey)> = msgs
            .iter()
            .zip(&self.heads)
            .enumerate()
            .map(|(i, (x, h))| {
                let shown = x
                    .attachments
                    .iter()
                    .filter(|a| !matches!(images.get(&a.id), None | Some(chat::Img::Loading)))
                    .count();
                let open = [m.editing, m.confirm_delete, m.picking].map(|v| v == Some(x.id));
                let edge = [i == 0 && more_before, i == 0, i == last];
                (
                    x.id,
                    (
                        x.updated_at,
                        [h.head, h.day.is_some(), h.new],
                        open,
                        edge,
                        shown,
                    ),
                )
            })
            .collect();
        if let Some(ix) = m.jump.and_then(|id| msgs.iter().position(|x| x.id == id)) {
            self.list.scroll_to_reveal_item(ix);
            self.tl.flash = Some((msgs[ix].id, now));
        }
        self.model.jump = None;
        if room != self.shown.0 {
            // Another room opens at its bottom; fit_list tops a short one on the next frame.
            if self.list_top {
                self.list_top = false;
                self.list = Self::new_list(false, cx);
            }
            self.list.reset(rows.len());
        } else {
            for (range, count) in splices(&self.shown.1, &rows) {
                self.list.splice(range, count);
            }
        }
        self.shown = (room, rows);
    }

    /// Draws again when the soonest "is typing" line ends.
    fn sync_typing(&mut self, cx: &mut Context<Self>) {
        let due = self.model.typing_deadline(Instant::now());
        if due != self.typing_timer.as_ref().map(|t| t.0) {
            self.typing_timer = due.map(|at| {
                let wait = at.saturating_duration_since(Instant::now());
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |v, cx| {
                        v.typing_timer = None;
                        cx.notify();
                    });
                });
                (at, task)
            });
        }
    }
}

/// What a row's height depends on: `updated_at`, (heads a group, starts a day, has the New row), (editing,
/// confirming a delete, picking a reaction), (has Load older, is first, is last), pictures loaded.
type RowKey = (i64, [bool; 3], [bool; 3], [bool; 3], usize);

/// The list splices that turn rows `old` into `new` (each `(id, look)`, ids ascending): rows put in front (an older
/// page), rows changed in place (an edit or delete), rows added at the end (new messages). Anything else replaces
/// every row.
pub fn splices<T: PartialEq>(old: &[(i64, T)], new: &[(i64, T)]) -> Vec<(Range<usize>, usize)> {
    let all = vec![(0..old.len(), new.len())];
    let Some(first) = old.first() else { return all };
    let Some(front) = new.iter().position(|r| r.0 == first.0) else {
        return all;
    };
    let Some(same) = new.get(front..front + old.len()) else {
        return all;
    };
    if same.iter().zip(old).any(|(a, b)| a.0 != b.0) {
        return all;
    }
    let mut ops = vec![];
    if front > 0 {
        ops.push((0..0, front));
    }
    for (i, (a, b)) in same.iter().zip(old).enumerate() {
        if a.1 != b.1 {
            ops.push((front + i..front + i + 1, 1));
        }
    }
    let end = front + old.len();
    if new.len() > end {
        ops.push((end..end, new.len() - end));
    }
    ops
}

impl Render for Vesper {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.model.screen {
            Screen::Starting => div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(theme::FG_3))
                .child("Vesper"),
            Screen::SignIn => self.render_sign_in(window, cx),
            Screen::Main => self.render_main(window, cx),
        };
        let t = theme::tokens(cx);
        // `body` of app.css: --bg-app, --fg, 13/20.
        div()
            .size_full()
            .bg(t.bg_app)
            .text_color(t.fg)
            .font_family(t.font_sans.clone())
            .text_size(px(theme::TEXT_SM.size))
            .line_height(px(theme::TEXT_SM.line))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, _| {
                this.window_key(Some(&e.keystroke.key), true);
            }))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, _| {
                this.window_key(Some(&e.keystroke.key), false);
            }))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::splices;

    #[test]
    fn splices_for_older_edited_and_new_rows() {
        let old = [(5, 1), (6, 1)];
        assert_eq!(splices(&[], &[(1, 1)]), [(0..0, 1)]);
        assert_eq!(
            splices(&old, &[(3, 1), (4, 1), (5, 1), (6, 1)]),
            [(0..0, 2)]
        );
        assert_eq!(
            splices(&old, &[(5, 1), (6, 2), (7, 1)]),
            [(1..2, 1), (2..2, 1)]
        );
        assert!(splices(&old, &old).is_empty());
        assert_eq!(splices(&old, &[(9, 1)]), [(0..2, 1)], "a gap: replace all");
        assert_eq!(
            splices(&old, &[(4, 1), (5, 1)]),
            [(0..2, 2)],
            "rows gone at the end: replace all"
        );
    }
}

#[cfg(test)]
mod view_tests;
