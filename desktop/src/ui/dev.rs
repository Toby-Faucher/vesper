//! Debug builds only (the module is `cfg(debug_assertions)`, so a release build has none of it): puts the app in a
//! state for the parity harness (`web/e2e/parity.mjs`) without typing.
//!
//! - `VESPER_DEV_SERVER`, `VESPER_DEV_USER`, `VESPER_DEV_PASSWORD`: sign in with these once the sign in screen shows.
//! - `VESPER_DEV_SCENE`: `sign-in` (stay there), or after signing in `room:<name>` (and wait for its messages),
//!   `settings`, or anything else for the main screen as it opens.
//! - `VESPER_DEV_CLOCK`: `HH:MM`, the time every message shows (the web scenes pin 7:41 PM). Dates are left alone.
//! - `VESPER_DEV_SHIFT`: `id:days,...`, messages moved back that many days (the web harness does the same to its pages).
//! - `VESPER_DEV_EMPTY`: the room has no messages: ready once its first page came.
//! - `VESPER_DEV_DRAFT`: text put in the composer, with focus there, once the room is ready.
//! - `VESPER_DEV_PINS`: opens the pins list once the room is ready.
//! - `VESPER_DEV_SEND`: text sent from the composer 2.5 s after the room is ready (past the first hello's calm), then
//!   `VESPER_DEV_ARRIVED` on stderr once it is in the list, for the harness's motion frames.
//! - `VESPER_DEV_TYPING`: the room is ready only once someone is typing there (a typing push comes every few seconds).
//! - `VESPER_DEV_TRAY`: `name|size|mime|state;...` rows put in the room's upload tray (no upload runs). The state is
//!   `queued`, `uploading:<share>` or `failed:quota`.
//!
//! Prints `VESPER_DEV_READY` on stderr once the state is reached; the harness takes its picture after that.

use std::collections::HashMap;
use std::path::PathBuf;

use gpui::{Context, Window};
use gpui_component::input::RopeExt;

use super::Vesper;
use crate::model::Screen;
use crate::uploads::{Item, UPLOAD_QUOTA, UpState};

#[derive(Debug, Default, PartialEq)]
enum Step {
    #[default]
    SignIn,
    Scene,
    /// The room is open; ready once its messages are in.
    Room,
    Done,
}

#[derive(Debug, Default)]
pub struct Dev {
    login: Option<(String, String, String)>,
    scene: Option<String>,
    step: Step,
    /// Days back, by message id.
    shift: HashMap<i64, i64>,
    empty: bool,
    draft: Option<String>,
    pins: bool,
    typing: bool,
    tray: Option<String>,
    send: Option<String>,
    /// The newest message id when the send went: ARRIVED is printed once a newer one is in.
    sent_after: Option<i64>,
}

impl Dev {
    pub fn from_env() -> Dev {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let login = match (
            var("VESPER_DEV_SERVER"),
            var("VESPER_DEV_USER"),
            var("VESPER_DEV_PASSWORD"),
        ) {
            (Some(s), Some(u), Some(p)) => Some((s, u, p)),
            _ => None,
        };
        let any = login.is_some() || var("VESPER_DEV_SCENE").is_some();
        if let Some((h, m)) = var("VESPER_DEV_CLOCK").and_then(|c| {
            let (h, m) = c.split_once(':')?;
            Some((h.parse().ok()?, m.parse().ok()?))
        }) {
            let _ = crate::timeline::FIXED_CLOCK.set((h, m));
        }
        let shift = var("VESPER_DEV_SHIFT")
            .unwrap_or_default()
            .split(',')
            .filter_map(|p| {
                let (id, d) = p.split_once(':')?;
                Some((id.parse().ok()?, d.parse().ok()?))
            })
            .collect();
        Dev {
            login,
            scene: var("VESPER_DEV_SCENE"),
            step: if any { Step::SignIn } else { Step::Done },
            shift,
            empty: var("VESPER_DEV_EMPTY").is_some(),
            draft: var("VESPER_DEV_DRAFT"),
            pins: var("VESPER_DEV_PINS").is_some(),
            typing: var("VESPER_DEV_TYPING").is_some(),
            send: var("VESPER_DEV_SEND"),
            sent_after: None,
            tray: var("VESPER_DEV_TRAY"),
        }
    }

    /// Moves the shifted messages back, once each: a message still within the last 20 hours has not been moved.
    fn shift(&self, model: &mut crate::model::Model) {
        const DAY: i64 = 86_400_000;
        let recent = chrono::Utc::now().timestamp_millis() - 20 * 3_600_000;
        for log in model.chat.rooms.values_mut() {
            for m in &mut log.messages {
                if let Some(d) = self.shift.get(&m.id).filter(|_| m.created_at > recent) {
                    m.created_at -= d * DAY;
                }
            }
        }
    }
}

fn ready(what: &str) {
    eprintln!("VESPER_DEV_READY {what}");
}

