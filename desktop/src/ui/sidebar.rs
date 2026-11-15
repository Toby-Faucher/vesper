//! The sidebar, after the web's `Sidebar.svelte`, `RoomRow.svelte` and `PresenceList.svelte`: the mark and the hide
//! button, the DMs and the rooms with their unread pills and the people in voice, and you with the account menu and
//! the Settings gear. Also the sidebar's motion: the selected room's glide, the hide and show slide, people joining
//! and leaving voice (`row`) and badges popping in (`bump`).
//!
//! A click on a person in voice opens their card (`person.rs`); the people in your call show their connection.
//!
//! Not here (no desktop feature behind them yet): the switcher button (Ctrl K), the half-strength fill of the row the pointer heads for (`intent.ts`).
//!
//! No `ml_auto()` here: taffy drops the flex gaps before an auto margin, so what the web pushes right with `ml-auto`
//! sits after a `flex_1` item instead.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, Corner, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement,
    Pixels, SharedString, StatefulInteractiveElement, Styled, Window, anchored, canvas, deferred,
    div, img, px, transparent_black,
};
use vesper_protocol::{Peer, RoomInfo, RoomKind};

use super::c::{Avatar, Badge, Ellipsis, IconButton, MenuItem, mix, popover_frame};
use super::icons::icon;
use super::motion::{self, BUMP, Pose, ROW, Roster, Tween};
use super::sign_in::logo;
use super::theme::{TEXT_SM, TEXT_XS, TextStepExt, tokens};
use super::{Vesper, theme};
use crate::model::Phase;

/// `w-62`.
pub const WIDTH: f32 = 248.;
/// More people than this fold into an avatar stack until the chevron opens them.
const FOLD_ABOVE: usize = 5;
/// Avatars drawn when folded; the rest is `+n`.
const STACK: usize = 8;

/// The sidebar's own state.
#[derive(Default)]
pub struct Side {
    pub account_open: bool,
    /// A press outside closed the menu: the click that ends that press on the account button must not open it again.
    menu_closed_at: Option<Instant>,
    /// The reader's fold choice per room; none follows the count.
    pub fold: HashMap<i64, bool>,
    /// How much of the sidebar is out: 1 shown, 0 hidden.
    slide: Option<Tween>,
    /// The selected fill's glide: from this row top, since then.
    glide: Option<(Pixels, Instant)>,
    /// Row tops of the last frame, by room id, and the list's top.
    rows: Rc<RefCell<(Pixels, HashMap<i64, Pixels>)>>,
    /// Per room, so a leaver's place is its index in that room.
    people: HashMap<i64, Roster<String, Peer>>,
    badges: Roster<(i64, bool, String), ()>,
    settled: bool,
    calm_until: Option<Instant>,
}

impl Side {
    /// The glide starts at the row of `from` (the room selected before).
    pub fn glide_from(&mut self, from: Option<i64>, reduced: bool) {
        let y = from.and_then(|id| self.rows.borrow().1.get(&id).copied());
        self.glide = y.filter(|_| !reduced).map(|y| (y, Instant::now()));
    }

    /// The selected fill is on its way to the new row.
    #[cfg(test)]
    pub fn gliding(&self) -> bool {
        self.glide.is_some()
    }

    /// The first hello is still settling (`settle` in motion.ts): nothing moves yet.
    pub fn calm(&self, now: Instant) -> bool {
        self.calm_until.is_none_or(|u| now < u)
    }

    /// Ends the first hello's calm now, for tests of motion.
    #[cfg(test)]
    pub fn unsettle(&mut self) {
        self.calm_until = Some(Instant::now());
    }

    /// How much of the sidebar shows now (0 to 1).
    pub fn shown(&self, now: Instant) -> f32 {
        self.slide.as_ref().map_or(1., |t| t.at(now))
    }
}

impl Vesper {
    /// Hides or shows the sidebar, sliding (`slide` in motion.ts: `--dur-3`, ease-out; none under reduced motion),
    /// and keeps the choice in the settings.
    pub(super) fn set_collapsed(&mut self, on: bool, cx: &mut Context<Self>) {
        let m = theme::tokens(cx).motion.clone();
        let t = self.side.slide.get_or_insert_with(|| Tween::new(1.));
        t.go(
            if on { 0. } else { 1. },
            Instant::now(),
            m.or(m.dur_3, Duration::ZERO),
            m.ease_out,
        );
        self.change_settings(|s| s.sidebar_collapsed = on, cx);
    }

