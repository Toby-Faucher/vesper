//! `CallBar.svelte`: the call, floating at the bottom of the main pane while you are in voice. The room and the
//! connection, mute, deafen, push to talk, the people in the call (a click opens their card), Call details and Leave,
//! then one status line: an error or a notice, and the noise filter at its end. It rises in when the call starts
//! (`rise`). Without a call, what ended the last one shows under the room header (`call_status`).
//! The call timer rolls its digits; the bar can be a small pill that opens on hover ([`Bar`]).
//!
//! Not here: Share screen and the share card (the desktop does not send a screen), the leave fade (the web fades an
//! inert copy out), Retry microphone and a new microphone's offer (the desktop's engine opens devices again itself).

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::{
    AppContext, Context, Div, ElementId, Entity, InteractiveElement, IntoElement, ParentElement,
    Pixels, Size, StatefulInteractiveElement, Styled, Task, Window, canvas, div, px, size,
    transparent_black,
};
use gpui_component::slider::{SliderEvent, SliderState};

use super::Vesper;
use super::c::{self, Avatar, Ellipsis, IconButton, StatusKind, mix};
use super::icons::icon;
use super::motion::{self, RISE};
use super::theme::{TEXT_SM, TEXT_XS, TextStepExt, tokens};
use crate::link::{Grade, quality, quality_text};
use crate::model::{HEADPHONES, Model, Phase, REJOINING};
use crate::net::Cmd;

/// The bar's height: its 1 px borders, the 52 px row and the 28 px status line (its rule inside, border-box).
pub const BAR_H: f32 = 82.;
/// The pill's: its borders and the 36 px row.
pub const PILL_H: f32 = 38.;

/// The pill (`CallBar.svelte`): with `call_small` the bar is the room, the mic and the timer, opened to the full bar
/// while the pointer is on it (closed 300 ms after it leaves); a notice or an error keeps it full. A change between the
/// two morphs the box (`morph` in motion.ts): its size and corners from the old to the new over `--dur-3`, ease-out,
/// the content fading in over `--dur-2`.
#[derive(Default)]
pub struct Bar {
    small: bool,
    peek: bool,
    leave: Option<Task<()>>,
    tick: Option<Task<()>>,
    /// Drawn as the pill last frame.
    compact: Option<bool>,
    /// The box last frame, and the content's own size (measured while morphing).
    size: Rc<Cell<Size<Pixels>>>,
    inner: Rc<Cell<Size<Pixels>>>,
    /// From this size and radius, since then.
    morph: Option<(Size<Pixels>, Pixels, Instant)>,
}

impl Bar {
    /// The room the main pane keeps under its content: the pill's while small, also while it peeks.
    pub fn room(&self) -> f32 {
        if self.small && (self.compact == Some(true) || self.peek) {
            PILL_H
        } else {
            BAR_H
        }
    }
}

/// The call timer: m:ss, and h:mm:ss from an hour on (`clock.ts`).
pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m) = (s / 3600, s / 60 % 60);
    if h > 0 {
        format!("{h}:{m:02}:{:02}", s % 60)
    } else {
        format!("{m}:{:02}", s % 60)
    }
}
/// Avatars drawn in the bar; the rest is `+n`.
const AVATARS: usize = 5;
pub const FORCE_MUTED: &str = "A moderator muted you.";

impl Vesper {
    /// Runs one model action and redraws.
    pub(super) fn act(&mut self, f: impl FnOnce(&mut Model) -> Vec<Cmd>, cx: &mut Context<Self>) {
        let cmds = f(&mut self.model);
        self.run(cmds);
        cx.notify();
    }

