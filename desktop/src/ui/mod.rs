//! The window: one view, `Vesper`, that owns the `Model` and draws the sign in screen or the main screen. Events from
//! the network thread come in through `listen`; what the model asks for goes out on `cmds`.

pub mod account;
mod admin;
mod banner;
pub mod c;
mod call;
mod chat;
mod chrome;
#[cfg(debug_assertions)]
mod dev;
mod find;
pub mod icons;
mod md;
pub mod motion;
mod person;
mod popout;
mod room;
mod select;
mod send;
pub mod settings;
mod share;
mod sidebar;
mod sign_in;
mod sound;
pub mod theme;
pub mod tray;
mod viewer;

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Application, Bounds, Context, Entity, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, KeyUpEvent, ListAlignment,
    ListState, ParentElement, Pixels, Render, Styled, Subscription, Task, TitlebarOptions, Window,
    WindowBounds, WindowOptions, actions, div, point, px, rgb, size,
};
use gpui_component::Root;
use gpui_component::input::{InputEvent, InputState};
use gpui_component::slider::SliderState;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::model::{Model, Phase, Screen};
use crate::net::{self, Cmd, Event};
use crate::store::Keyring;
use crate::voice::Cpal;
use crate::voice::ptt::{self, KeyMsg, KeyOut};

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
        PasteFile,
        Dismiss,
        CopySelection,
        SelectAllText,
        ConfirmDialog,
        FindUp,
        FindDown,
        FindEnter,
        FindTab,
        FindHome,
        FindEnd,
        FindBack,
        ViewerPrev,
        ViewerNext
    ]
);

/// The key context of the composer.
const COMPOSER: &str = "Composer";
/// The key context of a message's edit box.
const EDIT_BOX: &str = "EditBox";
/// The key context of an open dialog, which holds the focus.
const DIALOG: &str = "Dialog";

/// Keys for the app, bound after gpui-component's so they win over its multi-line input.
pub fn init(cx: &mut App) {
    gpui_component::init(cx);
    theme::apply(cx);
    let input = Some("Composer > Input");
    let edit = Some("EditBox > Input");
    let find_input = Some("Find > Input");
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
        // Esc anywhere closes the topmost thing (an input's own Esc goes first, and passes it on when it has nothing
        // to close).
        KeyBinding::new("escape", Dismiss, None),
        // Selected message text (select.rs). A focused field's own Ctrl+C comes first.
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-c", CopySelection, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-c", CopySelection, None),
        // Every loaded message (select.rs). The handler passes it on to a focused field's own Ctrl+A.
        #[cfg(target_os = "macos")]
        KeyBinding::new("cmd-a", SelectAllText, None),
        #[cfg(not(target_os = "macos"))]
        KeyBinding::new("ctrl-a", SelectAllText, None),
        KeyBinding::new("enter", ConfirmDialog, Some(DIALOG)),
        // The switcher's and search's field: the rows move, the field keeps the focus.
        KeyBinding::new("up", FindUp, find_input),
        KeyBinding::new("down", FindDown, find_input),
        KeyBinding::new("enter", FindEnter, find_input),
        KeyBinding::new("tab", FindTab, find_input),
        KeyBinding::new("home", FindHome, find_input),
        KeyBinding::new("end", FindEnd, find_input),
        KeyBinding::new("backspace", FindBack, find_input),
        KeyBinding::new("escape", Dismiss, find_input),
        KeyBinding::new("left", ViewerPrev, Some(viewer::VIEWER)),
        KeyBinding::new("right", ViewerNext, Some(viewer::VIEWER)),
    ]);
}