    /// The sidebar slot and the main pane, side by side: the sidebar slides over the pane's left edge while the pane
    /// has its final width and follows the edge, as the web's transforms do.
    pub(super) fn shell(
        &mut self,
        main: impl IntoElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let now = Instant::now();
        if !self.side.settled {
            // As the web's settle(): what comes with the first hello shows without motion, and a saved hide does
            // not slide.
            self.side.settled = true;
            let calm = now + Duration::from_millis(1500);
            self.side.calm_until = Some(calm);
            self.side.badges.settle(calm);
            let shown = if self.model.settings.sidebar_collapsed {
                0.
            } else {
                1.
            };
            self.side.slide = Some(Tween::new(shown));
        }
        let shown = self.side.shown(now);
        if self.side.slide.as_ref().is_some_and(|t| t.running(now)) {
            window.request_animation_frame();
        }
        let width = window.viewport_size().width;
        let target = self.side.slide.as_ref().map_or(1., |t| t.target());
        let main_w = width - px(WIDTH * target);
        div()
            .size_full()
            .relative()
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(WIDTH * shown))
                    .w(main_w)
                    .flex()
                    .child(main),
            )
            .when(shown > 0., |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(WIDTH * (shown - 1.)))
                        .w(px(WIDTH))
                        .child(self.render_sidebar(window, cx)),
                )
            })
    }

    fn render_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let now = Instant::now();
        let t = tokens(cx).clone();
        let m = &self.model;
        let s = &m.state;
        // The people in voice per room, yourself first in your call (the server does not tell you about your own
        // join), and the badges, through the rosters for their motion.
        let me = s.me.as_ref().map(|me| me.user.clone());
        let in_call = (m.call.phase == Phase::Joined)
            .then_some(m.call.room)
            .flatten();
        let mine = me.as_ref().zip(in_call).map(|(u, room)| Peer {
            user_id: u.user_id.clone(),
            name: u.display_name.clone(),
            room_id: room,
            muted: m.call.muted,
            deafened: m.call.deafened,
            force_muted: m.call.force_muted,
            share: None,
        });
        let mut people: Vec<((i64, String), Peer)> = mine
            .iter()
            .chain(
                s.voice
                    .iter()
                    .filter(|p| Some(&p.user_id) != mine.as_ref().map(|x| &x.user_id)),
            )
            .map(|p| ((p.room_id, p.user_id.clone()), p.clone()))
            .collect();
        // Yours first within your room, the rest in the server's order.
        people.sort_by_key(|(k, _)| (k.0, mine.as_ref().is_none_or(|x| x.user_id != k.1)));
        let mo = t.motion.clone();
        let keep = if mo.reduced { mo.dur_calm } else { mo.dur_out };
        let ids: Vec<i64> = s.rooms.iter().chain(&s.dms).map(|r| r.id).collect();
        self.side.people.retain(|id, _| ids.contains(id));
        for id in ids {
            let r = self.side.people.entry(id).or_insert_with(|| {
                let mut r = Roster::default();
                if let Some(c) = self.side.calm_until {
                    r.settle(c);
                }
                r
            });
            let here = people
                .iter()
                .filter(|(k, _)| k.0 == id)
                .map(|(k, p)| (k.1.clone(), p.clone()))
                .collect();
            r.sync(now, keep, here);
        }
        let badges: Vec<_> = s
            .rooms
            .iter()
            .chain(&s.dms)
            .filter_map(|r| s.unread.get(&r.id).map(|u| (r.id, u)))
            .flat_map(|(id, u)| {
                [(false, u.unread as i64), (true, u.mentions as i64)]
                    .into_iter()
                    .filter(|(_, n)| *n > 0)
                    .map(move |(mention, n)| ((id, mention, Badge::text(mention, n)), ()))
            })
            .collect();
        self.side.badges.sync(now, Duration::ZERO, badges);
        let mut moving = false;
        let mut pose_of = |came: Option<Instant>, p, out| {
            let r = came.and_then(|at| motion::pose(&mo, p, out, at, now));
            moving |= r.is_some();
            r
        };

        let m = &self.model;
        let s = &m.state;
        let rooms: Vec<&RoomInfo> = s.rooms.iter().collect();
        let dms = s.sorted_dms();
        let row_of =
            |r: &RoomInfo,
             pose_of: &mut dyn FnMut(Option<Instant>, motion::Preset, bool) -> Option<Pose>,
             window: &mut Window,
             cx: &mut Context<Self>|
             -> AnyElement {
                let roster = self.side.people.get(&r.id);
                let mut list: Vec<(Peer, Option<Pose>, bool)> = people
                    .iter()
                    .filter(|(k, _)| k.0 == r.id)
                    .map(|(k, p)| {
                        let came = roster.and_then(|x| x.came(&k.1));
                        (p.clone(), pose_of(came, ROW, false), false)
                    })
                    .collect();
                for (_, p, ix, at) in roster.map_or(&[][..], |x| x.gone()) {
                    let pose = pose_of(Some(*at), ROW, true);
                    if pose.is_some() {
                        list.insert((*ix).min(list.len()), (p.clone(), pose, true));
                    }
                }
                let u = s.unread.get(&r.id);
                let (unread, mentions) = u.map_or((0, 0), |u| (u.unread as i64, u.mentions as i64));
                let badge = |mention: bool,
                             n: i64,
                             pose_of: &mut dyn FnMut(
                    Option<Instant>,
                    motion::Preset,
                    bool,
                ) -> Option<Pose>| {
                    let key = (r.id, mention, Badge::text(mention, n));
                    Badge::new(mention, n).pose(pose_of(self.side.badges.came(&key), BUMP, false))
                };
                let badges = [
                    (unread > 0).then(|| badge(false, unread, pose_of)),
                    (mentions > 0).then(|| badge(true, mentions, pose_of)),
                ];
                self.room_row(r, badges, list, window, cx)
            };
        let dm_rows: Vec<AnyElement> = dms
            .iter()
            .map(|r| row_of(r, &mut pose_of, window, cx))
            .collect();
        let room_rows: Vec<AnyElement> = rooms
            .iter()
            .map(|r| row_of(r, &mut pose_of, window, cx))
            .collect();

        // The glide: a fill under the rows slides from the old row to the new one.
        let rows = self.side.rows.clone();
        let glide = self.side.glide.and_then(|(from, at)| {
            let (top, tops) = &*rows.borrow();
            let to = *tops.get(&m.active?)?;
            let x = motion::fraction(at, now, mo.dur_2);
            (x < 1.).then(|| to - *top + (from - to) * (1. - motion::ease(mo.ease_out)(x)))
        });
        if glide.is_none() {
            self.side.glide = None;
        } else {
            moving = true;
        }
        if moving {
            window.request_animation_frame();
        }
        let heading = |text: &'static str| {
            div()
                .flex()
                .h(px(28.))
                .items_center()
                .px(px(8.))
                .text_step(TEXT_XS)
                .text_color(t.fg_3)
                .child(text)
        };
        let list = |rows: Vec<AnyElement>| div().flex().flex_col().gap(px(1.)).children(rows);
        let rooms_list = list(room_rows)
            .relative()
            .child(
                canvas(
                    move |b, _, _| rows.borrow_mut().0 = b.origin.y,
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .when_some(glide, |d, y| {
                // First child drawn: under every row's text.
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(y)
                        .h(px(28.))
                        .rounded(t.radius_sm)
                        .bg(t.bg_selected),
                )
            });
        let dm_list = (!dm_rows.is_empty()).then(|| list(dm_rows).mb(px(8.)));

        let me_user = s.me.as_ref().map(|m| m.user.clone());
        let (acc_h, acc_hover) = motion::hover("account", window, cx);
        let account = me_user.map(|u| {
            let open = self.side.account_open;
            div()
                .flex()
                .h(px(48.))
                .flex_none()
                .items_center()
                .gap(px(4.))
                .border_t_1()
                .border_color(t.line_subtle)
                .px(px(8.))
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .id("account")
                                .debug_selector(|| "account".into())
                                .flex()
                                .flex_1()
                                .h(px(32.))
                                .min_w_0()
                                .items_center()
                                .gap(px(8.))
                                .rounded(t.radius_sm)
                                .px(px(8.))
                                .bg(mix(transparent_black(), t.bg_hover.into(), acc_h))
                                .on_hover(acc_hover)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let just_closed = this
                                        .side
                                        .menu_closed_at
                                        .is_some_and(|t| t.elapsed() < Duration::from_millis(500));
                                    this.side.account_open =
                                        !this.side.account_open && !just_closed;
                                    this.side.menu_closed_at = None;
                                    cx.notify();
                                }))
                                .child(Avatar::new(
                                    u.user_id.clone(),
                                    u.display_name.clone(),
                                    u.accent.clone(),
                                    20,
                                ))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .ellipsis()
                                        .text_color(t.fg)
                                        .child(u.display_name.clone()),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_none()
                                        .child(icon("chevronDown").text_color(t.fg_2)),
                                ),
                        )
                        .when(open, |d| d.child(self.account_menu(cx))),
                )
                .child(
                    div().debug_selector(|| "open-settings".into()).child(
                        IconButton::new("open-settings", "settings")
                            .turn()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_settings(window, cx)),
                            ),
                    ),
                )
        });

        div()
            .flex()
            .flex_col()
            .h_full()
            .w(px(WIDTH))
            .border_r_1()
            .border_color(t.line_subtle)
            .bg(t.bg_sidebar)
            .child(
                div()
                    .flex()
                    .h(px(48.))
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .pr(px(8.))
                    .pl(px(16.))
                    .child(img(logo(t.fill)).h(px(28.)).w(px(59.)).flex_none())
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .justify_end()
                            .child(div().debug_selector(|| "open-switcher".into()).child(
                                IconButton::new("open-switcher", "command").on_click(cx.listener(
                                    |this, _, w, cx| {
                                        this.open_layer(super::find::Layer::Switcher, w, cx)
                                    },
                                )),
                            ))
                            .child(div().debug_selector(|| "hide-sidebar".into()).child(
                                IconButton::new("hide-sidebar", "sidebar").on_click(
                                    cx.listener(|this, _, _, cx| this.set_collapsed(true, cx)),
                                ),
                            )),
                    ),
            )
            .child(
                div()
                    .id("sidebar-rooms")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(8.))
                    .pb(px(12.))
                    .when_some(dm_list, |d, l| d.child(heading("Direct messages")).child(l))
                    .child(heading("Rooms"))
                    .map(|d| {
                        if rooms.is_empty() {
                            d.when(m.online, |d| {
                                d.child(
                                    div()
                                        .px(px(8.))
                                        .text_step(TEXT_SM)
                                        .text_color(t.fg_3)
                                        .child("No rooms yet."),
                                )
                            })
                        } else {
                            d.child(rooms_list)
                        }
                    }),
            )
            .children(account)
            .into_any_element()
    }

    /// The account menu over the account button (`Menu.svelte` placed above, 4 px off): Admin (staff only), Desktop
    /// app, Log out.
    fn account_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        deferred(
            anchored().anchor(Corner::BottomLeft).child(
                div().pb(px(4.)).child(
                    popover_frame(cx)
                        .id("account-menu")
                        .debug_selector(|| "account-menu".into())
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.side.account_open = false;
                            this.side.menu_closed_at = Some(Instant::now());
                            cx.notify();
                        }))
                        .when(self.my_role() != vesper_protocol::Role::Member, |d| {
                            d.child(
                                div().debug_selector(|| "menu-admin".into()).child(
                                    MenuItem::new("menu-admin", "Admin")
                                        .icon("shield")
                                        .on_select(cx.listener(|this, _, window, cx| {
                                            this.side.account_open = false;
                                            let to = super::settings::At::Admin(
                                                super::admin::Section::Invites,
                                            );
                                            this.open_page(to, window, cx)
                                        })),
                                ),
                            )
                        })
                        .child(
                            MenuItem::new("menu-desktop", "Desktop app")
                                .icon("download")
                                .on_select(cx.listener(|this, _, window, cx| {
                                    this.side.account_open = false;
                                    let to = super::settings::At::Settings(
                                        super::settings::Section::Desktop,
                                    );
                                    this.open_page(to, window, cx)
                                })),
                        )
                        .child(
                            MenuItem::new("sign-out", "Log out")
                                .icon("logOut")
                                .on_select(cx.listener(|this, _, window, cx| {
                                    this.side.account_open = false;
                                    this.sign_out(window, cx)
                                })),
                        ),
                ),
            ),
        )
        .with_priority(1)
    }

    /// One room: the row (name, shield, pills, the fold chevron) and the people in voice under it.
    fn room_row(
        &self,
        r: &RoomInfo,
        badges: [Option<Badge>; 2],
        people: Vec<(Peer, Option<Pose>, bool)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = tokens(cx).clone();
        let m = &self.model;
        let s = &m.state;
        let id = r.id;
        let current = m.active == Some(id) && !self.settings_open;
        let unread = s.unread.get(&id).is_some_and(|u| u.unread > 0);
        let live = people.iter().filter(|p| !p.2).count();
        let folded = self
            .side
            .fold
            .get(&id)
            .copied()
            .unwrap_or(live > FOLD_ABOVE);
        let row_id = SharedString::from(format!("room-{id}"));
        let (h, on_hover) = motion::hover(row_id.clone(), window, cx);
        let gliding = current && self.side.glide.is_some();
        let fill: Hsla = if current && !gliding {
            t.bg_selected.into()
        } else {
            mix(transparent_black(), t.bg_hover.into(), h)
        };
        let fg = if unread || current { t.fg } else { t.fg_2 };
        let meta = if current { t.fg_2 } else { t.fg_3 };
        let dm_user = (r.kind == RoomKind::Dm)
            .then(|| r.members.iter().find(|u| *u != s.my_id()))
            .flatten()
            .and_then(|u| s.users.get(u));
        let chevron_id = ElementId::Name(format!("fold-{id}").into());
        let (ch, chevron_hover) =
            motion::transition(chevron_id.clone(), Duration::ZERO, window, cx);
        let rows = self.side.rows.clone();
        let row = div()
            .id(row_id.clone())
            .debug_selector(|| format!("room-{id}"))
            .relative()
            .flex()
            .h(px(28.))
            .items_center()
            .rounded(t.radius_sm)
            .bg(fill)
            .text_color(fg)
            .when(unread, |d| d.font_weight(t.weight_strong))
            .cursor_pointer()
            .on_hover(on_hover)
            .on_click(cx.listener(move |this, _, window, cx| this.open_room(id, window, cx)))
            .child(
                canvas(
                    move |b, _, _| {
                        rows.borrow_mut().1.insert(id, b.origin.y);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .flex()
                    .h_full()
                    .min_w_0()
                    .flex_1()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .when_some(dm_user, |d, u| {
                        d.child(Avatar::new(
                            u.user_id.clone(),
                            u.display_name.clone(),
                            u.accent.clone(),
                            20,
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap(px(6.))
                            .child(div().min_w_0().ellipsis().child(r.name.clone()))
                            .when(r.kind == RoomKind::Room && r.restricted, |d| {
                                d.child(icon("shield").text_color(meta))
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .gap(px(4.))
                            .pl(px(4.))
                            .children(badges.into_iter().flatten()),
                    ),
            )
            .when(live > 0, |d| {
                d.child(
                    div()
                        .id(chevron_id)
                        .mr(px(4.))
                        .flex()
                        .size(px(20.))
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(t.radius_sm)
                        .on_hover(chevron_hover)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.side.fold.insert(id, !folded);
                            cx.notify();
                        }))
                        .child(
                            icon(if folded {
                                "chevronRight"
                            } else {
                                "chevronDown"
                            })
                            .text_color(mix(
                                meta.into(),
                                t.fg.into(),
                                ch,
                            )),
                        ),
                )
            });
        div()
            .flex()
            .flex_col()
            .child(row)
            .when(!people.is_empty(), |d| {
                d.child(self.presence(id, people, folded, window, cx))
            })
            .into_any_element()
    }

    /// `PresenceList.svelte`: the people in voice under a room row, or folded into a stack of avatars. A person who
    /// comes or goes slides in or out 8 px from the left.
    fn presence(
        &self,
        room: i64,
        people: Vec<(Peer, Option<Pose>, bool)>,
        folded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = tokens(cx).clone();
        let m = &self.model;
        let s = &m.state;
        let speaking = |p: &Peer| {
            m.call.room == Some(room)
                && m.call.phase == Phase::Joined
                && (if p.user_id == s.my_id() {
                    m.call.me_speaking
                } else {
                    m.call.speaking.contains(&p.user_id)
                })
        };
        let accent = |p: &Peer| {
            s.users
                .get(&p.user_id)
                .map_or(String::new(), |u| u.accent.clone())
        };
        let link = |p: &Peer| {
            (m.call.room == Some(room) && m.call.phase == Phase::Joined)
                .then(|| {
                    if p.user_id == s.my_id() {
                        m.call.quality.map(|_| m.call.own_link)
                    } else {
                        m.call.links.get(&p.user_id).copied()
                    }
                })
                .flatten()
        };
        let posed = |el: gpui::Div, pose: Option<Pose>| match pose {
            Some(p) => el.relative().left(px(-8. * (1. - p.t))).opacity(p.opacity),
            None => el,
        };
        if folded {
            let n = people.iter().filter(|p| !p.2).count();
            return div()
                .flex()
                .h(px(28.))
                .items_center()
                .pr(px(8.))
                .pl(px(28.))
                .children(people.into_iter().take(STACK).map(|(p, pose, _)| {
                    let key = format!("stack-{room}-{}", p.user_id);
                    let uid = p.user_id.clone();
                    let k = key.clone();
                    posed(
                        div().flex().mr(px(4.)).child(
                            div()
                                .id(ElementId::Name(key.clone().into()))
                                .relative()
                                .flex()
                                .rounded_full()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_person(uid.clone(), k.clone(), false, cx)
                                }))
                                .child(self.probe(key))
                                .child(
                                    Avatar::new(p.user_id.clone(), p.name.clone(), accent(&p), 20)
                                        .speaking(speaking(&p)),
                                ),
                        ),
                        pose,
                    )
                }))
                .when(n > STACK, |d| {
                    d.child(
                        div()
                            .ml(px(8.))
                            .text_step(TEXT_XS)
                            .text_color(t.fg_2)
                            .child(format!("+{}", n - STACK)),
                    )
                })
                .into_any_element();
        }
        let rows: Vec<AnyElement> = people
            .into_iter()
            .map(|(p, pose, leaving)| {
                let pid = ElementId::Name(format!("person-{room}-{}", p.user_id).into());
                let (h, on_hover) = motion::hover(pid.clone(), window, cx);
                let h = if leaving { 0. } else { h };
                let state = if p.deafened {
                    Some(("headphonesOff", "Deafened"))
                } else if p.muted || p.force_muted {
                    Some(("micOff", "Muted"))
                } else {
                    None
                };
                let sp = speaking(&p);
                let key = format!("person-{room}-{}", p.user_id);
                let uid = p.user_id.clone();
                let k = key.clone();
                let lk = link(&p);
                let row = div()
                    .id(pid)
                    .debug_selector({
                        let k = key.clone();
                        move || k
                    })
                    .relative()
                    .cursor_pointer()
                    .when(!leaving, |d| {
                        d.on_click(cx.listener(move |this, _, _, cx| {
                            this.open_person(uid.clone(), k.clone(), false, cx)
                        }))
                    })
                    .child(self.probe(key))
                    .flex()
                    .h(px(28.))
                    .min_w_0()
                    .flex_1()
                    .items_center()
                    .gap(px(8.))
                    .rounded(t.radius_sm)
                    .pr(px(8.))
                    .pl(px(28.))
                    .bg(mix(transparent_black(), t.bg_hover.into(), h))
                    .text_color(mix(t.fg_2.into(), t.fg.into(), h))
                    .when(!leaving, |d| d.on_hover(on_hover))
                    .child(
                        Avatar::new(p.user_id.clone(), p.name.clone(), accent(&p), 20).speaking(sp),
                    )
                    .child(div().min_w_0().ellipsis().child(p.name.clone()))
                    .when(sp, |d| d.child(speaking_glyph(t.fg_2)))
                    .when_some(state, |d, (name, _)| {
                        d.child(icon(name).text_color(t.danger_fg))
                    })
                    .when(p.share.is_some(), |d| {
                        d.child(icon("monitor").text_color(t.fg_3))
                    })
                    // `ml-auto`: at the right end.
                    .when_some(lk, |d, l| {
                        d.child(
                            div()
                                .flex()
                                .flex_1()
                                .justify_end()
                                .child(super::person::link_icon(&l, &t)),
                        )
                    });
                posed(div().flex().min_w_0().child(row), pose).into_any_element()
            })
            .collect();
        div().flex().flex_col().children(rows).into_any_element()
    }
}

/// `Speaking.svelte`: three still bars, 12 px.
fn speaking_glyph(color: impl Into<Hsla>) -> impl IntoElement {
    let c = color.into();
    let bar = move |top: f32, h: f32| {
        div()
            .absolute()
            .top(px(top))
            .w(px(2.))
            .h(px(h))
            .rounded(px(1.))
            .bg(c)
    };
    div()
        .relative()
        .flex_none()
        .size(px(12.))
        .child(bar(4., 4.).left(px(1.)))
        .child(bar(1., 10.).left(px(5.)))
        .child(bar(3., 6.).left(px(9.)))
}
