//! The Settings and Admin pages, framed like `SettingsPage.svelte` and `AdminPage.svelte`: a 48 px header with the
//! title and Close, then a 200 px section list beside the open section (at most 640 px wide, 800 for Admin), centred.
//!
//! Settings has Voice, Notifications, Appearance, Account and App (the web's Desktop app section less its downloads:
//! start with the computer, start in the tray, what Close does, and Copy diagnostics). Voice holds what the desktop's
//! own audio does (the devices, the input level and its marker, the mic test, the noise filter, echo cancellation, the
//! bitrate, push to talk) and the shortcut list with the global mute and deafen keys. Not here, as the desktop has no
//! such feature yet: the web's noise gate switch (a gate after DeepFilterNet), DeepFilterNet tuning, browser noise
//! suppression, auto gain, DTX, levelling, Web Push, the custom accent colour, adding a passkey, the welcome steps and
//! the web's Diagnostics page. Account is in `account.rs`, Admin in `admin.rs`.

use std::collections::HashMap;
use std::time::Instant;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Bounds, ClickEvent, Context, Div, Entity, InteractiveElement, KeyDownEvent,
    ParentElement, Pixels, Point, Rgba, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Subscription, Window, WindowAppearance, div, point, px,
};
use gpui_component::input::{InputState, RopeExt};
use gpui_component::slider::{SliderEvent, SliderState};
use gpui_component::{h_flex, v_flex};
use reqwest::Method;
use serde_json::Value;
use vesper_protocol::Role;

use super::c::{self, Button, Dialog, Field, Select, Switch, Variant, kbd, radio_list, radio_row};
use super::theme::{self, Scheme, TEXT_BASE, TEXT_SM, TEXT_XL, TEXT_XS, TextStepExt};
use super::{Vesper, account, admin, motion};
use crate::model::{Hint, Phase};
use crate::net::{Cmd, Fail, Json};
use crate::settings::{self, Denoise, Settings};
use crate::voice::ptt::{self, KeyMsg};

pub const APPLIES_NEXT_CALL: &str =
    "Echo cancellation, the noise filter, devices and bitrate apply from your next call.";
pub const CAPTURING: &str = "Press the key to use. Esc cancels.";

/// The mute (1) or deafen (2) key setting.
fn toggle_key(s: &mut Settings, which: usize) -> &mut Option<String> {
    if which == 1 {
        &mut s.mute_key
    } else {
        &mut s.deafen_key
    }
}

/// The Settings sections, in the web's order (`SETTINGS_SECTIONS` less Diagnostics).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Section {
    #[default]
    Voice,
    Notifications,
    Appearance,
    Account,
    Desktop,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Voice,
        Section::Notifications,
        Section::Appearance,
        Section::Account,
        Section::Desktop,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Section::Voice => "Voice",
            Section::Notifications => "Notifications",
            Section::Appearance => "Appearance",
            Section::Account => "Account",
            // The web's "Desktop app" is its download page; here it holds Start with the computer and updates.
            Section::Desktop => "App",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Section::Voice => "voice",
            Section::Notifications => "notifications",
            Section::Appearance => "appearance",
            Section::Account => "account",
            Section::Desktop => "desktop",
        }
    }
    pub fn from_id(id: &str) -> Option<Section> {
        Section::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// Which page covers the room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    Settings(Section),
    Admin(admin::Section),
}

impl Default for At {
    fn default() -> Self {
        At::Settings(Section::Voice)
    }
}

/// The groups of Settings > Voice that have Reset to defaults (`sections.ts` RESETS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Input,
    Noise,
    Quality,
    Ptt,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Group::Input => "Input",
            Group::Noise => "Noise and echo",
            Group::Quality => "Quality",
            Group::Ptt => "Talk mode",
        }
    }
    /// What Reset puts back: never the devices.
    pub fn reset(self, s: &mut Settings) {
        let d = Settings::default();
        match self {
            Group::Input => s.gate_db = d.gate_db,
            Group::Noise => (s.denoise, s.echo_cancel) = (d.denoise, d.echo_cancel),
            Group::Quality => s.bitrate_kbps = d.bitrate_kbps,
            Group::Ptt => s.push_to_talk = d.push_to_talk,
        }
    }
}

/// What the pages keep while open. Reset when the session ends.
#[derive(Default)]
pub struct Page {
    pub at: At,
    /// The open dropdown, by id.
    pub select: Option<SharedString>,
    pub resetting: Option<Group>,
    /// Text fields, made on first draw.
    pub inputs: HashMap<&'static str, Entity<InputState>>,
    pub sliders: HashMap<&'static str, (Entity<SliderState>, Subscription)>,
    pub account: account::State,
    pub admin: admin::State,
    /// The look last painted: scheme, accent.
    look: Option<(Scheme, Option<String>)>,
    /// Where the pointer picked a colour scheme: the next look change spreads from there.
    pub(super) reveal_from: Option<Point<Pixels>>,
    reveal: Option<Reveal>,
    /// Copy diagnostics was pressed.
    pub copied: bool,
}

/// The theme switch (`reveal` in motion.ts): a circle grows from the click in the new page colour over the old look
/// (`--dur-reveal`, ease-out), then the new look is painted under it and it fades out (`--dur-out`, ease-in). The web
/// clips the new page itself to the circle; GPUI cannot draw the window twice, so here the circle is the new colour.
struct Reveal {
    at: Point<Pixels>,
    start: Instant,
    fill: Rgba,
    /// The look to paint once the circle covers the window.
    next: Option<theme::Tokens>,
}

impl Page {
    /// No invite or reset link is held.
    #[cfg(test)]
    pub fn admin_link_gone(&self) -> bool {
        self.admin.links_gone()
    }
}

/// `cx.listener` for a component that reports a `bool`.
pub(super) fn on_bool(
    cx: &mut Context<Vesper>,
    f: impl Fn(&mut Vesper, bool, &mut Window, &mut Context<Vesper>) + 'static,
) -> impl Fn(bool, &mut Window, &mut App) + 'static {
    let l = cx.listener(move |this, on: &bool, w, cx| f(this, *on, w, cx));
    move |on, w, cx| l(&on, w, cx)
}

/// `.sections` of the web pages: 32 px apart, each after the first under a `--line-subtle` rule 24 px above it.
pub(super) fn sections(list: Vec<Div>, cx: &App) -> Div {
    let line = theme::tokens(cx).line_subtle;
    v_flex().gap(px(32.)).children(
        list.into_iter()
            .enumerate()
            .map(|(i, s)| s.when(i > 0, |d| d.pt(px(24.)).border_t_1().border_color(line))),
    )
}

