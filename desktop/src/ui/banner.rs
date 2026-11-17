//! `ConnectionBanner.svelte` at the top of the main pane (the socket's state: it pushes the page down, never covers
//! it; in and out with `swap`), and `RingPrompt.svelte` at the bottom right (an incoming DM call: the caller,
//! Accept and Decline; it rises in, and a ring pulses around the picture, 1.6 s ease-out, none under reduced motion).
//!
//! Esc declines it (`Vesper::dismiss`); its sound is `sound.rs`. Not here: the first connect's calm "Connecting to the
//! server..." line (nothing shows until the first hello), the screen reader texts.

use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::{Context, Div, Hsla, InteractiveElement, ParentElement, Styled, Task, Window, div, px};

use super::Vesper;
use super::c::{self, Avatar, Ellipsis};
use super::icons::icon;
use super::motion::{self, RISE, SWAP};
use super::theme::{TEXT_BASE, TEXT_XS, TextStepExt, tokens};

/// The ring's pulse.
pub const PULSE: Duration = Duration::from_millis(1600);

/// The banner's own state: what it says now, since when, and what it said as it goes.
#[derive(Default)]
pub struct Banner {
    shown: Option<(String, bool, Instant)>,
    leaving: Option<(String, bool, Instant)>,
    tick: Option<Task<()>>,
    /// Online once since sign in: before that a drop is the first connect, which shows nothing.
    was_online: bool,
    /// The ring shown and since when, for its rise.
    ring: Option<(i64, Instant)>,
}

/// `Connection lost. Trying again in 3 s.`: whole seconds, at least 1.
pub fn retry_text(left: Duration) -> String {
    let s = left.as_millis().div_ceil(1000).max(1);
    format!("Connection lost. Trying again in {s} s.")
}

/// The pulse at `since` into it: (spread in px, opacity of the accent at 60 %), the CSS `ringing` keyframes.
pub fn pulse(ease: [f32; 4], since: Duration) -> (f32, f32) {
    let x = (since.as_secs_f32() / PULSE.as_secs_f32()).fract();
    let e = motion::ease(ease)(x);
    (10. * e, 0.6 * (1. - e))
}