/// Starts the network and key threads and opens the window. Returns when the app quits.
pub fn run() {
    use crate::single::Claim;
    let link = crate::single::link_arg();
    let updated = std::env::args().any(|a| a == crate::update::UPDATED_ARG);
    let mut tries = 0;
    let single = loop {
        match crate::single::claim(link.as_deref()) {
            // Just updated: the old app is still quitting. Wait for it (up to 10 s), never hand over to it.
            Claim::First(None) | Claim::Second if updated && tries < 50 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Claim::First(l) => break l,
            Claim::Second => return, // another Vesper was running: it shows itself and opens the link
        }
    };
    if let Some(dir) = crate::logfile::dir() {
        crate::logfile::start(&dir);
    }
    let audio: Arc<dyn crate::voice::AudioDevices> = Arc::new(Cpal);
    // The parity harness's calls run on fake devices (dev.rs).
    #[cfg(debug_assertions)]
    let audio = dev::audio().unwrap_or(audio);
    let (cmds, events) = net::start(Arc::new(Keyring), audio.clone());
    crate::files::clean_pasted();
    let file = crate::settings::path();
    let saved = file
        .as_deref()
        .map(crate::settings::Settings::load)
        .unwrap_or_default();
    let (open_tx, open_rx) = tokio::sync::mpsc::unbounded_channel();
    let keys = ptt::spawn(ptt::global_source, move |out| {
        let _ = open_tx.send(out);
    });
    // Started by the computer with "Start in the tray" on.
    let in_tray = std::env::args().any(|a| a == crate::settings::IN_TRAY) && saved.start_in_tray;
    let (click_tx, mut clicks) = tokio::sync::mpsc::unbounded_channel();
    let os = crate::notify::Os::start(move |room| {
        let _ = click_tx.send(room);
    });
    let app = Application::new().with_assets(icons::Assets);
    app.run(move |cx: &mut App| {
        init(cx);
        let screens: Vec<_> = cx.displays().iter().map(|d| d.bounds()).collect();
        let window_bounds = match saved.window {
            Some(w) => {
                let (at, wh) = (point(px(w.x), px(w.y)), size(px(w.w), px(w.h)));
                let b = Bounds::new(at, wh);
                let b = if on_a_screen(b, &screens) {
                    b
                } else {
                    Bounds::centered(None, wh, cx)
                };
                if w.maximized {
                    WindowBounds::Maximized(b)
                } else {
                    WindowBounds::Windowed(b)
                }
            }
            None => WindowBounds::centered(size(px(1100.), px(720.)), cx),
        };
        let options = WindowOptions {
            window_bounds: Some(window_bounds),
            // The app draws its own window buttons (chrome.rs): no system title bar on Windows, client-side
            // decorations on Linux. macOS keeps its title bar.
            titlebar: Some(TitlebarOptions {
                title: Some("Vesper".into()),
                appears_transparent: cfg!(windows),
                ..Default::default()
            }),
            window_decorations: cfg!(target_os = "linux")
                .then_some(gpui::WindowDecorations::Client),
            // Wayland's activation (the tray's Show Vesper) needs it, and the taskbar matches the window by it.
            app_id: Some("vesper".into()),
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
                    v.os = Some(os);
                    v.watch_key();
                    if let Some(l) = &link {
                        v.open_link(l, window, cx);
                    }
                    // Its place and size are kept for the next start.
                    cx.observe_window_bounds(window, |v, window, cx| {
                        v.model.settings.window = Some(saved_window(window.window_bounds()));
                        v.save_settings_soon(cx);
                    })
                    .detach();
                    v
                });
                if in_tray {
                    view.update(cx, |v, cx| v.minimised_by_close(cx));
                    window.minimize_window();
                }
                // The close button goes to the tray (minimised: GPUI 0.2.2 cannot hide a window on Windows) or quits,
                // as Settings > App says. By default it quits unless a call runs, which keeps going in the tray.
                let weak = view.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    let view = weak.upgrade().filter(|v| {
                        let m = &v.read(cx).model;
                        match m.settings.on_close {
                            crate::settings::OnClose::Tray => true,
                            crate::settings::OnClose::Quit => false,
                            crate::settings::OnClose::Call => m.call.phase != Phase::Idle,
                        }
                    });
                    if let Some(v) = &view {
                        v.update(cx, |v, cx| v.minimised_by_close(cx));
                        window.minimize_window();
                    }
                    view.is_none()
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
            view.update(cx, |v, _| (v._tray, v.audio) = (tray, Some(audio)));
            // A release build asks for a newer version at start and every few hours (update.rs).
            if !cfg!(debug_assertions) {
                let weak = view.downgrade();
                cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                    while weak
                        .update(cx, |v, _| {
                            if v.model.settings.check_updates {
                                v.run(vec![net::Cmd::CheckUpdate]);
                            }
                        })
                        .is_ok()
                    {
                        cx.background_executor().timer(crate::update::EVERY).await;
                    }
                })
                .detach();
            }
        }
        // A clicked notification opens its room in the window, brought to the front.
        let window: gpui::AnyWindowHandle = handle.into();
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            while let Some(room) = clicks.recv().await {
                let _ = cx.update_window(window, |root, window, cx| {
                    window.activate_window();
                    cx.activate(true);
                    if let Ok(view) = root.downcast::<Root>()
                        && let Ok(v) = view.read(cx).view().clone().downcast::<Vesper>()
                    {
                        v.update(cx, |v, cx| {
                            if v.model.state.find_room(room).is_some() {
                                v.open_room(room, window, cx);
                            }
                        });
                    }
                });
            }
        })
        .detach();
        // A second launch asks this one to come to the front, as the tray's Show Vesper does.
        if let Some(l) = single {
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            l.serve(move |link| {
                let _ = tx.send(link);
            });
            let vesper = handle
                .read(cx)
                .ok()
                .and_then(|r| r.view().clone().downcast::<Vesper>().ok())
                .map(|v| v.downgrade());
            let window: gpui::AnyWindowHandle = handle.into();
            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                while let Some(link) = rx.recv().await {
                    let _ = cx.update_window(window, |_, window, cx| {
                        window.activate_window();
                        cx.activate(true);
                        if let (Some(l), Some(v)) = (link, vesper.as_ref()) {
                            let _ = v.update(cx, |v, cx| v.open_link(&l, window, cx));
                        }
                    });
                }
            })
            .detach();
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
    /// Create your account's display name (a `vesper://invite` link).
    display_name: Entity<InputState>,
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
    /// A press on a top bar's empty space: the next move drags the window (`chrome.rs`).
    drag_armed: bool,
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
    /// Choosing a key: the next key pressed is it. For push to talk (0), mute (1) or deafen (2).
    pub capturing_key: bool,
    pub capture_for: usize,
    key_problem: &'static str,
    key_focus: FocusHandle,
    /// Held while text is selected, so Ctrl+C copies it (select.rs).
    text_focus: FocusHandle,
    settings_error: String,
    /// The Settings and Admin pages' own state.
    page: settings::Page,
    /// To the key thread (none in tests).
    pub ptt_tx: Option<Sender<KeyMsg>>,
    /// A settings save waiting for a slider to stop.
    save_later: Option<Task<()>>,
    /// The key the key thread was last told to watch.
    watched: std::cell::Cell<Option<Option<device_query::Keycode>>>,
    /// The mute and deafen keys the key thread was last told to watch.
    watched_toggles: std::cell::Cell<[Option<device_query::Keycode>; 2]>,
    /// Notifications and the launcher's count (none in tests), and the unread count shown.
    os: Option<crate::notify::Os>,
    badge: u32,
    /// Why push to talk works only while the window has focus.
    pub ptt_note: Option<&'static str>,
    /// The share panel's pictures and state.
    share: share::ShareView,
    /// Sends the next `watch off` when it is due.
    watch_timer: Option<(Instant, Task<()>)>,
    _tray: Option<tray_icon::TrayIcon>,
    /// The sound devices, for the ring (none in tests).
    audio: Option<Arc<dyn crate::voice::AudioDevices>>,
    /// The ring sound repeating, for the ring shown.
    ring_sound: Option<(i64, Task<()>)>,
    /// The window was minimised by its close button, so nothing of it is on screen until it is active again.
    minimised: bool,
    /// Frames drawn, and the check that a picture waiting untaken means the window is not drawn.
    draws: u64,
    frame_check: Option<Task<()>>,
    /// Holds the focus while a dialog is open.
    dialog_focus: FocusHandle,
    /// The composer's text of the other rooms, and a change that only put a draft back.
    drafts: HashMap<i64, String>,
    draft_put: bool,
    /// Draws again at midnight, so the day labels move on.
    _midnight: Task<()>,
    /// The sidebar's menu, folds and motion.
    side: sidebar::Side,
    /// The timeline's motion, New row and open menus.
    tl: chat::Tl,
    /// The room whose name the composer's placeholder has.
    placeholder_room: Option<i64>,
    /// Since when the call bar shows, for its rise and the call timer.
    bar_since: Option<Instant>,
    /// The call bar's pill, peek, morph and timer.
    bar: call::Bar,
    /// The open person card, the opener a press outside closed it on (and when), and where openers were drawn.
    person: Option<person::Card>,
    person_closed: Option<(String, Instant)>,
    anchors: person::Anchors,
    /// The connection banner and the ring prompt's motion.
    banner: banner::Banner,
    /// The switcher, search or shortcuts layer, and its fields.
    find: find::Find,
    /// The open image viewer.
    viewer: Option<viewer::Viewer>,
    /// The open share dialog.
    send: Option<send::Dialog>,
    /// The screen or window last chosen there, for the next share (until the app quits).
    share_source: Option<crate::voice::capture::Source>,
    /// The window of the share popped out, while it is open.
    pop: Option<Entity<popout::PopOut>>,
    find_field: Entity<InputState>,
    after_field: Entity<InputState>,
    before_field: Entity<InputState>,
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
        let display_name = cx.new(|cx| InputState::new(window, cx));
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
            match e {
                InputEvent::PressEnter { .. } => this.submit_sign_in(window, cx),
                InputEvent::Change => cx.notify(), // the plain http warning follows the address
                _ => {}
            }
        };
        let mut subs = vec![
            cx.subscribe_in(&server, window, enter),
            cx.subscribe_in(&username, window, enter),
            cx.subscribe_in(&display_name, window, enter),
            cx.subscribe_in(&password, window, enter),
            cx.subscribe_in(&composer, window, |this, _, e: &InputEvent, _, cx| {
                if matches!(e, InputEvent::Change) {
                    this.composer_changed(cx);
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                if active {
                    this.minimised = false;
                } else {
                    // The key-up never comes once focus is gone: a window-held key would leave the mic open.
                    this.window_key(None, false);
                }
                this.send_typing(window, cx);
                let cmds = this.model.set_focused(active);
                this.run(cmds);
                cx.notify();
            }),
        ];
        // Push to talk ignores a press while one of these has focus.
        for i in [
            &server,
            &username,
            &display_name,
            &password,
            &composer,
            &edit,
        ] {
            let f = i.read(cx).focus_handle(cx);
            subs.push(cx.on_focus(&f, window, |this, w, cx| this.send_typing(w, cx)));
            subs.push(cx.on_blur(&f, window, |this, w, cx| this.send_typing(w, cx)));
        }
        let find_field = cx.new(|cx| InputState::new(window, cx));
        let after_field = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD"));
        let before_field = cx.new(|cx| InputState::new(window, cx).placeholder("YYYY-MM-DD"));
        subs.push(cx.subscribe_in(
            &find_field,
            window,
            |this, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::Change) {
                    this.find_changed(window, cx);
                }
            },
        ));
        for (f, key) in [
            (&after_field, crate::searchq::Key::After),
            (&before_field, crate::searchq::Key::Before),
        ] {
            subs.push(
                cx.subscribe_in(f, window, move |this, _, e: &InputEvent, window, cx| {
                    if matches!(e, InputEvent::Change) {
                        this.date_changed(key, window, cx);
                    }
                }),
            );
        }
        for i in [&find_field, &after_field, &before_field] {
            let f = i.read(cx).focus_handle(cx);
            subs.push(cx.on_focus(&f, window, |this, w, cx| this.send_typing(w, cx)));
            subs.push(cx.on_blur(&f, window, |this, w, cx| this.send_typing(w, cx)));
        }
        let list = Self::new_list(false, cx);
        let _ = cmds.send(Cmd::Restore); // the first thing the app asks for: the stored session
        Vesper {
            model: Model::default(),
            cmds,
            server,
            username,
            display_name,
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
            drag_armed: false,
            mention: None,
            mention_ix: 0,
            mention_dismissed: None,
            typing_timer: None,
            sliders: HashMap::new(),
            settings_open: false,
            settings_file: None,
            devices: (vec![], vec![]),
            capturing_key: false,
            capture_for: 0,
            key_problem: "",
            key_focus: cx.focus_handle(),
            text_focus: {
                let f = cx.focus_handle();
                cx.set_global(select::TextFocus(f.clone()));
                f
            },
            settings_error: String::new(),
            page: Default::default(),
            ptt_tx: None,
            watched: Default::default(),
            watched_toggles: Default::default(),
            os: None,
            badge: 0,
            save_later: None,
            ptt_note: None,
            share: Default::default(),
            watch_timer: None,
            _tray: None,
            audio: None,
            ring_sound: None,
            minimised: false,
            draws: 0,
            frame_check: None,
            dialog_focus: cx.focus_handle(),
            drafts: HashMap::new(),
            draft_put: false,
            _midnight: midnight(cx),
            side: sidebar::Side::default(),
            tl: chat::Tl::default(),
            placeholder_room: None,
            bar_since: None,
            bar: Default::default(),
            person: None,
            person_closed: None,
            anchors: Default::default(),
            banner: Default::default(),
            find: Default::default(),
            viewer: None,
            send: None,
            share_source: None,
            pop: None,
            find_field,
            after_field,
            before_field,
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

    /// The key thread's push to talk, as it opens and closes, and its mute and deafen keys.
    pub fn listen_ptt(
        &mut self,
        mut open: UnboundedReceiver<KeyOut>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            while let Some(o) = open.recv().await {
                let r = this.update(cx, |v, cx| match o {
                    KeyOut::Ptt(open) => v.act(|m| m.set_ptt(open), cx),
                    KeyOut::Toggle(0) => v.act(Model::toggle_mute, cx),
                    KeyOut::Toggle(_) => v.act(Model::toggle_deafen, cx),
                });
                if r.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// A key down or up in the window: the push-to-talk, mute and deafen keys go to the key thread (all there is on
    /// Wayland). `None` releases them whatever they were.
    fn window_key(&self, key: Option<&str>, down: bool) {
        let Some(tx) = &self.ptt_tx else { return };
        let s = &self.model.settings;
        let name = key.map(ptt::key_from_window);
        let is = |k: &str| name.as_ref().is_none_or(|n| n.as_deref() == Ok(k));
        if s.push_to_talk && is(&s.ptt_key) {
            let _ = tx.send(KeyMsg::Window(down));
        }
        for (i, k) in [&s.mute_key, &s.deafen_key].into_iter().enumerate() {
            if k.as_deref().is_some_and(is) {
                let _ = tx.send(KeyMsg::WindowToggle(i, down));
            }
        }
    }

    /// Tells the key thread whether a text input in the active window has focus.
    fn send_typing(&self, window: &Window, cx: &App) {
        let typing = window.is_window_active() && self.typing(window, cx);
        if let Some(tx) = &self.ptt_tx {
            let _ = tx.send(KeyMsg::Typing(typing));
        }
    }

    /// One event into the model; what it asks for goes to the network.
    pub fn apply(&mut self, ev: Event, window: &mut Window, cx: &mut Context<Self>) {
        let signed_out = matches!(ev, Event::SignedOut { .. });
        #[cfg(debug_assertions)]
        let latest = matches!(ev, Event::Latest { .. });
        // A late answer of a session that ended is not shown.
        let main = self.model.screen == Screen::Main;
        if let Event::Http { tag, result } = ev {
            if main {
                self.http_answer(&tag, result, window, cx);
            }
            return;
        }
        if let Event::Server(m) = &ev {
            self.notify(m);
        }
        log_event(&ev);
        match &ev {
            Event::Image { id, picture } if main => self.image_loaded(id, picture.clone(), window),
            Event::Avatar {
                user_id,
                version,
                picture,
            } => {
                let image = picture.clone().and_then(chat::render_image);
                cx.default_global::<c::Pictures>()
                    .loaded(user_id, version, image);
            }
            Event::Downloaded { path, open: true } if main => chat::open_file(path, cx),
            Event::Latest { room, .. } => drop(self.tl.loaded.insert(*room)),
            Event::Searched { seq, page } => self.searched(*seq, page.clone(), cx),
            Event::Full { id, picture } => self.full_loaded(id, picture.clone(), window),
            // The new build is in place: it starts, waits for this one to quit, and takes over.
            Event::UpdateInstalled(path) if crate::update::restart(path).is_ok() => {
                cx.quit();
                return;
            }
            _ => {}
        }
        let editing = self.model.editing;
        let cmds = self.model.apply(ev);
        self.run(cmds);
        self.sync_pictures(window, cx);
        if main && self.model.screen != Screen::Main {
            self.reset_view(window, cx);
        }
        self.sync_ring_sound(cx);
        if !self.model.call.frames.is_empty() {
            self.frame_came(cx);
        }
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
        self.sync_look(window, cx);
        self.sync_badge(window);
        #[cfg(debug_assertions)]
        self.dev_step(latest, window, cx);
        cx.notify();
    }

    /// A system notification for a message or a DM call, when the settings want one (`notify.rs`).
    fn notify(&self, m: &vesper_protocol::ServerMsg) {
        use vesper_protocol::ServerMsg;
        let Some(os) = &self.os else { return };
        let m_ = &self.model;
        let seen = m_.focused && !self.minimised;
        let looking_at = m_.active.filter(|_| seen && !self.settings_open);
        let toast = match m {
            ServerMsg::MessageNew { message } => {
                crate::notify::for_message(&m_.state, message, looking_at, &m_.settings)
            }
            ServerMsg::RingStart { room_id, user_id }
                if !m_.rings.iter().any(|r| r.0 == *room_id) =>
            {
                crate::notify::for_ring(&m_.state, *room_id, user_id, seen, &m_.settings)
            }
            _ => None,
        };
        if let Some(t) = toast {
            os.show(t);
        }
    }

    /// The unread count on the window title, the tray icon and the Windows taskbar button (a dot), and the Linux
    /// launcher, when it changed.
    fn sync_badge(&mut self, window: &mut Window) {
        let n = if self.model.screen == Screen::Main {
            crate::notify::badge(&self.model.state)
        } else {
            0
        };
        if n == self.badge {
            return;
        }
        let was = std::mem::replace(&mut self.badge, n);
        window.set_window_title(&crate::notify::title(n));
        if (was > 0) != (n > 0) {
            if let Some(t) = &self._tray {
                tray::set_unread(t, n > 0);
            }
            let dot = tray::badge(32);
            window.set_overlay_icon((n > 0).then_some((dot.as_slice(), 32)), "Unread messages");
        }
        if let Some(os) = &self.os {
            os.count(n);
        }
    }

    /// Profile pictures follow the users: a new or changed one is fetched, an old one let go.
    fn sync_pictures(&mut self, window: &mut Window, cx: &mut App) {
        let users = self.model.state.users.values();
        let (fetch, gone) = cx
            .default_global::<c::Pictures>()
            .sync(users.map(|u| (u.user_id.as_str(), u.avatar.as_deref())));
        for i in gone {
            let _ = window.drop_image(i);
        }
        let cmds = fetch
            .into_iter()
            .map(|(user_id, version)| Cmd::FetchAvatar { user_id, version });
        self.run(cmds.collect());
    }

    fn run(&self, cmds: Vec<Cmd>) {
        for c in cmds {
            let _ = self.cmds.send(c);
        }
        self.watch_key(); // a call started or ended
    }

    fn submit_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (server, user, pass) = (
            self.server.read(cx).value(),
            self.username.read(cx).value(),
            self.password.read(cx).value(),
        );
        let cmds = if self.model.invite.is_some() {
            let name = self.display_name.read(cx).value();
            self.model.sign_up(&server, &user, &name, &pass)
        } else {
            self.model.sign_in(&server, &user, &pass)
        };
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

    /// A `vesper://` link, from this launch or a later one: a room opens (now or once the rooms are known), an invite
    /// turns the sign in screen into Create your account.
    pub fn open_link(&mut self, link: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = crate::single::link(link) else {
            return;
        };
        if let Some(id) = self.model.open_link(link) {
            self.open_room(id, window, cx);
        } else if self.model.invite.is_some() {
            let s = self.model.server.clone();
            self.server.update(cx, |i, cx| i.set_value(s, window, cx));
        }
        cx.notify();
    }

    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cmds = self.model.sign_out();
        self.run(cmds);
        self.reset_view(window, cx);
    }

    /// What the view holds of a session, gone when it ends (signed out, or the server ended it).
    fn reset_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.drafts.clear();
        self.settings_open = false;
        let cmds = self.model.set_mic_open(false);
        self.run(cmds);
        self.page = Default::default();
        self.side = sidebar::Side::default();
        self.banner = Default::default();
        self.person = None;
        self.find = Default::default();
        self.close_viewer(window, cx);
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
        self.show_settings(false);
        if was.is_some_and(|w| w != id) {
            self.side.glide_from(was, theme::tokens(cx).motion.reduced);
        }
        // Each room keeps its own draft, like the web.
        if let Some(old) = self.model.active.filter(|&a| a != id) {
            let text = self.composer.read(cx).value().to_string();
            let new = self.drafts.remove(&id).unwrap_or_default();
            if !text.is_empty() {
                self.drafts.insert(old, text);
            }
            self.draft_put = true;
            self.composer
                .update(cx, |i, cx| i.set_value(new, window, cx));
        }
        crate::switcher::push_recent(&mut self.find.recent, id);
        if self.model.active != Some(id) {
            self.close_viewer(window, cx);
        }
        let cmds = self.model.open_room(id);
        self.run(cmds);
        self.composer.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    fn send(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        if end_composition(&self.composer, window, cx) || self.pick_active(window, cx) {
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

    /// Shows or leaves the Settings view. The room behind it is not on screen meanwhile, so it is not marked read.
    fn show_settings(&mut self, open: bool) {
        self.settings_open = open;
        let mut cmds = self.model.set_covered(open);
        let voice = open && self.page.at == settings::At::Settings(settings::Section::Voice);
        cmds.extend(self.model.set_mic_open(voice));
        self.run(cmds);
    }

    /// A dialog is drawn: it holds the focus.
    fn dialog_open(&self) -> bool {
        (self.model.confirm_delete.is_some() && !self.settings_open)
            || self.person.as_ref().is_some_and(|c| c.confirm)
            || self.viewer.is_some()
            || self.send.is_some()
    }

    /// Esc: closes the topmost thing, in the web's order: a dialog, the person card, a menu or popover, the pins,
    /// then declines the ring shown.
    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        if self.share.full.is_some() {
            self.leave_full(window, cx);
            return cx.notify();
        }
        if self.find.layer.is_some() {
            return self.close_layer(window, cx);
        }
        if self.viewer.is_some() {
            return self.close_viewer(window, cx);
        }
        if self.send.is_some() {
            return self.close_share(cx);
        }
        if self.page_escape(window, cx) {
            return;
        }
        let m = &mut self.model;
        if m.confirm_delete.is_some() {
            m.cancel_delete();
        } else if let Some(c) = self.person.as_mut().filter(|c| c.confirm) {
            c.confirm = false;
        } else if self.person.is_some() {
            self.close_person(cx);
        } else if self.side.account_open
            || self.tl.menu.is_some()
            || m.picking.is_some()
            || self.tl.help
        {
            self.side.account_open = false;
            (self.tl.menu, m.picking, self.tl.help) = (None, None, false);
        } else if m.pins.is_some() {
            m.pins = None;
        } else if let Some(room) = self.model.rings.first().map(|r| r.0) {
            let cmds = self.model.decline_ring(room);
            self.run(cmds);
        } else {
            return cx.propagate();
        }
        cx.notify();
    }

    /// Enter in a dialog: its confirm button.
    fn confirm_dialog(&mut self, _: &ConfirmDialog, _: &mut Window, cx: &mut Context<Self>) {
        if self.send.is_some() {
            return self.go_share(cx);
        }
        if let Some(id) = self.model.confirm_delete.filter(|_| !self.settings_open) {
            return self.act(|m| m.delete(id, true), cx);
        }
        if let Some(id) = self
            .person
            .as_ref()
            .filter(|c| c.confirm)
            .map(|c| c.user_id.clone())
        {
            self.act(|m| m.person_act(&id, "disconnect", None), cx);
            self.person = None;
        }
    }

    /// Keeps the ring sound going while a ring shows (every 2.5 s, the web's `RING_EVERY_MS`), keyed on its room so
    /// a second caller does not restart it. The tray's tooltip says who calls, for a window in the tray.
    fn sync_ring_sound(&mut self, cx: &mut Context<Self>) {
        let ring = self.model.rings.first().map(|r| r.0);
        if ring == self.ring_sound.as_ref().map(|r| r.0) {
            return;
        }
        let tip = match self.model.rings.first() {
            Some((_, caller)) => format!("Vesper: {} is calling", self.model.state.name_of(caller)),
            None => "Vesper".into(),
        };
        if let Some(t) = &self._tray {
            let _ = t.set_tooltip(Some(tip));
        }
        let (audio, speakers, volume) = (
            self.audio.clone().filter(|_| self.model.settings.ring),
            self.model.settings.output_device.clone(),
            self.model.settings.volume,
        );
        self.ring_sound = ring.map(|room| {
            let task = cx.spawn(async move |_, cx| {
                loop {
                    if let Some(a) = &audio {
                        sound::play_ring(a.clone(), speakers.clone(), volume);
                    }
                    cx.background_executor().timer(sound::RING_EVERY).await;
                }
            });
            (room, task)
        });
    }

    /// The close button minimised the window during a call: its share view is off screen from now on.
    pub fn minimised_by_close(&mut self, cx: &mut Context<Self>) {
        self.minimised = true;
        self.share_hidden(cx);
    }

    /// A share picture came. If it is still untaken 2 s later with no frame drawn since, the window is not being drawn
    /// (minimised, or covered by a game): the share counts as not shown, so its `watch off` goes out after the grace.
    /// The next draw shows it again.
    pub(super) fn frame_came(&mut self, cx: &mut Context<Self>) {
        if self.frame_check.is_some() {
            return;
        }
        let draws = self.draws;
        self.frame_check = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(2))
                .await;
            let _ = this.update(cx, |v, cx| {
                v.frame_check = None;
                if v.draws == draws && !v.model.call.frames.is_empty() {
                    v.share_hidden(cx);
                }
            });
        }));
    }

    /// The share view is not on screen. No notify: a draw would count it as shown again.
    fn share_hidden(&mut self, cx: &mut Context<Self>) {
        let cmds = self.model.set_share_visible(false, Instant::now());
        self.run(cmds);
        self.sync_watch_timer(cx);
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

/// A line in the log for the events that tell what happened to the connection and the session. No names, no text.
fn log_event(ev: &Event) {
    match ev {
        Event::SignedIn { .. } => log::info!("signed in"),
        Event::SignedOut { .. } => log::info!("signed out"),
        Event::SessionEnded => log::info!("the server ended the session"),
        Event::Disconnected => log::warn!("disconnected"),
        Event::Offline { retry_in } => log::warn!("offline, trying again in {retry_in:?}"),
        Event::Server(m) if matches!(**m, vesper_protocol::ServerMsg::Hello { .. }) => {
            log::info!("connected")
        }
        _ => {}
    }
}

/// A saved window whose top strip is at least 100 by 20 px on one of the screens; else it is moved back on.
fn on_a_screen(b: Bounds<Pixels>, screens: &[Bounds<Pixels>]) -> bool {
    let top = Bounds::new(b.origin, size(b.size.width, px(40.)));
    screens.iter().any(|s| {
        let i = s.intersect(&top);
        i.size.width >= px(100.) && i.size.height >= px(20.)
    })
}

/// The window's bounds as saved: the restored size when maximized.
fn saved_window(b: WindowBounds) -> crate::settings::SavedWindow {
    let (r, maximized) = match b {
        WindowBounds::Windowed(r) => (r, false),
        WindowBounds::Maximized(r) | WindowBounds::Fullscreen(r) => (r, true),
    };
    crate::settings::SavedWindow {
        x: r.origin.x.into(),
        y: r.origin.y.into(),
        w: r.size.width.into(),
        h: r.size.height.into(),
        maximized,
    }
}

/// Enter while an input method composes ends the composition (as the system would), and sends nothing. True then.
fn end_composition(input: &Entity<InputState>, window: &mut Window, cx: &mut App) -> bool {
    use gpui::EntityInputHandler as _;
    input.update(cx, |i, cx| {
        let marked = i.marked_text_range(window, cx).is_some();
        if marked {
            i.unmark_text(window, cx);
        }
        marked
    })
}

/// A task that draws the view again at each local midnight, so "Today" and "Yesterday" move on in an idle window.
fn midnight(cx: &mut Context<Vesper>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        loop {
            let wait = until_midnight(chrono::Local::now());
            cx.background_executor().timer(wait).await;
            if this.update(cx, |_, cx| cx.notify()).is_err() {
                break;
            }
        }
    })
}

