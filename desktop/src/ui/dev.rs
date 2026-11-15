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
//! - `VESPER_DEV_VOICE`: a room name: once the scene is reached, join voice there, with a silent fake microphone and
//!   speakers that play nothing (no audio device is opened). Ready once connected and the first stats came.
//! - `VESPER_DEV_LINKS`: with VOICE, ready only once everyone else in the call has a connection grade.
//! - `VESPER_DEV_RTT`: with VOICE, the call's round trip in ms, in place of the measured one (`call-weak`).
//! - `VESPER_DEV_MIC_LOST`: with VOICE, the fake microphone is unplugged a second into the call and does not come back.
//! - `VESPER_DEV_DEAFEN`: with VOICE, deafened once in.
//! - `VESPER_DEV_PTT_HELD`: with VOICE, the push-to-talk key is held (push to talk is on in the scene's settings).
//! - `VESPER_DEV_PERSON`: `<display name>@bar` or `@side`: opens that person's card from the call bar or the sidebar
//!   once ready; `VESPER_DEV_VOLUME` sets their volume first.
//! - `VESPER_DEV_RING`: a username: a DM call from them rings once ready (not from the server: for motion frames).
//!
//! Prints `VESPER_DEV_READY` on stderr once the state is reached; the harness takes its picture after that. Then
//! `VESPER_DEV_OFFLINE` once the socket drops (the harness stops the server for the offline scenes), and
//! `VESPER_DEV_ARRIVED` once a ring comes (`motion-ring`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::{Context, Window};
use gpui_component::input::RopeExt;

use super::Vesper;
use crate::model::{Phase, Screen};
use crate::uploads::{Item, UPLOAD_QUOTA, UpState};
use crate::voice::{AudioDevices, AudioInput, AudioOutput, RATE};

#[derive(Debug, Default, PartialEq)]
enum Step {
    #[default]
    SignIn,
    Scene,
    /// The room is open; ready once its messages are in.
    Room,
    /// In voice; ready once connected and measured.
    Voice,
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
    voice: Option<String>,
    links: bool,
    rtt: Option<f32>,
    ptt_held: bool,
    deafen: bool,
    person: Option<(String, bool)>,
    volume: Option<f32>,
    ring: Option<String>,
    offline_said: bool,
    /// A ring came after ready: ARRIVED was printed.
    rang: bool,
}

/// A silent microphone paced by the clock, and speakers that drop what they get. `lose` unplugs the microphone a
/// second in, for good.
pub struct FakeAudio {
    lose: bool,
    opened: AtomicBool,
}

struct Mic {
    start: Instant,
    made: u64,
    lose: bool,
}

impl AudioInput for Mic {
    fn read(&mut self, out: &mut Vec<f32>) {
        let due = (self.start.elapsed().as_secs_f64() * f64::from(RATE)) as u64;
        out.extend((self.made..due).map(|_| 0.));
        self.made = due;
    }
    fn alive(&self) -> bool {
        !self.lose || self.start.elapsed() < Duration::from_secs(1)
    }
}

struct Speakers;

impl AudioOutput for Speakers {
    fn write(&mut self, _: &[f32]) {}
}

impl AudioDevices for FakeAudio {
    fn input(&self, _: Option<&str>) -> Result<Box<dyn AudioInput>, String> {
        if self.lose && self.opened.swap(true, Ordering::Relaxed) {
            return Err("No microphone was found.".into());
        }
        Ok(Box::new(Mic {
            start: Instant::now(),
            made: 0,
            lose: self.lose,
        }))
    }
    fn output(&self, _: Option<&str>) -> Result<Box<dyn AudioOutput>, String> {
        Ok(Box::new(Speakers))
    }
}