    /// The volume slider state of `id`, made once and kept.
    pub(super) fn slider(&mut self, id: &str, cx: &mut Context<Self>) -> Entity<SliderState> {
        if !self.sliders.contains_key(id) {
            let start = self.model.volume(id);
            let state = cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(crate::voice::mix::MAX_VOLUME)
                    .step(5.)
                    .default_value(start)
            });
            let user = id.to_string();
            let sub = cx.subscribe(&state, move |this, _, e: &SliderEvent, cx| {
                let SliderEvent::Change(v) = e;
                let v = v.start();
                this.act(|m| m.set_volume(&user, v), cx);
                this.save_settings_soon(cx);
            });
            self.sliders.insert(id.to_string(), (state, sub));
        }
        self.sliders[id].0.clone()
    }

    /// Without a call: a join that failed or a call that ended (`Status` error), or a rejoin on its way (warn), under
    /// the room header as in RoomView.svelte.
    pub(super) fn call_status(&self, cx: &Context<Self>) -> Option<Div> {
        let c = &self.model.call;
        if c.phase != Phase::Idle {
            return None;
        }
        let (kind, text) = if !c.error.is_empty() {
            (StatusKind::Error, c.error.clone())
        } else if c.rejoin.is_some() {
            (StatusKind::Warn, REJOINING.to_string())
        } else {
            return None;
        };
        Some(
            div()
                .flex_none()
                .px(px(16.))
                .pt(px(12.))
                .debug_selector(|| "call-error".into())
                .child(c::status(kind, text, cx)),
        )
    }

    /// The bar, absolutely placed at the bottom of the main pane (which keeps `BAR_H + 24` px free under its content).
    pub(super) fn render_call(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let joined = self.model.call.phase == Phase::Joined;
        if !joined {
            self.bar_since = None;
            self.bar = Bar::default();
            return None;
        }
        let now = Instant::now();
        let since = *self.bar_since.get_or_insert(now);
        let t = tokens(cx).clone();
        let pose = motion::pose(&t.motion, RISE, false, since, now);
        if pose.is_some() {
            window.request_animation_frame();
        }
        // The timer redraws the bar once a second, but not while a share is watched: an untaken picture with no draw
        // is how a covered window is found (`frame_came`), and the share's own pictures redraw the bar then.
        if self.bar.tick.is_none() {
            self.bar.tick = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    let tick = |v: &mut Vesper, cx: &mut Context<Vesper>| {
                        if v.model.call.frame.is_none() {
                            cx.notify();
                        }
                    };
                    if this.update(cx, tick).is_err() {
                        break;
                    }
                }
            }));
        }
        let time = clock(now.saturating_duration_since(since));
        // The pill, and the morph when it changes.
        let small = self.model.settings.call_small;
        let quiet = {
            let c = &self.model.call;
            c.error.is_empty()
                && c.devices.is_empty()
                && !c.force_muted
                && c.echo_off.is_empty()
                && !c.headphones
        };
        let compact = small && !self.bar.peek && quiet;
        let radius = if compact {
            px(PILL_H / 2.)
        } else {
            t.radius_lg
        };
        self.bar.small = small;
        if self.bar.compact.is_some_and(|was| was != compact) && !t.motion.reduced {
            let r0 = if compact {
                t.radius_lg
            } else {
                px(PILL_H / 2.)
            };
            self.bar.inner.set(Size::default());
            self.bar.morph = Some((self.bar.size.get(), r0, now));
        }
        self.bar.compact = Some(compact);
        let morph = self.bar.morph.and_then(|(from, r0, at)| {
            let x = motion::fraction(at, now, t.motion.dur_3);
            (x < 1.).then_some((from, r0, x))
        });
        if morph.is_some() {
            window.request_animation_frame();
        } else {
            self.bar.morph = None;
        }

        let m = &self.model;
        let c = &m.call;
        let s = &m.state;
        let room = c
            .room
            .and_then(|id| s.find_room(id))
            .map(|r| r.name.clone())
            .unwrap_or_default();
        let grade = quality(c.quality.as_ref());
        let q_text = quality_text(c.quality.as_ref());
        let q_color = if grade == Some(Grade::Poor) {
            t.warn_fg
        } else {
            t.fg_3
        };
        let set = &m.settings;
        let ptt = set
            .push_to_talk
            .then(|| (crate::voice::ptt::key_label(&set.ptt_key), m.ptt_open));
        let (muted, deafened, forced) = (c.muted, c.deafened, c.force_muted);
        let me = s.my_id().to_string();
        // You first, then the others in the call, as the web's `people`.
        let mut people: Vec<(String, String, String, bool)> =
            s.me.as_ref()
                .map(|u| {
                    let u = &u.user;
                    (
                        u.user_id.clone(),
                        u.display_name.clone(),
                        u.accent.clone(),
                        c.me_speaking,
                    )
                })
                .into_iter()
                .collect();
        people.extend(
            s.voice
                .iter()
                .filter(|p| Some(p.room_id) == c.room && p.user_id != me)
                .map(|p| {
                    let accent = s
                        .users
                        .get(&p.user_id)
                        .map_or(String::new(), |u| u.accent.clone());
                    (
                        p.user_id.clone(),
                        p.name.clone(),
                        accent,
                        c.speaking.contains(&p.user_id),
                    )
                }),
        );
        let more = people.len().saturating_sub(AVATARS);
        let notice = if !c.devices.is_empty() {
            Some(("call-devices", c.devices.clone()))
        } else if forced {
            Some(("call-forced", FORCE_MUTED.to_string()))
        } else if !c.echo_off.is_empty() {
            Some(("call-echo", c.echo_off.clone()))
        } else if c.headphones {
            Some(("call-headphones", HEADPHONES.to_string()))
        } else {
            None
        };
        let headphones = c.headphones && notice.as_ref().is_some_and(|n| n.0 == "call-headphones");
        let error = c.error.clone();
        let filter = noise_line(&c.filter);

        let sep = || {
            div()
                .mx(px(8.))
                .h(px(24.))
                .w(px(1.))
                .flex_none()
                .bg(t.line_subtle)
        };
        let avatars: Vec<_> = people
            .into_iter()
            .take(AVATARS)
            .map(|(id, name, accent, speaking)| {
                let key = format!("bar-{id}");
                let el = ElementId::Name(format!("call-person-{id}").into());
                let (h, on_hover) = motion::hover(el.clone(), window, cx);
                let probe = self.probe(key.clone());
                let sel = id.clone();
                let user = id.clone();
                div()
                    .id(el)
                    .debug_selector(move || format!("call-person-{sel}"))
                    .relative()
                    .flex()
                    .size(px(28.))
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(mix(transparent_black(), t.bg_hover.into(), h))
                    .on_hover(on_hover)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_person(user.clone(), key.clone(), true, cx)
                    }))
                    .child(probe)
                    .when(speaking, |d| {
                        let sel = id.clone();
                        d.child(
                            div()
                                .absolute()
                                .debug_selector(move || format!("speaking-{sel}")),
                        )
                    })
                    .child(
                        Avatar::new(id, name, accent, 20)
                            .speaking(speaking)
                            .surface(t.bg_raised),
                    )
            })
            .collect();
        let (dh, details_hover) = motion::hover("call-details", window, cx);

        let row = div()
            .flex()
            .h(px(52.))
            .min_w_0()
            .items_center()
            .gap(px(4.))
            .pr(px(8.))
            .pl(px(12.))
            .child(
                div()
                    .mr(px(8.))
                    .flex()
                    .flex_col()
                    .w(px(192.))
                    .min_w_0()
                    .flex_shrink()
                    .child(
                        div()
                            .flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .min_w_0()
                                    .ellipsis()
                                    .text_step(TEXT_SM)
                                    .font_weight(t.weight_strong)
                                    .text_color(t.fg)
                                    .child(room.clone()),
                            )
                            .child(
                                div()
                                    .ml_auto()
                                    .flex_none()
                                    .text_step(TEXT_XS)
                                    .text_color(t.fg_3)
                                    .when(!compact, |d| {
                                        d.child(motion::roll(
                                            "call-time",
                                            &time,
                                            px(16.),
                                            false,
                                            window,
                                            cx,
                                        ))
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .debug_selector(|| "call-quality".into())
                            .flex()
                            .min_w_0()
                            .items_center()
                            .gap(px(6.))
                            .text_step(TEXT_XS)
                            .text_color(q_color)
                            .child(c::bars(grade.map_or(0, Grade::bars), q_color))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(q_text),
                            ),
                    ),
            )
            .child(
                div().debug_selector(|| "call-mute".into()).child(
                    IconButton::new("call-mute", if muted { "micOff" } else { "mic" })
                        .danger(muted)
                        .disabled(forced)
                        .on_click(cx.listener(|this, _, _, cx| this.act(Model::toggle_mute, cx))),
                ),
            )
            .child(
                div().debug_selector(|| "call-deafen".into()).child(
                    IconButton::new(
                        "call-deafen",
                        if deafened {
                            "headphonesOff"
                        } else {
                            "headphones"
                        },
                    )
                    .danger(deafened)
                    .on_click(cx.listener(|this, _, _, cx| this.act(Model::toggle_deafen, cx))),
                ),
            )
            .when_some(ptt, |d, (key, held)| d.child(ptt_pill(&key, held, &t)))
            .child(sep())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.))
                    .children(avatars)
                    .when(more > 0, |d| {
                        d.child(
                            div()
                                .px(px(4.))
                                .text_step(TEXT_XS)
                                .text_color(t.fg_2)
                                .child(format!("+{more}")),
                        )
                    }),
            )
            .child(sep())
            .child(
                div()
                    .id("call-details")
                    .flex()
                    .h(px(32.))
                    .flex_none()
                    .items_center()
                    .rounded(t.radius)
                    .px(px(8.))
                    .text_step(TEXT_SM)
                    .text_color(mix(t.fg_2.into(), t.fg.into(), dh))
                    .bg(mix(transparent_black(), t.bg_hover.into(), dh))
                    .cursor_pointer()
                    .on_hover(details_hover)
                    .on_click(cx.listener(|this, _, window, cx| this.open_settings(window, cx)))
                    .child("Call details"),
            )
            .child(
                div().debug_selector(|| "call-leave".into()).child(
                    c::Button::new("call-leave", "Leave")
                        .variant(c::Variant::DangerGhost)
                        .on_click(cx.listener(|this, _, _, cx| this.act(Model::leave_voice, cx))),
                ),
            )
            .child(
                div().debug_selector(|| "call-small".into()).child(
                    IconButton::new(
                        "call-small",
                        if small { "chevronUp" } else { "chevronDown" },
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.bar.peek = false;
                        this.change_settings(|s| s.call_small = !small, cx);
                        cx.notify();
                    })),
                ),
            );

        // One status line, always the same height; the noise filter keeps the right end.
        // Its content in an absolute layer: out of the bar's content sizing, so the ellipses are measured once, at
        // the bar's width.
        let layer = div()
            .absolute()
            .inset_0()
            .flex()
            .min_w_0()
            .items_center()
            .gap(px(8.))
            .border_t_1()
            .border_color(t.line_subtle)
            .px(px(12.))
            .text_step(TEXT_XS)
            .text_color(t.fg_3)
            .map(|d| {
                if !error.is_empty() {
                    d.child(
                        div()
                            .debug_selector(|| "call-error".into())
                            .min_w_0()
                            .ellipsis()
                            .text_color(t.danger_fg)
                            .child(error),
                    )
                } else if let Some((sel, text)) = notice {
                    d.child(
                        div()
                            .debug_selector(move || sel.into())
                            .flex()
                            .min_w_0()
                            .mx(px(-8.))
                            .py(px(2.))
                            .px(px(8.))
                            .gap(px(8.))
                            .rounded(t.radius)
                            .bg(t.warn_subtle)
                            .text_color(t.fg)
                            .child(
                                div()
                                    .flex()
                                    .h(px(16.))
                                    .flex_none()
                                    .items_center()
                                    .child(icon("warn").text_color(t.warn_fg)),
                            )
                            .child(div().min_w_0().ellipsis().child(text)),
                    )
                    .when(headphones, |d| {
                        d.child(link_button("headphones-ok", "OK", &t, window, cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.model.call.headphones = false;
                                cx.notify();
                            }),
                        ))
                    })
                } else {
                    d
                }
            })
            // `margin-left: auto`: a spacer, then the mode, which gives up its width first next to a notice.
            .child(div().flex_1())
            .child(
                div()
                    .debug_selector(|| "call-filter".into())
                    .min_w_0()
                    .flex_shrink()
                    .ellipsis()
                    .child(filter),
            );
        let line = div().relative().h(px(28.)).child(layer);

        let content = if compact {
            div()
                .id("call-pill")
                .debug_selector(|| "call-pill".into())
                .flex()
                .h(px(36.))
                .min_w_0()
                .items_center()
                .gap(px(8.))
                .pl(px(12.))
                .pr(px(14.))
                .text_step(TEXT_SM)
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.bar_hover(true, cx)))
                .child(
                    icon(if muted || forced { "micOff" } else { "mic" }).text_color(if muted {
                        t.danger_fg
                    } else {
                        t.fg_2
                    }),
                )
                .child(
                    div()
                        .max_w(px(160.))
                        .min_w_0()
                        .ellipsis()
                        .font_weight(t.weight_strong)
                        .text_color(t.fg)
                        .child(room),
                )
                .child(
                    div()
                        .text_step(TEXT_XS)
                        .text_color(t.fg_3)
                        .child(motion::roll("call-time", &time, px(16.), false, window, cx)),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .child(row)
                .child(line)
                .into_any_element()
        };
        let (box_size, inner) = (self.bar.size.clone(), self.bar.inner.clone());
        let bar = div()
            .id("call-bar")
            .debug_selector(|| "call-bar".into())
            .relative()
            .flex()
            .flex_col()
            .min_w_0()
            .max_w_full()
            .border_1()
            .border_color(t.line)
            .bg(t.bg_raised)
            .shadow(t.shadow_float.clone())
            .on_hover(cx.listener(|this, on: &bool, _, cx| this.bar_hover(*on, cx)))
            .child(
                canvas(move |b, _, _| box_size.set(b.size), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .map(|d| match morph {
                None => d.rounded(radius).child(content),
                Some((from, r0, x)) => {
                    // The content at its own size in a layer the box clips, measured on its first frame.
                    let e = motion::ease(t.motion.ease_out)(x);
                    let fade = motion::ease(t.motion.ease_out)(
                        (x * t.motion.dur_3.as_secs_f32() / t.motion.dur_2.as_secs_f32()).min(1.),
                    );
                    let to = inner.get();
                    let to = if to.width > px(0.) {
                        to + size(px(2.), px(2.))
                    } else {
                        from
                    };
                    let mid = |a: Pixels, b: Pixels| a + (b - a) * e;
                    d.overflow_hidden()
                        .w(mid(from.width, to.width))
                        .h(mid(from.height, to.height))
                        .rounded(mid(r0, radius))
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .bottom_0()
                                .opacity(fade)
                                .child(
                                    canvas(move |b, _, _| inner.set(b.size), |_, _, _, _| {})
                                        .absolute()
                                        .size_full(),
                                )
                                .child(content),
                        )
                }
            });
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                // `rise`: from 6 px lower, fading in.
                .bottom(px(12. - 6. * (1. - pose.map_or(1., |p| p.t))))
                .flex()
                .justify_center()
                .px(px(16.))
                .when_some(pose, |d, p| d.opacity(p.opacity))
                .child(bar),
        )
    }
}

