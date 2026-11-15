//! `PersonCard.svelte`: one person, opened from a name or an avatar (the timeline, the sidebar's voice rows, the call
//! bar). Who they are, where they are in voice, their connection and volume when they are in your call, Message, and
//! the moderator voice actions when you outrank them. A popover next to what was clicked (`place.ts`: to the right,
//! or above from the call bar, clear of the composer), clamped 8 px inside the window; it fades in (`pop`: the web
//! also scales from 0.97, which GPUI cannot do to a box).
//!
//! Not here: Esc to close and focus handling (a click outside closes it), the switcher's dialog form.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Bounds, Context, Corner, Div, ElementId, InteractiveElement, IntoElement,
    ParentElement, Pixels, StatefulInteractiveElement, Styled, Window, anchored, canvas, deferred,
    div, point, px, transparent_black,
};

use super::Vesper;
use super::c::{self, Avatar, Ellipsis, mix};
use super::call::link_button;
use super::icons::icon;
use super::motion::{self, POP};
use super::theme::{TEXT_BASE, TEXT_XS, TextStepExt, Tokens, tokens};
use crate::link::Link;
use crate::model::Phase;

/// Where each opener was drawn last frame, by key (`bar-<id>`, `person-<room>-<id>`, `name-<message>`, `composer`).
pub type Anchors = Rc<RefCell<HashMap<String, Bounds<Pixels>>>>;

/// The open card.
pub struct Card {
    pub user_id: String,
    anchor: String,
    /// Opened from the call bar: above it.
    top: bool,
    since: Instant,
    /// The Disconnect confirm is open.
    confirm: bool,
}

impl Vesper {
    /// An invisible layer over its parent that records the parent's bounds as `key`, for a card opened from it.
    pub(super) fn probe(&self, key: String) -> impl IntoElement {
        let anchors = self.anchors.clone();
        canvas(
            move |b, _, _| {
                anchors.borrow_mut().insert(key, b);
            },
            |_, _, _, _| {},
        )
        // Insets, not a full size: an absolute child without them starts inside the parent's padding.
        .absolute()
        .inset_0()
    }

    /// Opens `id`'s card at `anchor`, or closes it when it is open there (`togglePerson`).
    pub(super) fn open_person(
        &mut self,
        id: String,
        anchor: String,
        top: bool,
        cx: &mut Context<Self>,
    ) {
        let same = self
            .person
            .as_ref()
            .is_some_and(|c| c.user_id == id && c.anchor == anchor);
        // A press outside on the same opener closed it just now: this click must not open it again.
        let just = self
            .person_closed
            .take()
            .is_some_and(|(a, at)| a == anchor && at.elapsed() < Duration::from_millis(500));
        self.person = (!same && !just).then(|| Card {
            user_id: id,
            anchor,
            top,
            since: Instant::now(),
            confirm: false,
        });
        self.model.person_error.clear();
        cx.notify();
    }

    pub(super) fn close_person(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = self.person.take() {
            self.person_closed = Some((c.anchor, Instant::now()));
        }
        cx.notify();
    }