/// A `section`: its parts 16 px apart.
pub(super) fn section() -> Div {
    v_flex().gap(px(16.))
}

/// An `h3`: 14/20, medium, `--fg`, in a 32 px row when `row`.
pub(super) fn h3(text: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    div()
        .text_step(TEXT_BASE)
        .font_weight(theme::tokens(cx).weight_strong)
        .text_color(t.fg)
        .child(text.into())
}

/// A line of 13/20 or 12/16 text in a colour.
pub(super) fn text(s: impl Into<SharedString>, color: impl Into<gpui::Hsla>, small: bool) -> Div {
    div()
        .text_step(if small { TEXT_XS } else { TEXT_SM })
        .text_color(color.into())
        .child(s.into())
}

/// `Meter.svelte`: a bar on a track from -100 to 0 dBFS (`--ok-fg`, `--warn-fg` from -3), the level beside it, and
/// the marker line.
pub(super) fn meter(db: f32, marker: Option<f32>, cx: &App) -> Div {
    let t = theme::tokens(cx);
    let v = db.clamp(-100., 0.).round();
    let share = |x: f32| gpui::relative((x + 100.) / 100.);
    h_flex()
        .gap(px(12.))
        .child(
            div()
                .debug_selector(|| "settings-meter".into())
                .relative()
                .flex()
                .flex_1()
                .h(px(8.))
                .overflow_hidden()
                .rounded(px(4.))
                .bg(t.bg_selected)
                .border_1()
                .border_color(t.line_strong)
                .child(
                    div()
                        .debug_selector(|| "settings-meter-bar".into())
                        .h_full()
                        .w(share(v))
                        .bg(if v >= -3. { t.warn_fg } else { t.ok_fg }),
                )
                .children(marker.map(|m| {
                    div()
                        .absolute()
                        .top_0()
                        .left(share(m.clamp(-100., 0.)))
                        .ml(px(-1.))
                        .w(px(2.))
                        .h_full()
                        .bg(t.fg)
                })),
        )
        .child(
            div()
                .w(px(64.))
                .flex_none()
                .text_right()
                .text_step(TEXT_XS)
                .text_color(t.fg_2)
                .child(if v <= -100. {
                    "Silent".to_string()
                } else {
                    format!("{v} dBFS")
                }),
        )
}

/// The words of a failed request.
pub(super) fn fail_text(f: Fail) -> String {
    f.text().to_string()
}