impl Vesper {
    /// The pointer came onto the bar (it opens, as a pill) or left it (it closes 300 ms later).
    pub(super) fn bar_hover(&mut self, on: bool, cx: &mut Context<Self>) {
        self.bar.leave = None;
        if on {
            if !self.bar.peek {
                self.bar.peek = true;
                cx.notify();
            }
            return;
        }
        self.bar.leave = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let _ = this.update(cx, |v, cx| {
                v.bar.peek = false;
                cx.notify();
            });
        }));
    }
}

/// The web's status line wording for the engine's noise filter status: `Noise suppression: <mode>`, and why it is not
/// DeepFilterNet in brackets.
pub fn noise_line(filter: &str) -> String {
    use crate::voice::denoise::{DEEPFILTER, GATE, LOADING, OFF};
    let (mode, why) = match filter {
        "" => return String::new(),
        DEEPFILTER => ("deepfilter", ""),
        GATE => ("gate", ""),
        OFF => ("off", ""),
        LOADING => ("gate", "DeepFilterNet is loading"),
        other => ("gate", other.trim_end_matches('.')),
    };
    if why.is_empty() {
        format!("Noise suppression: {mode}")
    } else {
        format!("Noise suppression: {mode} ({why})")
    }
}

/// The push-to-talk pill: both texts take the same place, so it keeps its width while the key goes down and up.
fn ptt_pill(key: &str, held: bool, t: &super::theme::Tokens) -> Div {
    let idle = div()
        .when(held, |d| d.invisible())
        .child(format!("Push to talk: hold {key}"));
    let talking = div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .when(!held, |d| d.invisible())
        .child(div().size(px(6.)).rounded_full().bg(t.accent))
        .child("Talking");
    div()
        .debug_selector(|| "call-ptt".into())
        .relative()
        .ml(px(4.))
        .flex()
        .h(px(28.))
        .flex_none()
        .items_center()
        .rounded(t.radius_sm)
        .border_1()
        .border_color(t.line_subtle)
        .bg(t.bg_element)
        .px(px(8.))
        .text_step(TEXT_XS)
        .text_color(if held { t.fg } else { t.fg_2 })
        .child(idle)
        .child(talking)
}

