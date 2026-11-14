//! The window: one view, `Vesper`, that owns the `Model` and draws the sign in screen or the main screen. Events from
//! the network thread come in through `listen`; what the model asks for goes out on `cmds`.

mod room;
mod sign_in;
pub mod theme;

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, AppContext, Application, Context, Entity, IntoElement, KeyBinding, ListAlignment,
    ListState, ParentElement, Render, Styled, Subscription, Task, TitlebarOptions, Window,
    WindowBounds, WindowOptions, actions, div, px, rgb, size,
};
use gpui_component::Root;
use gpui_component::input::{InputEvent, InputState};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::model::{Model, Screen};
use crate::net::{self, Cmd, Event};
use crate::store::Keyring;

actions!(vesper, [SendMessage, NewLine]);

/// The key context of the composer.
const COMPOSER: &str = "Composer";

/// Keys for the app, bound after gpui-component's so they win over its multi-line input.
pub fn init(cx: &mut App) {
    gpui_component::init(cx);
    theme::apply(cx);
    let input = Some("Composer > Input");
    cx.bind_keys([
        KeyBinding::new("enter", SendMessage, input),
        KeyBinding::new("shift-enter", NewLine, input),
    ]);
}

/// Starts the network thread and opens the window. Returns when the app quits.
pub fn run() {
    let (cmds, events) = net::start(Arc::new(Keyring));
    Application::new().run(move |cx: &mut App| {
        init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("Vesper".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            let view = cx.new(|cx| {
                let mut v = Vesper::new(cmds, window, cx);
                v.listen(events, window, cx);
                v
            });
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("the window opens");
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
    composer: Entity<InputState>,
    list: ListState,
    /// What the list shows: the room and each row's (id, updated_at), to tell the list which rows changed.
    shown: (Option<i64>, Vec<(i64, i64)>),
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
                .auto_grow(1, 8)
                .placeholder("Write a message")
        });
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
            cx.observe_window_activation(window, |this, window, cx| {
                let cmds = this.model.set_focused(window.is_window_active());
                this.run(cmds);
                cx.notify();
            }),
        ];
        let list = ListState::new(0, ListAlignment::Bottom, px(400.));
        let this = cx.weak_entity();
        list.set_scroll_handler(move |e, _, cx| {
            // The list is borrowed while this runs: only the model changes here, the rows update on the next render.
            let (top, bottom) = (e.visible_range.start == 0, !e.is_scrolled);
            let _ = this.update(cx, |v, cx| {
                let cmds = v.model.scrolled(top, bottom);
                v.run(cmds);
                cx.notify();
            });
        });
        let _ = cmds.send(Cmd::Restore); // the first thing the app asks for: the stored session
        Vesper {
            model: Model::default(),
            cmds,
            server,
            username,
            password,
            composer,
            list,
            shown: (None, vec![]),
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

    /// One event into the model; what it asks for goes to the network.
    pub fn apply(&mut self, ev: Event, window: &mut Window, cx: &mut Context<Self>) {
        let signed_out = matches!(ev, Event::SignedOut { .. });
        let cmds = self.model.apply(ev);
        self.run(cmds);
        if signed_out && self.server.read(cx).value().is_empty() && !self.model.server.is_empty() {
            let s = self.model.server.clone();
            self.server.update(cx, |i, cx| i.set_value(s, window, cx));
        }
        if let Some(text) = self.model.draft_back.take()
            && self.composer.read(cx).value().is_empty()
        {
            self.composer
                .update(cx, |i, cx| i.set_value(text, window, cx));
        }
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
            // The password is not kept in the field once it is on its way.
            self.password
                .update(cx, |i, cx| i.set_value("", window, cx));
        }
        self.run(cmds);
        cx.notify();
    }

    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cmds = self.model.sign_out();
        self.run(cmds);
        self.composer
            .update(cx, |i, cx| i.set_value("", window, cx));
        cx.notify();
    }

    fn open_room(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let cmds = self.model.open_room(id);
        self.run(cmds);
        self.composer.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    fn send(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Tells the list which rows were added or changed since the last render, so it keeps its scroll position.
    fn sync_list(&mut self) {
        let room = self.model.active;
        let rows: Vec<(i64, i64)> = self
            .model
            .log()
            .map(|l| l.messages.iter().map(|m| (m.id, m.updated_at)).collect())
            .unwrap_or_default();
        if room != self.shown.0 {
            self.list.reset(rows.len());
        } else {
            for (range, count) in splices(&self.shown.1, &rows) {
                self.list.splice(range, count);
            }
        }
        self.shown = (room, rows);
    }
}

/// The list splices that turn rows `old` into `new` (each `(id, updated_at)`, ids ascending): rows put in front
/// (an older page), rows changed in place (an edit or delete), rows added at the end (new messages). Anything else
/// replaces every row.
pub fn splices(old: &[(i64, i64)], new: &[(i64, i64)]) -> Vec<(Range<usize>, usize)> {
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
        div()
            .size_full()
            .bg(rgb(theme::BG_APP))
            .text_color(rgb(theme::FG))
            .text_size(px(14.))
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