/// The fake devices when a scene joins voice, else none (the real ones).
pub fn audio() -> Option<Arc<dyn AudioDevices>> {
    std::env::var("VESPER_DEV_VOICE")
        .ok()
        .filter(|v| !v.is_empty())?;
    Some(Arc::new(FakeAudio {
        lose: std::env::var_os("VESPER_DEV_MIC_LOST").is_some(),
        opened: AtomicBool::new(false),
    }))
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
            voice: var("VESPER_DEV_VOICE"),
            links: var("VESPER_DEV_LINKS").is_some(),
            rtt: var("VESPER_DEV_RTT").and_then(|v| v.parse().ok()),
            ptt_held: var("VESPER_DEV_PTT_HELD").is_some(),
            deafen: var("VESPER_DEV_DEAFEN").is_some(),
            person: var("VESPER_DEV_PERSON").and_then(|v| {
                let (name, at) = v.rsplit_once('@')?;
                Some((name.to_string(), at == "bar"))
            }),
            volume: var("VESPER_DEV_VOLUME").and_then(|v| v.parse().ok()),
            ring: var("VESPER_DEV_RING"),
            offline_said: false,
            rang: false,
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
        if self.dev.step == Step::Done && !self.model.rings.is_empty() && !self.dev.rang {
            self.dev.rang = true;
            eprintln!("VESPER_DEV_ARRIVED");
        }
        if self.dev.step == Step::Done && !self.model.online && !self.dev.offline_said {
            self.dev.offline_said = true;
            eprintln!("VESPER_DEV_OFFLINE");
        }
        if let Some(rtt) = self.dev.rtt
            && let Some(q) = &mut self.model.call.quality
        {
            q.rtt_ms = Some(rtt);
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
                        self.dev_finish("main", window, cx);
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
                self.dev_finish("room", window, cx);
            }
            Step::Voice
                if self.model.call.phase == Phase::Joined
                    && self.model.call.connected
                    && self.model.call.quality.is_some()
                    && (!self.dev.links || {
                        let c = &self.model.call;
                        let me = self.model.state.my_id();
                        self.model
                            .state
                            .voice
                            .iter()
                            .filter(|p| Some(p.room_id) == c.room && p.user_id != me)
                            .all(|p| c.links.get(&p.user_id).is_some_and(|l| l.grade.is_some()))
                    })
                    && (self.model.call.devices.is_empty()
                        == std::env::var_os("VESPER_DEV_MIC_LOST").is_none()) =>
            {
                self.dev.step = Step::Done;
                if self.dev.ptt_held {
                    self.act(|m| m.set_ptt(true), cx);
                }
                if self.dev.deafen {
                    self.act(crate::model::Model::toggle_deafen, cx);
                }
                self.dev_person(window, cx);
                ready("voice");
            }
            _ => {}
        }
    }
}

impl Vesper {
    /// The scene is reached: join voice when asked (ready comes later), else open what was asked and say ready.
    fn dev_finish(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(name) = self.dev.voice.clone() {
            let Some(id) = self
                .model
                .state
                .rooms
                .iter()
                .find(|r| r.name == name)
                .map(|r| r.id)
            else {
                return ready(what);
            };
            self.act(|m| m.join_voice(id), cx);
            self.dev.step = Step::Voice;
            return;
        }
        if let Some(user) = self.dev.ring.take() {
            let me = self.model.state.my_id().to_string();
            let caller = self
                .model
                .state
                .users
                .values()
                .find(|u| u.username == user)
                .map(|u| u.user_id.clone());
            let dm = caller.as_ref().and_then(|c| {
                self.model
                    .state
                    .dms
                    .iter()
                    .find(|d| d.members.contains(c) && d.members.contains(&me))
                    .map(|d| d.id)
            });
            if let (Some(c), Some(room)) = (caller, dm) {
                self.model.rings.push((room, c));
            }
        }
        self.dev_person(window, cx);
        ready(what);
    }

    /// Opens the asked-for person card, at its volume.
    fn dev_person(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some((name, bar)) = self.dev.person.take() else {
            return;
        };
        let s = &self.model.state;
        let me = s.me.as_ref().map(|m| m.user.clone());
        let Some(id) = s
            .users
            .values()
            .chain(me.as_ref())
            .find(|u| u.display_name == name)
            .map(|u| u.user_id.clone())
        else {
            return;
        };
        if let Some(v) = self.dev.volume {
            self.act(|m| m.set_volume(&id, v), cx);
        }
        let anchor = if bar {
            format!("bar-{id}")
        } else {
            let room = self
                .model
                .state
                .voice
                .iter()
                .find(|p| p.user_id == id)
                .map_or(0, |p| p.room_id);
            format!("person-{room}-{id}")
        };
        self.open_person(id, anchor, bar, cx);
    }
}