impl Vesper {
    /// One step towards the asked-for state, after each event from the network.
    pub(super) fn dev_step(&mut self, latest: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dev.shift.is_empty() {
            self.dev.shift(&mut self.model);
        }
        let last = self
            .model
            .log()
            .and_then(|l| l.messages.last())
            .map(|m| m.id);
        if self
            .dev
            .sent_after
            .is_some_and(|a| last.is_some_and(|l| l > a))
        {
            self.dev.sent_after = None;
            eprintln!("VESPER_DEV_ARRIVED");
        }
        match self.dev.step {
            Step::SignIn if self.model.screen == Screen::SignIn => {
                let Some((server, user, pass)) = self.dev.login.clone() else {
                    self.dev.step = Step::Done;
                    return ready("sign-in");
                };
                self.server
                    .update(cx, |i, cx| i.set_value(server, window, cx));
                self.username
                    .update(cx, |i, cx| i.set_value(user, window, cx));
                self.password
                    .update(cx, |i, cx| i.set_value(pass, window, cx));
                self.submit_sign_in(window, cx);
                self.dev.step = Step::Scene;
            }
            // The server takes 5 sign ins per name, then one a minute, fewer than a harness run makes: wait, try again.
            Step::Scene if self.model.screen == Screen::SignIn && !self.model.busy => {
                self.dev.step = Step::Done;
                cx.spawn_in(window, async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(13))
                        .await;
                    let _ = this.update_in(cx, |v, window, cx| {
                        v.dev.step = Step::SignIn;
                        v.dev_step(false, window, cx);
                    });
                })
                .detach();
            }
            Step::Scene
                if self.model.screen == Screen::Main && !self.model.state.rooms.is_empty() =>
            {
                match self.dev.scene.as_deref().unwrap_or("") {
                    "settings" => {
                        self.open_settings(window, cx);
                        self.dev.step = Step::Done;
                        ready("settings");
                    }
                    s if s.starts_with("room:") => {
                        let name = &s["room:".len()..];
                        let Some(id) = self
                            .model
                            .state
                            .rooms
                            .iter()
                            .find(|r| r.name == name)
                            .map(|r| r.id)
                        else {
                            return;
                        };
                        self.open_room(id, window, cx);
                        self.dev.step = Step::Room;
                    }
                    _ => {
                        self.model.not_remembered = false;
                        self.dev.step = Step::Done;
                        ready("main");
                    }
                }
            }
            Step::Room
                if (self.model.log().is_some_and(|l| !l.messages.is_empty())
                    || (self.dev.empty && latest))
                    && (!self.dev.typing
                        || self.model.active.is_some_and(|r| {
                            !self.model.typers(r, std::time::Instant::now()).is_empty()
                        })) =>
            {
                // The throwaway home has no keyring on purpose: its note is not part of any web scene.
                self.model.not_remembered = false;
                if let Some(text) = self.dev.draft.take() {
                    self.composer.update(cx, |i, cx| {
                        let end = text.len();
                        i.set_value(text, window, cx);
                        // The caret at the end, as after typing: the @ list reads the text before it.
                        let pos = i.text().offset_to_position(end);
                        i.set_cursor_position(pos, window, cx);
                    });
                    self.composer.update(cx, |i, cx| i.focus(window, cx));
                    self.composer_changed(cx);
                } else {
                    // Nothing focused, as the web scenes' rest(): no caret or focus border in the picture.
                    self.key_focus.focus(window);
                }
                if self.dev.pins {
                    let cmds = self.model.toggle_pins();
                    self.run(cmds);
                }
                if let (Some(rows), Some(room)) = (self.dev.tray.take(), self.model.active) {
                    self.model
                        .tray
                        .items
                        .extend(rows.split(';').zip(1000..).filter_map(|(r, key)| {
                            let mut f = r.split('|');
                            let (name, size) = (f.next()?, f.next()?.parse().ok()?);
                            let state = f.nth(1)?;
                            let (state, progress, error) = match state.split_once(':') {
                                Some(("uploading", p)) => (UpState::Uploading, p.parse().ok()?, ""),
                                Some(("failed", _)) => (UpState::Failed, 0., UPLOAD_QUOTA),
                                _ => (UpState::Queued, 0., ""),
                            };
                            Some(Item {
                                key,
                                room,
                                path: PathBuf::from(name),
                                name: name.into(),
                                size,
                                state,
                                progress,
                                error: error.into(),
                                retry: state == UpState::Failed,
                                attachment: None,
                            })
                        }));
                }
                if let Some(text) = self.dev.send.take() {
                    cx.spawn_in(window, async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(2500))
                            .await;
                        let _ = this.update_in(cx, |v, window, cx| {
                            v.composer.update(cx, |i, cx| i.set_value(text, window, cx));
                            v.dev.sent_after = Some(
                                v.model
                                    .log()
                                    .and_then(|l| l.messages.last())
                                    .map_or(0, |m| m.id),
                            );
                            v.send(&super::SendMessage, window, cx);
                        });
                    })
                    .detach();
                }
                self.dev.step = Step::Done;
                ready("room");
            }
            _ => {}
        }
    }
}