/// From `now` to a second past the next local midnight (an hour on a day with no midnight).
fn until_midnight(now: chrono::DateTime<chrono::Local>) -> std::time::Duration {
    use chrono::TimeZone;
    let next = now
        .date_naive()
        .succ_opt()
        .and_then(|d| d.and_hms_opt(0, 0, 1))
        .and_then(|t| chrono::Local.from_local_datetime(&t).earliest());
    next.and_then(|n| (n - now).to_std().ok())
        .unwrap_or(std::time::Duration::from_secs(3600))
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
        // With the texts of the last frame, before they are drawn again.
        self.drag_scroll(window, cx);
        select::Selection::new_frame(cx);
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
        // A dialog holds the focus while open (so Enter is its confirm, not the composer's send), and gives it back.
        if self.dialog_open() {
            self.dialog_focus.focus(window);
        } else if self.dialog_focus.is_focused(window) {
            self.composer.update(cx, |i, cx| i.focus(window, cx));
        }
        let reveal = self.reveal_layer(window, cx);
        let t = theme::tokens(cx);
        // `body` of app.css: --bg-app, --fg, 13/20.
        div()
            .on_action(cx.listener(Self::dismiss))
            .track_focus(&self.text_focus)
            .on_action(cx.listener(|this, _: &CopySelection, _, cx| {
                match this.selected_text(cx) {
                    Some(t) => cx.write_to_clipboard(gpui::ClipboardItem::new_string(t)),
                    // Nothing selected here: a focused field's own Ctrl+C runs (a binding with no context outranks it).
                    None => cx.propagate(),
                }
            }))
            .on_action(cx.listener(|this, _: &SelectAllText, window, cx| {
                // A focused field keeps its own Ctrl+A; the list's (or no one's) focus selects the messages.
                if window.focused(cx).is_some_and(|f| f != this.text_focus) {
                    cx.propagate();
                    return;
                }
                select::Selection::select_all(select::CHAT, cx);
                cx.notify();
            }))
            .on_action(cx.listener(Self::confirm_dialog))
            .on_action(cx.listener(|this, _: &FindUp, _, cx| this.find_move(false, cx)))
            .on_action(cx.listener(|this, _: &FindDown, _, cx| this.find_move(true, cx)))
            .on_action(cx.listener(|this, _: &FindEnter, w, cx| this.find_enter(false, w, cx)))
            .on_action(cx.listener(|this, _: &FindTab, w, cx| this.find_enter(true, w, cx)))
            .on_action(cx.listener(|this, _: &FindHome, _, cx| this.find_edge(false, cx)))
            .on_action(cx.listener(|this, _: &FindEnd, _, cx| this.find_edge(true, cx)))
            .on_action(cx.listener(|this, _: &FindBack, _, cx| this.find_back(cx)))
            .on_action(cx.listener(|this, _: &ViewerPrev, _, cx| this.viewer_go(-1, cx)))
            .on_action(cx.listener(|this, _: &ViewerNext, _, cx| this.viewer_go(1, cx)))
            .capture_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                this.shortcut_key(e, window, cx);
            }))
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
            // Before the main screen there is no top bar: its top strip moves the window.
            .when(self.model.screen != Screen::Main, |d| {
                d.child(
                    self.draggable(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .right(px(16. + chrome::RESERVE))
                            .h(px(48.)),
                        cx,
                    ),
                )
            })
            .children(self.window_controls(window, cx))
            .children(reveal)
    }
}