/// The bar's and the card's underlined text buttons (`.link`): `--fg-2`, `--fg` on hover.
pub(super) fn link_button(
    id: &'static str,
    text: &'static str,
    t: &super::theme::Tokens,
    window: &mut Window,
    cx: &mut Context<Vesper>,
) -> gpui::Stateful<Div> {
    let (h, on_hover) = motion::hover(id, window, cx);
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex_none()
        .underline()
        .text_color(mix(t.fg_2.into(), t.fg.into(), h))
        .cursor_pointer()
        .on_hover(on_hover)
        .child(text)
}

#[cfg(test)]
mod tests {
    use super::{clock, noise_line};
    use crate::voice::denoise::{DEEPFILTER, LOADING};
    use std::time::Duration;

    #[test]
    fn the_call_timer_counts_minutes_then_hours() {
        let s = Duration::from_secs;
        assert_eq!(clock(s(0)), "0:00");
        assert_eq!(clock(s(61)), "1:01");
        assert_eq!(clock(s(3599)), "59:59");
        assert_eq!(clock(s(3600)), "1:00:00");
        assert_eq!(clock(s(36061)), "10:01:01");
    }

    #[test]
    fn the_noise_line_reads_like_the_web() {
        assert_eq!(noise_line(DEEPFILTER), "Noise suppression: deepfilter");
        assert_eq!(
            noise_line(LOADING),
            "Noise suppression: gate (DeepFilterNet is loading)"
        );
        assert_eq!(
            noise_line(
                "DeepFilterNet is not available (404), so the noise gate runs for this call."
            ),
            "Noise suppression: gate (DeepFilterNet is not available (404), so the noise gate runs for this call)"
        );
        assert_eq!(noise_line(""), "");
    }
}