    /// The card, over everything, next to its opener.
    pub(super) fn render_person(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let card = self.person.as_ref()?;
        let b = *self.anchors.borrow().get(&card.anchor)?;
        let t = tokens(cx).clone();
        let now = Instant::now();
        let pose = motion::pose(&t.motion, POP, false, card.since, now);
        if pose.is_some() {
            window.request_animation_frame();
        }
        let (pos, corner) = if card.top {
            // Above the opener and above the composer too.
            let composer = self.anchors.borrow().get("composer").map(|c| c.origin.y);
            let y = composer.map_or(b.origin.y, |c| c.min(b.origin.y));
            (point(b.origin.x, y - px(4.)), Corner::BottomLeft)
        } else {
            (
                point(b.origin.x + b.size.width + px(4.), b.origin.y),
                Corner::TopLeft,
            )
        };
        let body = self.person_body(window, cx)?;
        Some(
            deferred(
                anchored()
                    .position(pos)
                    .anchor(corner)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        c::popover_frame(cx)
                            .id("person-card")
                            .debug_selector(|| "person-card".into())
                            .opacity(pose.map_or(1., |p| p.opacity))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                // Not while its confirm is up: that dialog is outside the card.
                                if !this.person.as_ref().is_some_and(|c| c.confirm) {
                                    this.close_person(cx);
                                }
                            }))
                            .child(body),
                    ),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn person_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let t = tokens(cx).clone();
        let id = self.person.as_ref()?.user_id.clone();
        let m = &self.model;
        let s = &m.state;
        let me = s.my_id().to_string();
        let own = id == me;
        let user = s
            .users
            .get(&id)
            .cloned()
            .or_else(|| s.me.as_ref().filter(|_| own).map(|m| m.user.clone()));
        let peer = s.voice.iter().find(|p| p.user_id == id).cloned();
        let name = user
            .as_ref()
            .map(|u| u.display_name.clone())
            .or_else(|| peer.as_ref().map(|p| p.name.clone()))
            .unwrap_or_else(|| "Someone".into());
        let c = &m.call;
        let joined = c.phase == Phase::Joined;
        // The server does not echo your own join: your place comes from the call.
        let room_id = if own {
            c.room.filter(|_| joined)
        } else {
            peer.as_ref().map(|p| p.room_id)
        };
        let where_ = match room_id {
            None => "Not in voice".to_string(),
            Some(r) => format!(
                "In voice in {}",
                s.find_room(r).map_or("a room".into(), |r| r.name.clone())
            ),
        };
        let in_call = !own && joined && room_id.is_some() && room_id == c.room;
        let link: Option<Link> = (joined && (own || in_call)).then(|| {
            if own {
                c.own_link
            } else {
                c.links.get(&id).copied().unwrap_or_default()
            }
        });
        let volume = m.volume(&id).round();
        let outranks = m.outranks(&id);
        let busy = m.person_busy;
        let error = m.person_error.clone();
        let role = user.as_ref().map(|u| {
            let r = match u.role {
                vesper_protocol::Role::Member => "Member",
                vesper_protocol::Role::Moderator => "Moderator",
                vesper_protocol::Role::Owner => "Owner",
            };
            format!(
                "@{} · {r}{}",
                u.username,
                if u.banned { " · Banned" } else { "" }
            )
        });
        let (muted, deafened) = peer
            .as_ref()
            .map_or((false, false), |p| (p.muted || p.force_muted, p.deafened));

        let head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .when_some(user.clone(), |d, u| {
                d.child(
                    Avatar::new(u.user_id, u.display_name, u.accent, 40)
                        .muted(muted)
                        .deafened(deafened)
                        .surface(t.bg_raised),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .ellipsis()
                            .text_step(TEXT_BASE)
                            .font_weight(t.weight_strong)
                            .text_color(t.fg)
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .ellipsis()
                            .text_step(TEXT_XS)
                            .text_color(t.fg_2)
                            .child(role.unwrap_or_default()),
                    ),
            );
        let bio = user
            .as_ref()
            .map(|u| u.bio.clone())
            .filter(|b| !b.is_empty());
        let mut body = div()
            .flex()
            .flex_col()
            .w(px(272.))
            .gap(px(12.))
            .p(px(8.))
            .child(head)
            .when_some(bio, |d, b| d.child(div().text_color(t.fg).child(b)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_color(t.fg_2)
                    .child(
                        icon(if room_id.is_none() {
                            "headphonesOff"
                        } else {
                            "headphones"
                        })
                        .text_color(t.fg_2),
                    )
                    .child(where_),
            );
        if let Some(l) = link {
            let label = l.grade.map(|g| format!("Connection {}. ", g.word()));
            body = body.child(
                div()
                    .debug_selector(|| "person-link".into())
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_color(t.fg_2)
                    .child(
                        div()
                            .flex()
                            .h(px(20.))
                            .items_center()
                            .child(link_icon(&l, &t)),
                    )
                    .child(
                        div().flex_1().min_w_0().child(
                            gpui::StyledText::new(format!(
                                "{}{}",
                                label.clone().unwrap_or_default(),
                                l.detail()
                            ))
                            .with_highlights(label.map(|x| {
                                (
                                    0..x.len(),
                                    gpui::HighlightStyle {
                                        color: Some(t.fg.into()),
                                        ..Default::default()
                                    },
                                )
                            })),
                        ),
                    ),
            );
        }
        if !own && user.as_ref().is_some_and(|u| !u.banned) {
            let uid = id.clone();
            body = body.child(
                act_row(
                    "person-message",
                    "send",
                    "Message",
                    false,
                    !busy.is_empty(),
                    &t,
                    window,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let uid = uid.clone();
                    this.act(|m| m.person_act(&uid, "dm", None), cx)
                })),
            );
        }
        if in_call {
            let state = self.slider(&id, cx);
            let uid = id.clone();
            let reset = (volume != 100.).then(|| {
                link_button("person-volume-reset", "Reset", &t, window, cx).on_click(cx.listener(
                    move |this, _, window, cx| {
                        let uid = uid.clone();
                        this.act(|m| m.set_volume(&uid, 100.), cx);
                        this.save_settings();
                        if let Some((s, _)) = this.sliders.get(&uid) {
                            s.update(cx, |s, cx| s.set_value(100., window, cx));
                        }
                    },
                ))
            });
            let sel = id.clone();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .border_t_1()
                    .border_color(t.line_subtle)
                    .pt(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(div().flex_1().text_color(t.fg_2).child("Volume"))
                            .when(volume > 100., |d| {
                                d.child(
                                    div()
                                        .text_step(TEXT_XS)
                                        .text_color(t.fg_3)
                                        .child("Can distort"),
                                )
                            })
                            .child(div().text_color(t.fg).child(format!("{volume} %")))
                            .children(reset),
                    )
                    .child(
                        div()
                            .debug_selector(move || format!("volume-{sel}"))
                            .child(c::range(&state, cx)),
                    ),
            );
        }
        if outranks {
            let mut acts = div()
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(t.line_subtle)
                .pt(px(8.));
            if let Some(p) = &peer {
                let forced = p.force_muted;
                let uid = id.clone();
                acts = acts.child(
                    act_row(
                        "person-force-mute",
                        if forced { "mic" } else { "micOff" },
                        if forced {
                            "Allow to speak"
                        } else {
                            "Mute for everyone"
                        },
                        false,
                        !busy.is_empty(),
                        &t,
                        window,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let uid = uid.clone();
                        this.act(|m| m.person_act(&uid, "force-mute", Some(!forced)), cx)
                    })),
                );
                if p.share.is_some() {
                    let uid = id.clone();
                    acts = acts.child(
                        act_row(
                            "person-stop-share",
                            "monitor",
                            "Stop share",
                            false,
                            !busy.is_empty(),
                            &t,
                            window,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let uid = uid.clone();
                            this.act(|m| m.person_act(&uid, "stop-share", None), cx);
                            this.close_person(cx);
                        })),
                    );
                }
                let uid = id.clone();
                acts = acts.child(
                    act_row(
                        "person-kick",
                        "phoneOff",
                        "Remove from voice",
                        false,
                        !busy.is_empty(),
                        &t,
                        window,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let uid = uid.clone();
                        this.act(|m| m.person_act(&uid, "kick", None), cx);
                        this.close_person(cx);
                    })),
                );
            }
            acts = acts.child(
                act_row(
                    "person-disconnect",
                    "logOut",
                    "Disconnect",
                    true,
                    !busy.is_empty(),
                    &t,
                    window,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let Some(c) = &mut this.person {
                        c.confirm = true;
                    }
                    cx.notify();
                })),
            );
            body = body.child(acts);
        }
        if !error.is_empty() {
            body = body.child(
                div()
                    .debug_selector(|| "person-error".into())
                    .text_color(t.danger_fg)
                    .child(error),
            );
        }
        Some(body)
    }

    /// The Disconnect confirm over the window (`Dialog`), while asked.
    pub(super) fn person_confirm(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let card = self.person.as_ref().filter(|c| c.confirm)?;
        let id = card.user_id.clone();
        let name = self.model.state.name_of(&id);
        let busy = self.model.person_busy == "disconnect";
        let cancel = cx.listener(|this, _, _, cx| {
            if let Some(c) = &mut this.person {
                c.confirm = false;
            }
            cx.notify();
        });
        Some(
            div().debug_selector(|| "person-confirm".into()).absolute().inset_0().child(
                c::Dialog::new(format!("Disconnect {name}?"))
                    .on_close(cx.listener(|this, _, _, cx| {
                        if let Some(c) = &mut this.person {
                            c.confirm = false;
                        }
                        cx.notify();
                    }))
                    .child("Every open tab of theirs loses its connection to the server. They can connect again.")
                    .footer(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                c::Button::new("person-confirm-cancel", "Cancel")
                                    .variant(c::Variant::Ghost)
                                    .on_click(cancel),
                            )
                            .child(div().debug_selector(|| "person-confirm-disconnect".into()).child(
                                c::Button::new("person-confirm-disconnect", "Disconnect")
                                    .variant(c::Variant::Danger)
                                    .loading(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let uid = id.clone();
                                        this.act(|m| m.person_act(&uid, "disconnect", None), cx);
                                        this.person = None;
                                        cx.notify();
                                    })),
                            )),
                    ),
            ),
        )
    }
}