#[cfg(test)]
mod tests {
    use super::{on_a_screen, saved_window, splices, until_midnight};
    use gpui::{Bounds, WindowBounds, point, px, size};

    #[test]
    fn a_saved_window_off_every_screen_is_not_used() {
        let b =
            |x: f32, y: f32, w: f32, h: f32| Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
        let screens = [b(0., 0., 1920., 1080.), b(1920., 0., 2560., 1440.)];
        assert!(on_a_screen(b(100., 100., 1100., 720.), &screens));
        assert!(
            on_a_screen(b(3000., 1200., 800., 600.), &screens),
            "on the second one"
        );
        assert!(
            !on_a_screen(b(-3000., 100., 800., 600.), &screens),
            "a screen that is gone"
        );
        assert!(
            !on_a_screen(b(-750., 100., 800., 600.), &screens),
            "50 px left on screen"
        );
        assert!(
            !on_a_screen(b(100., 1070., 800., 600.), &screens),
            "its top below the screen"
        );
        assert!(!on_a_screen(b(100., 100., 800., 600.), &[]));
        let s = saved_window(WindowBounds::Maximized(b(10., 20., 800., 600.)));
        assert_eq!(
            (s.x, s.y, s.w, s.h, s.maximized),
            (10., 20., 800., 600., true)
        );
    }

    #[test]
    fn the_day_labels_redraw_just_after_midnight() {
        use chrono::TimeZone;
        let at = |h, m, s| {
            let t = chrono::NaiveDate::from_ymd_opt(2026, 7, 15)
                .unwrap()
                .and_hms_opt(h, m, s)
                .unwrap();
            chrono::Local.from_local_datetime(&t).earliest().unwrap()
        };
        assert_eq!(until_midnight(at(23, 59, 0)).as_secs(), 61);
        assert_eq!(until_midnight(at(0, 0, 1)).as_secs(), 24 * 3600);
    }

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