impl Vesper {
    /// The banner, while offline or after a moderator's disconnect (`retry` with Try now, `now`, or the notice).
    pub(super) fn render_banner(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let t = tokens(cx).clone();
        let m = &self.model;
        let now = Instant::now();
        self.banner.was_online |= m.online;
        let want: Option<(String, bool)> = if !m.notice.is_empty() {
            Some((m.notice.clone(), false))
        } else if m.online || !self.banner.was_online || m.call.resuming.is_some() {
            // A call that resumes on the next socket shows only the call bar's quiet line.
            None
        } else {
            Some(match m.retry_at {
                Some(at) => (retry_text(at.saturating_duration_since(now)), true),
                None => ("Connection lost. Trying again now.".into(), false),
            })
        };
        let b = &mut self.banner;
        match (&want, &mut b.shown) {
            (Some((text, retry)), Some(s)) => (s.0, s.1) = (text.clone(), *retry),
            (Some((text, retry)), None) => b.shown = Some((text.clone(), *retry, now)),
            (None, Some(_)) => {
                let (text, retry, _) = b.shown.take().expect("shown");
                b.leaving = Some((text, retry, now));
            }
            (None, None) => {}
        }
        // The countdown ticks while it shows.
        if want.as_ref().is_some_and(|w| w.1) {
            if b.tick.is_none() {
                b.tick = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(250))
                            .await;
                        if this.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
        } else {
            b.tick = None;
        }
        let mo = &t.motion;
        let (text, retry, pose) = if let Some((text, retry, at)) = &b.shown {
            (
                text.clone(),
                *retry,
                motion::pose(mo, SWAP, false, *at, now),
            )
        } else {
            let (text, retry, at) = b.leaving.clone()?;
            match motion::pose(mo, SWAP, true, at, now) {
                Some(p) => (text, retry, Some(p)),
                None => {
                    b.leaving = None;
                    return None;
                }
            }
        };
        if pose.is_some() {
            window.request_animation_frame();
        }
        Some(
            div()
                .debug_selector(|| "connection-banner".into())
                .relative()
                .when_some(pose, |d, p| d.top(px(4. * (1. - p.t))).opacity(p.opacity))
                .flex()
                .flex_none()
                .min_h(px(40.))
                .items_center()
                .gap(px(8.))
                .pl(px(16.))
                .pr(px(8.))
                .border_b_1()
                .border_color(t.line_subtle)
                .bg(t.warn_subtle)
                .text_color(t.fg)
                .child(div().flex().child(icon("warn").text_color(t.warn_fg)))
                .child(div().min_w_0().child(text))
                .when(retry, |d| {
                    d.child(
                        div().debug_selector(|| "retry-now".into()).child(
                            c::Button::new("retry-now", "Try now")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.run(vec![crate::net::Cmd::RetryNow]);
                                    cx.notify();
                                })),
                        ),
                    )
                }),
        )
    }

    /// A newer release (`update.rs`): the web's calm bar, with Update and restart. Nothing installs without the click.
    pub(super) fn render_update(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        use crate::model::Update;
        let t = tokens(cx).clone();
        let (text, busy) = match &self.model.update {
            Update::None => return None,
            Update::Offered(o) => (format!("Vesper {} is ready.", o.version), false),
            Update::Installing(o) => (format!("Downloading Vesper {}.", o.version), true),
            Update::Failed(_, why) => (why.clone(), false),
        };
        Some(
            div()
                .debug_selector(|| "update-banner".into())
                .flex()
                .flex_none()
                .min_h(px(40.))
                .items_center()
                .gap(px(8.))
                .pl(px(16.))
                .pr(px(8.))
                .border_b_1()
                .border_color(t.line_subtle)
                .bg(t.bg_sidebar)
                .text_color(t.fg_2)
                .child(div().flex().child(icon("info").text_color(t.fg_3)))
                .child(div().min_w_0().child(text))
                .child(
                    div().debug_selector(|| "update-restart".into()).child(
                        c::Button::new("update-restart", "Update and restart")
                            .small()
                            .loading(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let cmds = this.model.install_update();
                                this.run(cmds);
                                cx.notify();
                            })),
                    ),
                ),
        )
    }

    /// The first ring, bottom right over everything.
    pub(super) fn render_ring(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let Some((room, caller)) = self.model.rings.first().cloned() else {
            self.banner.ring = None;
            return None;
        };
        let t = tokens(cx).clone();
        let now = Instant::now();
        let since = match self.banner.ring {
            Some((r, at)) if r == room => at,
            _ => {
                self.banner.ring = Some((room, now));
                now
            }
        };
        let mo = &t.motion;
        let rise = motion::pose(mo, RISE, false, since, now);
        // The pulse never stops, so frames keep coming while it shows (none under reduced motion).
        if rise.is_some() || !mo.reduced {
            window.request_animation_frame();
        }
        let s = &self.model.state;
        let (name, accent) = s
            .users
            .get(&caller)
            .map_or(("Someone".to_string(), String::new()), |u| {
                (u.display_name.clone(), u.accent.clone())
            });
        // The keyframes' `box-shadow: 0 0 0 <spread>` as a disc behind the picture (a GPUI shadow without blur draws
        // nothing).
        let ring = (!mo.reduced).then(|| {
            let (spread, alpha) = pulse(mo.ease_out, now.duration_since(since));
            let mut color: Hsla = t.accent.into();
            color.a *= alpha;
            div()
                .absolute()
                .left(px(-spread))
                .top(px(-spread))
                .size(px(40. + 2. * spread))
                .rounded_full()
                .bg(color)
        });
        Some(
            div()
                .debug_selector(|| "ring-prompt".into())
                .occlude()
                .absolute()
                .right(px(16.))
                .bottom(px(16. - 6. * (1. - rise.map_or(1., |p| p.t))))
                .when_some(rise, |d, p| d.opacity(p.opacity))
                .flex()
                .items_center()
                .gap(px(12.))
                .w(px(420.))
                .p(px(12.))
                .border_1()
                .border_color(t.line)
                .rounded(t.radius)
                .bg(t.bg_raised)
                .shadow(t.shadow_float.clone())
                .child(
                    div()
                        .debug_selector(|| "ring-pulse".into())
                        .relative()
                        .flex()
                        .flex_none()
                        .children(ring)
                        .child(
                            Avatar::new(caller.clone(), name.clone(), accent, 40)
                                .surface(t.bg_raised),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .ellipsis()
                                .text_step(TEXT_BASE)
                                .font_weight(t.weight_strong)
                                .text_color(t.fg)
                                .child(name),
                        )
                        .child(
                            div()
                                .text_step(TEXT_XS)
                                .text_color(t.fg_2)
                                .child("is calling you in a direct message"),
                        ),
                )
                .child(
                    div().debug_selector(|| "ring-accept".into()).child(
                        c::Button::new("ring-accept", "Accept")
                            .primary()
                            .small()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let cmds = this.model.accept_ring(room);
                                this.run(cmds);
                                this.open_room(room, window, cx);
                            })),
                    ),
                )
                .child(
                    div().debug_selector(|| "ring-decline".into()).child(
                        c::Button::new("ring-decline", "Decline")
                            .variant(c::Variant::Danger)
                            .small()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.act(|m| m.decline_ring(room), cx)
                            })),
                    ),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_countdown_and_the_pulse() {
        assert_eq!(
            retry_text(Duration::from_millis(2100)),
            "Connection lost. Trying again in 3 s."
        );
        assert_eq!(
            retry_text(Duration::ZERO),
            "Connection lost. Trying again in 1 s."
        );
        let ease = [0.2, 0., 0., 1.];
        assert_eq!(pulse(ease, Duration::ZERO), (0., 0.6));
        let (s, a) = pulse(ease, Duration::from_millis(800));
        assert!(s > 5. && s < 10. && a > 0. && a < 0.3, "{s} {a}");
        // It repeats every 1.6 s.
        assert_eq!(pulse(ease, PULSE), (0., 0.6));
    }
}