/// `LinkIcon.svelte`: the bars of a grade, all faint while measuring; poor in the warning colour.
pub fn link_icon(l: &Link, t: &Tokens) -> Div {
    let poor = l.grade == Some(crate::link::Grade::Poor);
    c::bars(
        l.grade.map_or(0, crate::link::Grade::bars),
        if poor { t.warn_fg } else { t.fg_3 },
    )
}

/// The card's `.act` button: a 32 px row 8 px past the card's padding each side, `--radius-sm`, the icon in `--fg-2`,
/// hover `--bg-hover` (`--dur-1`); `danger` all in the danger colour. Busy, it ignores clicks.
#[allow(clippy::too_many_arguments)]
fn act_row(
    id: &'static str,
    name: &'static str,
    text: &'static str,
    danger: bool,
    busy: bool,
    t: &Tokens,
    window: &mut Window,
    cx: &mut Context<Vesper>,
) -> gpui::Stateful<Div> {
    let (h, on_hover) = motion::hover(id, window, cx);
    let h = if busy { 0. } else { h };
    let (fg, mark) = if danger {
        (t.danger_fg, t.danger_fg)
    } else {
        (t.fg, t.fg_2)
    };
    div()
        .id(ElementId::Name(id.into()))
        .debug_selector(move || id.into())
        .flex()
        .h(px(32.))
        .items_center()
        .gap(px(8.))
        .mx(px(-8.))
        .px(px(8.))
        .rounded(t.radius_sm)
        .text_color(fg)
        .bg(mix(transparent_black(), t.bg_hover.into(), h))
        .on_hover(on_hover)
        .child(icon(name).text_color(mark))
        .child(div().min_w_0().ellipsis().child(text))
}