impl Vesper {
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_page(At::Settings(Section::Voice), window, cx);
    }

    /// Opens a page, or another section of the one open, and loads what it shows.
    pub(super) fn open_page(&mut self, at: At, window: &mut Window, cx: &mut Context<Self>) {
        self.page.at = at;
        self.show_settings(true);
        self.capturing_key = false;
        self.page.select = None;
        match at {
            At::Settings(Section::Voice | Section::Desktop) if self.settings_file.is_some() => {
                // cpal can take a moment to list devices; not on the UI thread.
                let list = cx.background_spawn(async { crate::voice::device_names() });
                cx.spawn_in(window, async move |this, cx| {
                    let names = list.await;
                    let _ = this.update(cx, |v, cx| {
                        v.devices = names;
                        cx.notify();
                    });
                })
                .detach();
            }
            At::Settings(Section::Account) => self.open_account(window, cx),
            At::Admin(s) => self.open_admin(s, window, cx),
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn close_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_settings(false);
        self.capturing_key = false;
        self.page.select = None;
        self.page.resetting = None;
        self.composer.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    /// Esc on a page: a dropdown or a dialog first, then the page itself (Shell.svelte's closePage).
    pub(super) fn page_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.settings_open {
            return false;
        }
        if self.page.select.take().is_none()
            && self.page.resetting.take().is_none()
            && !self.admin_escape()
        {
            self.close_page(window, cx);
        }
        cx.notify();
        true
    }

    /// A JSON request for the pages; the answer comes to [`Vesper::http_answer`] with `tag`.
    pub(super) fn http(
        &self,
        tag: impl Into<String>,
        method: Method,
        path: impl Into<String>,
        body: Option<Value>,
    ) {
        let _ = self.cmds.send(Cmd::Http {
            tag: tag.into(),
            method,
            path: path.into(),
            body: body.map(Json),
        });
    }

    pub(super) fn http_answer(
        &mut self,
        tag: &str,
        result: Result<Json, Fail>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = result.map(|j| j.0);
        if let Some(t) = tag.strip_prefix("account:") {
            self.account_answer(t, result, window, cx);
        } else if let Some(t) = tag.strip_prefix("admin:") {
            self.admin_answer(t, result, cx);
        } else if tag == "accent" {
            self.accent_answer(result);
        }
        self.sync_look(window, cx);
        cx.notify();
    }

    /// A text field of the pages, made the first time it is drawn.
    pub(super) fn page_input(
        &mut self,
        key: &'static str,
        masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        self.page
            .inputs
            .entry(key)
            .or_insert_with(|| cx.new(|cx| InputState::new(window, cx).masked(masked)))
            .clone()
    }

    pub(super) fn input_text(&self, key: &'static str, cx: &App) -> String {
        self.page
            .inputs
            .get(key)
            .map(|i| i.read(cx).value().to_string())
            .unwrap_or_default()
    }

    pub(super) fn set_input(
        &mut self,
        key: &'static str,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let i = self.page_input(key, false, window, cx);
        let text = text.into();
        i.update(cx, |i, cx| {
            i.set_value(text, window, cx);
            // The caret at the start: set before the field has a width, a caret at the end scrolls the text out of view.
            let start = i.text().offset_to_position(0);
            i.set_cursor_position(start, window, cx);
        });
    }

    /// A dropdown whose open state is the page's.
    pub(super) fn select(
        &self,
        id: &'static str,
        options: Vec<SharedString>,
        chosen: Option<usize>,
        width: f32,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Vesper, usize, &mut Window, &mut Context<Vesper>) + 'static,
    ) -> Select {
        self.select_dyn(id.into(), options, chosen, width, cx, pick)
    }

    pub(super) fn select_dyn(
        &self,
        id: SharedString,
        options: Vec<SharedString>,
        chosen: Option<usize>,
        width: f32,
        cx: &mut Context<Self>,
        pick: impl Fn(&mut Vesper, usize, &mut Window, &mut Context<Vesper>) + 'static,
    ) -> Select {
        let open = self.page.select.as_ref() == Some(&id);
        let key = id.clone();
        let toggle = on_bool(cx, move |this, on, _, cx| {
            this.page.select = on.then(|| key.clone());
            cx.notify();
        });
        let pick = cx.listener(move |this, i: &usize, w, cx| {
            pick(this, *i, w, cx);
            cx.notify();
        });
        Select::new(id, options, chosen, open)
            .list_width(px(width))
            .on_toggle(toggle)
            .on_pick(move |i, w, cx| pick(&i, w, cx))
    }

    /// A slider of the pages, made the first time it is drawn; `set` gets each new value.
    pub(super) fn page_slider(
        &mut self,
        key: &'static str,
        (min, max, step): (f32, f32, f32),
        value: f32,
        cx: &mut Context<Self>,
        set: impl Fn(&mut Vesper, f32, &mut Context<Vesper>) + 'static,
    ) -> Entity<SliderState> {
        if !self.page.sliders.contains_key(key) {
            let state = cx.new(|_| {
                SliderState::new()
                    .min(min)
                    .max(max)
                    .step(step)
                    .default_value(value)
            });
            let sub = cx.subscribe(&state, move |this, _, e: &SliderEvent, cx| {
                let SliderEvent::Change(v) = e;
                set(this, v.start(), cx);
            });
            self.page.sliders.insert(key, (state, sub));
        }
        self.page.sliders[key].0.clone()
    }

    /// Changes the settings: into the model (push to talk applies at once), to the key thread, and to the file.
    pub(super) fn change_settings(
        &mut self,
        f: impl FnOnce(&mut Settings),
        cx: &mut Context<Self>,
    ) {
        let mut s = self.model.settings.clone();
        f(&mut s);
        let was = (
            self.model.settings.start_with_computer,
            self.model.settings.start_in_tray,
        );
        let autostart = Some((s.start_with_computer, s.start_in_tray)).filter(|&now| now != was);
        self.settings_error.clear();
        let cmds = self.model.set_settings(s);
        self.run(cmds);
        self.watch_key();
        if let Some((on, in_tray)) = autostart
            && self.settings_file.is_some()
            && let Err(e) = settings::set_start_with_computer(on, in_tray)
        {
            log::warn!("start with the computer: {e}");
            let m = &mut self.model.settings;
            (m.start_with_computer, m.start_in_tray) = was;
            self.settings_error = e;
        }
        self.save_settings();
        cx.notify();
    }

    /// Writes the model's settings to the file, if the app has one (tests do not).
    pub(super) fn save_settings(&mut self) {
        if let Some(path) = &self.settings_file
            && let Err(e) = self.model.settings.save(path)
        {
            self.settings_error = format!("Could not save the settings: {e}");
        }
    }

    /// Saves once a slider stops moving, not at each of its steps.
    pub(super) fn save_settings_soon(&mut self, cx: &mut Context<Self>) {
        self.save_later = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(400))
                .await;
            let _ = this.update(cx, |v, _| v.save_settings());
        }));
    }

    /// Tells the key thread which key to watch, if push to talk is on and a call runs (else it does not poll), when
    /// that changed.
    pub(super) fn watch_key(&self) {
        let s = &self.model.settings;
        let key = (s.push_to_talk && self.model.call.phase != Phase::Idle)
            .then(|| ptt::parse_key(&s.ptt_key))
            .flatten();
        let in_call = self.model.call.phase != Phase::Idle;
        let toggles = [&s.mute_key, &s.deafen_key]
            .map(|k| k.as_deref().filter(|_| in_call).and_then(ptt::parse_key));
        if self.watched_toggles.replace(toggles) != toggles
            && let Some(tx) = &self.ptt_tx
        {
            let _ = tx.send(KeyMsg::Toggles(toggles));
        }
        if self.watched.replace(Some(key)) == Some(key) {
            return;
        }
        if let Some(tx) = &self.ptt_tx {
            let _ = tx.send(KeyMsg::Watch(key));
        }
    }

    /// The box that shows a key and takes a new one on a click: 0 push to talk, 1 mute, 2 deafen.
    fn key_button(&self, which: usize, key: Option<&String>, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let id = [
            "settings-ptt-key",
            "settings-mute-key",
            "settings-deafen-key",
        ][which];
        let on = self.capturing_key && self.capture_for == which;
        div().debug_selector(move || id.into()).child(
            div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(32.))
                .px(px(12.))
                .border_1()
                .border_color(if on { t.accent } else { t.line })
                .rounded(t.radius)
                .bg(t.bg_element)
                .font_weight(t.weight_ui)
                .text_color(if on { t.accent_fg } else { t.fg })
                .hover(|s| s.bg(t.bg_hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    (this.capturing_key, this.key_problem) = (true, "");
                    this.capture_for = which;
                    window.focus(&this.key_focus);
                    cx.notify();
                }))
                .when(on, |d| d.child("Press a key..."))
                .when(!on, |d| match key {
                    Some(k) => d.child("Key:").child(kbd(ptt::key_label(k), cx)),
                    None => d.child("None"),
                }),
        )
    }

    /// A key pressed while choosing the push-to-talk key.
    pub(super) fn capture_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        if !self.capturing_key || e.is_held {
            return;
        }
        cx.stop_propagation();
        let k = &e.keystroke;
        if k.key == "escape" {
            (self.capturing_key, self.key_problem) = (false, "");
        } else if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            self.key_problem = "Pick a key that is not Ctrl, Alt, Shift or Meta.";
        } else {
            match ptt::key_from_window(&k.key) {
                Ok(name) => {
                    (self.capturing_key, self.key_problem) = (false, "");
                    match self.capture_for {
                        0 => self.change_settings(|s| s.ptt_key = name, cx),
                        n => self.change_settings(|s| *toggle_key(s, n) = Some(name), cx),
                    }
                }
                Err(why) => self.key_problem = why,
            }
        }
        cx.notify();
    }

    /// The look from the settings (scheme) and the account (accent), painted when it changed.
    pub(super) fn sync_look(&mut self, window: &Window, cx: &mut App) {
        let s = &self.model.settings;
        let scheme = match s.scheme.as_str() {
            "light" => Scheme::Light,
            "system" => match window.appearance() {
                WindowAppearance::Light | WindowAppearance::VibrantLight => Scheme::Light,
                _ => Scheme::Dark,
            },
            _ => Scheme::Dark,
        };
        let accent = self.my_user().and_then(|u| u.ui_accent.clone());
        let look = (scheme, accent);
        if self.page.look.as_ref() == Some(&look) {
            return;
        }
        let motion = theme::tokens(cx).motion.clone();
        let mut t = theme::Tokens::new(look.0, look.1.as_deref());
        t.motion = motion;
        let new_scheme = self.page.look.as_ref().is_some_and(|l| l.0 != look.0);
        self.page.look = Some(look);
        if let Some(r) = self.page.reveal.as_mut().filter(|r| r.next.is_some()) {
            r.next = Some(t); // changed again while the circle grows: paint the latest at its end
            return;
        }
        let from = self.page.reveal_from.take();
        if let Some(at) = from.filter(|_| new_scheme && !t.motion.reduced) {
            self.page.reveal = Some(Reveal {
                at,
                start: Instant::now(),
                fill: t.bg_app,
                next: Some(t),
            });
            return;
        }
        theme::set(t, cx);
    }

    /// The theme switch's layer over the window while it runs (see [`Reveal`]).
    pub(super) fn reveal_layer(&mut self, window: &mut Window, cx: &mut App) -> Option<Div> {
        let r = self.page.reveal.as_mut()?;
        let m = theme::tokens(cx).motion.clone();
        let now = Instant::now();
        window.request_animation_frame();
        if r.next.is_some() {
            let x = motion::fraction(r.start, now, m.dur_reveal);
            if x < 1. {
                let view = Bounds::new(point(px(0.), px(0.)), window.viewport_size());
                let rad = motion::reveal_radius(&m, r.at, view, x);
                return Some(
                    div()
                        .absolute()
                        .left(r.at.x - rad)
                        .top(r.at.y - rad)
                        .size(rad * 2.)
                        .rounded_full()
                        .bg(r.fill),
                );
            }
            theme::set(r.next.take().expect("checked"), cx);
            r.start = now;
        }
        let y = motion::fraction(r.start, now, m.dur_out);
        if y >= 1. {
            self.page.reveal = None;
            return None;
        }
        let fill = r.fill;
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(fill)
                .opacity(1. - motion::ease(m.ease_in)(y)),
        )
    }

    /// You, as the user list has you (it follows `user_updated`), else as signed in.
    pub(super) fn my_user(&self) -> Option<&vesper_protocol::UserInfo> {
        let st = &self.model.state;
        st.users.get(st.my_id()).or(st.me.as_ref().map(|m| &m.user))
    }

    pub(super) fn my_role(&self) -> Role {
        self.my_user().map_or(Role::Member, |u| u.role)
    }

    fn save_accent(&mut self, v: Option<&str>) {
        let st = &mut self.model.state;
        let id = st.my_id().to_string();
        let prev = st.users.get(&id).and_then(|u| u.ui_accent.clone());
        if let Some(u) = st.users.get_mut(&id) {
            u.ui_accent = v.map(String::from);
        }
        self.page.account.accent_error.clear();
        self.page.account.accent_prev = prev;
        let body = serde_json::json!({ "ui_accent": v.unwrap_or("amber") });
        self.http("accent", Method::PATCH, "/api/me", Some(body));
    }

    fn accent_answer(&mut self, r: Result<Value, Fail>) {
        match r.map(serde_json::from_value::<vesper_protocol::UserInfo>) {
            Ok(Ok(u)) => {
                self.model.state.users.insert(u.user_id.clone(), u);
            }
            Ok(Err(_)) => {}
            Err(f) => {
                // A failed save puts the saved one back.
                let prev = self.page.account.accent_prev.take();
                let st = &mut self.model.state;
                let id = st.my_id().to_string();
                if let Some(u) = st.users.get_mut(&id) {
                    u.ui_accent = prev;
                }
                self.page.account.accent_error = fail_text(f);
            }
        }
    }

    /// The page in the main pane: the header, the section list and the open section.
    pub(super) fn render_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = theme::tokens(cx).clone();
        let at = self.page.at;
        let (title, wide) = match at {
            At::Settings(_) => ("Settings", false),
            At::Admin(_) => ("Admin", true),
        };
        let nav: Vec<(At, &'static str)> = match at {
            At::Settings(_) => Section::ALL
                .into_iter()
                .map(|s| (At::Settings(s), s.name()))
                .collect(),
            At::Admin(_) => admin::sections_for(self.my_role())
                .iter()
                .map(|s| (At::Admin(*s), s.name()))
                .collect(),
        };
        let (heading, body) = match at {
            At::Settings(s) => (
                Some(s.name()),
                match s {
                    Section::Voice => self.render_voice(cx),
                    Section::Notifications => self.render_notifications(cx),
                    Section::Appearance => self.render_appearance(cx),
                    Section::Account => self.render_account(window, cx),
                    Section::Desktop => self.render_desktop(cx),
                },
            ),
            At::Admin(s) => self.render_admin(s, window, cx),
        };
        let header = h_flex()
            .h(px(48.))
            .flex_none()
            .gap(px(12.))
            .pl(px(16.))
            .pr(px(16. + super::chrome::RESERVE))
            .border_b_1()
            .border_color(t.line_subtle)
            .children(self.show_sidebar(cx))
            .child(
                self.draggable(div(), cx)
                    .flex_1()
                    .min_w_0()
                    .text_step(TEXT_BASE)
                    .font_weight(theme::tokens(cx).weight_strong)
                    .child(title),
            )
            .child(
                div().debug_selector(|| "settings-close".into()).child(
                    Button::new("settings-close", "Close")
                        .variant(Variant::Ghost)
                        .on_click(cx.listener(|this, _, window, cx| this.close_page(window, cx))),
                ),
            );
        let (hover, selected) = (t.bg_hover, t.bg_selected);
        // The picked section's fill slides to the next one (`slides` in motion.ts).
        let picked = nav.iter().position(|(to, _)| *to == at).unwrap_or(0);
        let slide = super::motion::Slide::new(
            if matches!(at, At::Settings(_)) {
                "nav-settings"
            } else {
                "nav-admin"
            },
            picked,
            window,
            cx,
        );
        let gliding = slide.gliding();
        let nav = (!nav.is_empty()).then(|| {
            // The gliding fill first, so it paints under the names.
            let list = v_flex().w(px(200.)).flex_none().gap(px(1.));
            slide
                .under(list, t.radius_sm, selected, window, cx)
                .children(nav.into_iter().enumerate().map(|(i, (to, name))| {
                    let current = to == at;
                    let sel = format!("nav-{}", name.to_lowercase().replace(' ', "-"));
                    let fg = t.fg;
                    div()
                        .id(SharedString::from(sel.clone()))
                        .debug_selector(move || sel)
                        .flex()
                        .items_center()
                        .h(px(32.))
                        .px(px(8.))
                        .relative()
                        .rounded(t.radius_sm)
                        .text_color(if current { t.fg } else { t.fg_2 })
                        .when(current && !gliding, |d| d.bg(selected))
                        .when(!current, |d| d.hover(move |s| s.bg(hover).text_color(fg)))
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.open_page(to, window, cx)),
                        )
                        .child(slide.track(i))
                        .child(name)
                }))
        });
        let content = v_flex()
            .flex_1()
            .min_w_0()
            .max_w(px(if wide { 800. } else { 640. }))
            .pb(px(48.))
            .when_some(heading, |d, h| {
                d.child(
                    div()
                        .mb(px(24.))
                        .text_step(TEXT_XL)
                        .font_weight(theme::tokens(cx).weight_strong)
                        .text_color(t.fg)
                        .child(h),
                )
            })
            .child(body);
        v_flex()
            .id("settings")
            .debug_selector(|| "settings".into())
            .track_focus(&self.key_focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.capture_key(e, cx)))
            .flex_1()
            .min_h_0()
            .child(header)
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        h_flex()
                            .items_start()
                            .justify_center()
                            .gap(px(32.))
                            .px(px(16.))
                            .py(px(24.))
                            .children(nav)
                            .child(content),
                    ),
            )
    }

    /// The Reset to defaults dialog of Settings > Voice, or an admin dialog, over the whole window.
    pub(super) fn page_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Dialog> {
        if !self.settings_open {
            return None;
        }
        if let Some(g) = self.page.resetting {
            let name = g.name();
            return Some(
                Dialog::new(format!("Reset {name} to defaults?"))
                    .on_close(cx.listener(|this, _, _, cx| {
                        this.page.resetting = None;
                        cx.notify();
                    }))
                    .child(format!(
                        "Every setting under {name} goes back to its default. Your microphone, your speaker and the \
                         other sections stay as they are."
                    ))
                    .footer(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("reset-cancel", "Cancel")
                                    .variant(Variant::Ghost)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.page.resetting = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div().debug_selector(|| "reset-confirm".into()).child(
                                    Button::new("reset-confirm", "Reset")
                                        .variant(Variant::Danger)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.page.resetting = None;
                                            this.change_settings(|s| g.reset(s), cx);
                                            // The sliders show the new values.
                                            this.page.sliders.clear();
                                        })),
                                ),
                            ),
                    ),
            );
        }
        self.admin_dialog(window, cx)
    }

    /// A group's heading, with Reset to defaults at its right when it has one.
    fn voice_head(&self, name: &'static str, reset: Option<Group>, cx: &mut Context<Self>) -> Div {
        h_flex()
            .h(px(32.))
            .justify_between()
            .gap_4()
            .child(h3(name, cx))
            .children(reset.map(|g| {
                let sel = format!("reset-{}", name.to_lowercase().replace(' ', "-"));
                div().mr(px(-8.)).debug_selector(move || sel).child(
                    Button::new(
                        SharedString::from(format!("reset-{name}")),
                        "Reset to defaults",
                    )
                    .variant(Variant::Ghost)
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.page.resetting = Some(g);
                        cx.notify();
                    })),
                )
            }))
    }

    /// A device dropdown: the system default, then each device by name. A saved name that is not there shows as
    /// missing.
    fn device_select(
        &self,
        id: &'static str,
        names: &[String],
        chosen: Option<&str>,
        cx: &mut Context<Self>,
        set: fn(&mut Settings, Option<String>),
    ) -> Div {
        let options: Vec<SharedString> = std::iter::once("System default".into())
            .chain(names.iter().map(|n| n.clone().into()))
            .collect();
        let at = chosen
            .and_then(|c| names.iter().position(|n| n == c))
            .map_or(0, |i| i + 1);
        let list = names.to_vec();
        let missing = chosen.filter(|c| !names.iter().any(|n| n == c));
        let t = theme::tokens(cx);
        let warn = t.fg_2;
        v_flex()
            .gap(px(6.))
            .child(
                self.select(id, options, Some(at), 640., cx, move |this, i, _, cx| {
                    let name = i.checked_sub(1).and_then(|i| list.get(i).cloned());
                    this.change_settings(|s| set(s, name), cx);
                }),
            )
            .children(missing.map(|c| {
                text(
                    format!("{c} is not connected, so the default is used."),
                    warn,
                    true,
                )
                .debug_selector(|| "settings-missing".into())
            }))
    }

    fn render_voice(&mut self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let s = self.model.settings.clone();
        let (inputs, outputs) = self.devices.clone();
        let mic = self.device_select(
            "settings-mic",
            &inputs,
            s.input_device.as_deref(),
            cx,
            |s, n| s.input_device = n,
        );
        let speakers = self.device_select(
            "settings-speakers",
            &outputs,
            s.output_device.as_deref(),
            cx,
            |s, n| s.output_device = n,
        );
        let mode = match s.denoise {
            Denoise::DeepFilter => 0,
            Denoise::Rnnoise => 1,
            Denoise::Gate => 2,
            Denoise::Off => 3,
        };
        let mode_help = match s.denoise {
            Denoise::DeepFilter => "Strongest. Runs on your computer.",
            Denoise::Rnnoise => {
                "Lighter on your computer. Takes out less noise than DeepFilterNet."
            }
            Denoise::Gate => {
                "Lighter. Mutes your mic between words, when it is under the level marker."
            }
            Denoise::Off => "No filtering.",
        };
        let noise = self.select(
            "settings-noise",
            vec![
                "DeepFilterNet (best)".into(),
                "RNNoise".into(),
                "Noise gate".into(),
                "Off".into(),
            ],
            Some(mode),
            640.,
            cx,
            |this, i, _, cx| {
                let d = [
                    Denoise::DeepFilter,
                    Denoise::Rnnoise,
                    Denoise::Gate,
                    Denoise::Off,
                ][i.min(3)];
                this.change_settings(|s| s.denoise = d, cx);
            },
        );
        let bitrate = self.page_slider(
            "bitrate",
            (32., 510., 1.),
            s.bitrate_kbps as f32,
            cx,
            |this, v, cx| {
                let kbps = v.round() as u32;
                if kbps != this.model.settings.bitrate_kbps {
                    this.change_settings(|s| s.bitrate_kbps = kbps, cx);
                }
            },
        );
        let in_call = self.model.call.phase != Phase::Idle;
        let key = h_flex()
            .min_h(px(32.))
            .justify_between()
            .gap_4()
            .child(text("Key", t.fg, false))
            .child(self.key_button(0, Some(&s.ptt_key), cx));
        let (cap_on, cap_for) = (self.capturing_key, self.capture_for);
        let capturing = move |which| cap_on && cap_for == which;
        let ptt_more = s.push_to_talk.then(|| {
            v_flex()
                .gap(px(6.))
                .child(key)
                .when(capturing(0), |d| {
                    d.child(
                        text(CAPTURING, t.fg_2, true).debug_selector(|| "settings-capturing".into()),
                    )
                })
                .when(self.capture_for == 0 && !self.key_problem.is_empty(), |d| {
                    d.child(
                        text(self.key_problem, t.warn_fg, true)
                            .debug_selector(|| "settings-key-problem".into()),
                    )
                })
                .child(text(
                    format!(
                        "Works while other apps have focus too. Your mic keeps sending for {} ms after you let go.",
                        ptt::RELEASE_DELAY.as_millis()
                    ),
                    t.fg_3,
                    true,
                ))
                .when_some(self.ptt_note, |d, n| {
                    d.child(text(n, t.warn_fg, true).debug_selector(|| "settings-ptt-note".into()))
                })
        });

        let marker = self.page_slider(
            "gate",
            (-100., 0., 1.),
            s.gate_db as f32,
            cx,
            |this, v, cx| {
                let db = v.round() as i32;
                if db != this.model.settings.gate_db {
                    this.change_settings(|s| s.gate_db = db, cx);
                }
            },
        );
        let mc = &self.model.mic;
        let hint = match mc.hint {
            Hint::Clipping => {
                Some("Too loud: the signal is clipping. Lower your microphone's volume.")
            }
            Hint::TooQuiet => Some("Very quiet or silent. Speak, or check the mic."),
            Hint::Ok => None,
        };
        let input = section()
            .child(self.voice_head("Input", Some(Group::Input), cx))
            .child(
                Field::new("Microphone")
                    .help("The system default follows your computer's sound settings.")
                    .child(mic),
            )
            .child(
                v_flex()
                    .gap(px(6.))
                    .child(div().font_weight(t.weight_ui).child("Input level"))
                    .child(meter(mc.db, Some(s.gate_db as f32), cx))
                    .children(hint.map(|h| text(h, t.warn_fg, true))),
            )
            .child(
                Field::new("Level marker")
                    .value(format!("{} dB", s.gate_db))
                    .help("The thin line across the meter. The noise gate mutes sound below it.")
                    .child(
                        div()
                            .debug_selector(|| "settings-marker".into())
                            .child(c::range(&marker, cx)),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_x(px(12.))
                    .gap_y(px(4.))
                    .child(
                        div().debug_selector(|| "settings-mic-test".into()).child(
                            Button::new(
                                "settings-mic-test",
                                if mc.testing {
                                    "Stop mic test"
                                } else {
                                    "Mic test (hear yourself)"
                                },
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                let cmds = this.model.mic_test();
                                this.run(cmds);
                                cx.notify();
                            })),
                        ),
                    )
                    .child(text(
                        "Use headphones, or you will get feedback.",
                        t.fg_3,
                        true,
                    ))
                    .when(!mc.error.is_empty(), |d| {
                        d.child(
                            text(mc.error.clone(), t.danger_fg, false)
                                .w_full()
                                .debug_selector(|| "settings-mic-error".into()),
                        )
                    }),
            );
        let echo = s.echo_cancel;
        let noise = section()
            .child(self.voice_head("Noise and echo", Some(Group::Noise), cx))
            .child(Field::new("Noise suppression").help(mode_help).child(noise))
            .child(
                div().debug_selector(|| "settings-echo".into()).child(
                    Switch::new("settings-echo", "Echo cancellation", echo)
                        .help("Takes the sound of your speakers out of your mic. Turn it off only with headphones.")
                        .on_change(on_bool(cx, |this, on, _, cx| {
                            this.change_settings(|s| s.echo_cancel = on, cx)
                        })),
                ),
            );
        let output = section()
            .child(self.voice_head("Output", None, cx))
            .child(Field::new("Speaker").child(speakers));
        let quality = section()
            .child(self.voice_head("Quality", Some(Group::Quality), cx))
            .child(
                Field::new("Bitrate")
                    .value(format!("{} kbps", s.bitrate_kbps))
                    .help("Higher sounds better and uses more data. 64 kbps is plenty for speech.")
                    .child(
                        div()
                            .debug_selector(|| "settings-bitrate".into())
                            .child(c::range(&bitrate, cx)),
                    ),
            )
            .when(in_call, |d| d.child(text(APPLIES_NEXT_CALL, t.fg_2, true)));
        let talk = section()
            .child(self.voice_head("Talk mode", Some(Group::Ptt), cx))
            .child(
                div().debug_selector(|| "settings-ptt".into()).child(
                    Switch::new("settings-ptt", "Push to talk", s.push_to_talk)
                        .help("Your mic sends only while you hold the key.")
                        .on_change(on_bool(cx, |this, on, _, cx| {
                            this.change_settings(|s| s.push_to_talk = on, cx)
                        })),
                ),
            )
            .children(ptt_more);
        let toggle_row =
            |which: usize, name: &'static str, key: Option<&String>, cx: &mut Context<Self>| {
                let clear = Button::new(SharedString::from(format!("clear-key-{which}")), "Clear")
                    .variant(Variant::Ghost)
                    .small()
                    .disabled(key.is_none())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_settings(|s| *toggle_key(s, which) = None, cx)
                    }));
                h_flex()
                    .min_h(px(32.))
                    .justify_between()
                    .gap_4()
                    .child(text(name, t.fg, false))
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .child(
                                div()
                                    .debug_selector(move || format!("clear-key-{which}"))
                                    .child(clear),
                            )
                            .child(self.key_button(which, key, cx)),
                    )
            };
        let toggles = v_flex()
            .gap(px(6.))
            .child(toggle_row(1, "Mute key", s.mute_key.as_ref(), cx))
            .child(toggle_row(2, "Deafen key", s.deafen_key.as_ref(), cx))
            .when(capturing(1) || capturing(2), |d| {
                d.child(text(CAPTURING, t.fg_2, true))
            })
            .when(self.capture_for > 0 && !self.key_problem.is_empty(), |d| {
                d.child(text(self.key_problem, t.warn_fg, true))
            })
            .child(text(
                "During a call these keys mute and deafen you, also while other apps have focus.",
                t.fg_3,
                true,
            ))
            .when_some(self.ptt_note, |d, n| d.child(text(n, t.warn_fg, true)));
        let keys = section()
            .child(h_flex().h(px(32.)).child(h3("Keyboard shortcuts", cx)))
            .child(toggles)
            .child(super::find::shortcut_list(&t));
        let error = (!self.settings_error.is_empty()).then(|| {
            text(self.settings_error.clone(), t.danger_fg, false)
                .debug_selector(|| "settings-error".into())
        });
        v_flex().gap(px(16.)).children(error).child(sections(
            vec![input, noise, output, quality, talk, keys],
            cx,
        ))
    }

    fn render_notifications(&mut self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let s = self.model.settings.clone();
        let volume = self.page_slider(
            "volume",
            (0., 100., 1.),
            s.volume as f32,
            cx,
            |this, v, cx| {
                let v = v.round() as u32;
                if v != this.model.settings.volume {
                    this.model.settings.volume = v;
                    this.save_settings_soon(cx);
                    cx.notify();
                }
            },
        );
        let sample = cx.listener(|this, _, _, _| {
            if let Some(a) = &this.audio {
                let s = &this.model.settings;
                super::sound::play_ring(a.clone(), s.output_device.clone(), s.volume);
            }
        });
        let test = cx.listener(|this, _, _, _| {
            if let Some(os) = &this.os {
                os.show(crate::notify::Toast {
                    title: "Vesper".into(),
                    body: "Mentions show like this.".into(),
                    room: 0,
                });
            }
        });
        sections(
            vec![
                section()
                    .child(h_flex().h(px(32.)).child(h3("Mentions and direct messages", cx)))
                    .child(
                        div().debug_selector(|| "settings-notify-mentions".into()).child(
                            Switch::new(
                                "settings-notify-mentions",
                                "Notify me when someone mentions me",
                                s.notify_mentions,
                            )
                            .on_change(on_bool(cx, |this, on, _, cx| {
                                this.change_settings(|s| s.notify_mentions = on, cx)
                            })),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "settings-notify-dms".into()).child(
                            Switch::new(
                                "settings-notify-dms",
                                "Notify me when someone sends me a direct message",
                                s.notify_dms,
                            )
                            .help("Also when someone calls you while Vesper is in the background. Nothing shows for the room you are looking at.")
                            .on_change(on_bool(cx, |this, on, _, cx| {
                                this.change_settings(|s| s.notify_dms = on, cx)
                            })),
                        ),
                    )
                    .child(
                        h_flex().child(
                            Button::new("notify-test", "Send a test notification")
                                .small()
                                .on_click(test),
                        ),
                    ),
                section()
                    .child(h_flex().h(px(32.)).child(h3("Sounds", cx)))
                    .child(
                        div().debug_selector(|| "settings-ring".into()).child(
                            Switch::new("settings-ring", "Ring for direct message calls", s.ring)
                                .help("A repeating tone while someone is calling you in a direct message. The prompt shows either way.")
                                .on_change(on_bool(cx, |this, on, _, cx| {
                                    this.change_settings(|s| s.ring = on, cx)
                                })),
                        ),
                    )
                    .child(
                        h_flex().ml(px(-8.)).mt(px(-12.)).child(
                            Button::new("ring-sample", "Play a sample")
                                .variant(Variant::Ghost)
                                .small()
                                .disabled(!s.ring || s.volume == 0)
                                .on_click(sample),
                        ),
                    )
                    .child(
                        Field::new("Volume").value(s.volume.to_string()).child(
                            div()
                                .when(!s.ring, |d| d.opacity(0.5))
                                .child(c::range(&volume, cx).disabled(!s.ring)),
                        ),
                    )
                    .child(text(
                        "The other call sounds of the web app are not in the desktop app yet.",
                        t.fg_3,
                        true,
                    )),
            ],
            cx,
        )
    }

    fn render_appearance(&mut self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let s = self.model.settings.clone();
        let schemes = [("dark", "Dark"), ("light", "Light"), ("system", "System")]
            .into_iter()
            .map(|(id, name)| {
                radio_row(
                    SharedString::from(format!("scheme-{id}")),
                    s.scheme == id,
                    cx,
                )
                .h(px(40.))
                .on_click(cx.listener(move |this, e: &ClickEvent, window, cx| {
                    this.page.reveal_from = e.mouse_position(); // keys switch at once
                    this.change_settings(|s| s.scheme = id.into(), cx);
                    this.sync_look(window, cx);
                }))
                .child(name)
            });
        let accent = self
            .my_user()
            .and_then(|u| u.ui_accent.clone())
            .unwrap_or_else(|| "amber".into());
        let swatches = theme::PRESETS.into_iter().map(|(id, name, fill)| {
            let on = accent == id;
            let hover = t.bg_hover;
            div()
                .id(SharedString::from(format!("accent-{id}")))
                .w_full()
                .debug_selector(move || format!("accent-{id}"))
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(40.))
                .px(px(10.))
                .rounded(t.radius)
                .border_1()
                .border_color(if on { t.line_strong } else { t.line })
                .when(on, |d| d.bg(t.bg_selected))
                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.save_accent(Some(id));
                    this.sync_look(window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(16.))
                        .flex_none()
                        .rounded_full()
                        .bg(gpui::rgb(fill)),
                )
                .child(div().flex_1().child(name))
                .when(on, |d| {
                    d.child(super::icons::icon("check").text_color(t.fg))
                })
        });
        let swatches: Vec<_> = swatches.collect();
        let mut grid = v_flex().gap(px(8.));
        let mut it = swatches.into_iter();
        loop {
            let row: Vec<_> = it.by_ref().take(4).collect();
            if row.is_empty() {
                break;
            }
            grid = grid.child(
                h_flex().gap(px(8.)).children(
                    row.into_iter()
                        .map(|s| v_flex().flex_1().min_w_0().child(s)),
                ),
            );
        }
        let scheme_help = "On this device only. System follows your device setting.";
        // A fieldset's legend is outside its flex gap: 4 px under it, then the gap between the rest.
        let fieldset = |name: &'static str, gap: f32, rest: Div| {
            v_flex()
                .child(h3(name, cx).mb(px(4.)))
                .child(rest.gap(px(gap)))
        };
        let err = self.page.account.accent_error.clone();
        v_flex()
            .gap(px(32.))
            .child(fieldset(
                "Colour scheme",
                8.,
                v_flex()
                    .child(text(scheme_help, t.fg_2, false).mb(px(4.)))
                    .child(radio_list(schemes.collect(), cx)),
            ))
            .child(
                fieldset(
                    "Accent",
                    12.,
                    v_flex()
                        .child(text(
                            "Buttons, focus, selection and mentions use it, on every device you sign in on.",
                            t.fg_2,
                            false,
                        ))
                        .child(grid)
                        .when(!err.is_empty(), |d| d.child(c::status(c::StatusKind::Error, err, cx)))
                        .child(
                            h_flex().child(
                                Button::new("accent-reset", "Reset to amber")
                                    .disabled(accent == "amber")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_accent(Some("amber"));
                                        this.sync_look(window, cx);
                                        cx.notify();
                                    })),
                            ),
                        ),
                )
                .w_full()
                .max_w(px(480.)),
            )
            .child(text(
                "When your system asks for less motion, Vesper fades panels in place of sliding them.",
                t.fg_2,
                false,
            ))
    }

    fn render_desktop(&mut self, cx: &mut Context<Self>) -> Div {
        let t = theme::tokens(cx).clone();
        let s = self.model.settings.clone();
        let error = self.settings_error.clone();
        let closes =
            [
                (
                    settings::OnClose::Call,
                    "Quit, but keep a call going in the tray",
                ),
                (settings::OnClose::Tray, "Keep running in the tray"),
                (settings::OnClose::Quit, "Quit, ending a call"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (choice, name))| {
                radio_row(
                    SharedString::from(format!("on-close-{i}")),
                    s.on_close == choice,
                    cx,
                )
                .h(px(40.))
                .debug_selector(move || format!("on-close-{i}"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change_settings(|s| s.on_close = choice, cx)
                }))
                .child(name)
            })
            .collect();
        let copy = cx.listener(|this, _, window, cx| {
            let report = this.diagnostics(window);
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(report));
            this.page.copied = true;
            cx.notify();
        });
        sections(
            vec![
                section()
                    .child(
                        div().debug_selector(|| "settings-autostart".into()).child(
                            Switch::new("settings-autostart", "Start with the computer", s.start_with_computer)
                                .help("Vesper opens when you sign in to your computer.")
                                .on_change(on_bool(cx, |this, on, _, cx| {
                                    this.change_settings(|s| s.start_with_computer = on, cx)
                                })),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "settings-in-tray".into()).child(
                            Switch::new("settings-in-tray", "Start in the tray", s.start_in_tray)
                                .help("When it starts with the computer, Vesper waits in the tray without opening its window.")
                                .disabled(!s.start_with_computer)
                                .on_change(on_bool(cx, |this, on, _, cx| {
                                    this.change_settings(|s| s.start_in_tray = on, cx)
                                })),
                        ),
                    )
                    .child(
                        div().debug_selector(|| "settings-updates".into()).child(
                            Switch::new("settings-updates", "Check for updates", s.check_updates)
                                .help("Vesper asks GitHub for a new version every few hours. It installs one only when you click Update and restart.")
                                .on_change(on_bool(cx, |this, on, _, cx| {
                                    this.change_settings(|s| s.check_updates = on, cx)
                                })),
                        ),
                    )
                    .child(
                        v_flex()
                            .gap(px(8.))
                            .child(h3("When you close the window", cx))
                            .child(radio_list(closes, cx))
                            .child(text("Quit in the tray menu always quits.", t.fg_3, true)),
                    )
                    .when(!error.is_empty(), |d| {
                        d.child(text(error, t.danger_fg, false).debug_selector(|| "settings-error".into()))
                    }),
                section()
                    .child(h_flex().h(px(32.)).child(h3("Diagnostics", cx)))
                    .child(text(
                        "Copies the app version, your system, graphics card and sound devices, and the end of the log, to paste into a bug report. Messages, names and passwords are never in it.",
                        t.fg_2,
                        true,
                    ))
                    .child(
                        h_flex()
                            .gap(px(12.))
                            .child(
                                div().debug_selector(|| "copy-diagnostics".into()).child(
                                    Button::new("copy-diagnostics", "Copy diagnostics").on_click(copy),
                                ),
                            )
                            .when(self.page.copied, |d| d.child(text("Copied.", t.fg_2, true))),
                    )
                    .when_some(crate::logfile::dir(), |d, dir| {
                        d.child(text(format!("Log files: {}", dir.display()), t.fg_3, true))
                    }),
            ],
            cx,
        )
    }

    /// The Copy diagnostics report. No message text, names, server address or secrets.
    pub(super) fn diagnostics(&self, window: &Window) -> String {
        let s = &self.model.settings;
        let gpu = window.gpu_specs().map_or("unknown".into(), |g| {
            format!(
                "{} ({} {}){}",
                g.device_name,
                g.driver_name,
                g.driver_info,
                if g.is_software_emulated {
                    ", software"
                } else {
                    ""
                }
            )
        });
        let session = if cfg!(target_os = "linux") {
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                " (Wayland)"
            } else {
                " (X11)"
            }
        } else {
            ""
        };
        let list = |v: &[String]| {
            if v.is_empty() {
                "not listed".into()
            } else {
                v.join(", ")
            }
        };
        let chosen = |d: &Option<String>| d.clone().unwrap_or_else(|| "system default".into());
        let mut r = format!(
            "Vesper desktop {}\nSystem: {} {}{session}\nGraphics: {gpu}\nMicrophones: {}\nSpeakers: {}\n\
             Chosen: {} / {}\nNoise filter: {:?}, echo cancellation {}, push to talk {}\nGlobal keys: {}\n\
             In a call: {}\n\nLog:\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            list(&self.devices.0),
            list(&self.devices.1),
            chosen(&s.input_device),
            chosen(&s.output_device),
            s.denoise,
            if s.echo_cancel { "on" } else { "off" },
            if s.push_to_talk { "on" } else { "off" },
            self.ptt_note.unwrap_or("available"),
            if self.model.call.phase == Phase::Idle {
                "no"
            } else {
                "yes"
            },
        );
        for l in crate::logfile::dir()
            .map(|d| crate::logfile::tail(&d, 100))
            .unwrap_or_default()
        {
            r.push_str(&l);
            r.push('\n');
        }
        r
    }
}
