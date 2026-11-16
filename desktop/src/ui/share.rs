//! The screens shared in my call, on the call room's page (`SharePanel.svelte`, `ShareTile.svelte`): a head line,
//! then a tile per share in one of three layouts (Grid, Focus, Dock) with chat beside them. Every share with a tile is
//! watched. A tile draws the whole picture inside its 16:9 box (letterboxed), and has Point, Zoom, Full screen and
//! Hide on hover.
//!
//! Not here: pop out (the web has none in Firefox either), the figures line, the divider's arrow keys.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::{
    Bounds, Context, Div, ElementId, ImageSource, InteractiveElement, MouseButton, ParentElement,
    Pixels, Point, RenderImage, Size, StatefulInteractiveElement, Styled, Task, Window, canvas,
    div, img, point, px, relative,
};

use super::Vesper;
use super::c::{self, Avatar};
use super::icons::icon;
use super::motion;
use super::theme::{TEXT_XS, TextStepExt, Tokens, tokens};
use crate::model::Phase;
use crate::net::Cmd;
use crate::settings::ShareLayout;
use crate::voice::{Frame, VoiceCmd};

/// The stage's gap between tiles, `gap-3`.
const GAP: f32 = 12.;
/// A thumbnail in Focus, `w-44`.
const THUMB: f32 = 176.;

/// A decoded picture as GPUI's image. The pixels move over without a copy when nothing else holds them.
pub fn picture(f: Frame) -> Option<Arc<RenderImage>> {
    let bgra = Arc::try_unwrap(f.bgra).unwrap_or_else(|a| (*a).clone());
    let buf = image::RgbaImage::from_raw(f.width, f.height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

/// The largest 16:9 tiles for `n` shares in a `w` x `h` stage, with at most `max_cols` columns: the column count and
/// the tile width (`fit` in `web/src/lib/share/geometry.ts`).
pub fn fit(n: usize, w: f32, h: f32, gap: f32, max_cols: usize) -> (usize, f32) {
    let mut best = (1, 0f32);
    for cols in 1..=n.min(max_cols).max(1) {
        let rows = n.div_ceil(cols) as f32;
        let c = cols as f32;
        let width = ((w - gap * (c - 1.)) / c).min((h - gap * (rows - 1.)) / rows * (16. / 9.));
        if width > best.1 {
            best = (cols, width);
        }
    }
    (best.0, best.1.floor().max(0.))
}

/// How `repeat(auto-fit, minmax(min(160px, 100%), {width}px))` lays out `n` tiles in a `w` wide grid: the columns
/// and their width. A track never goes below 160 px (or the whole width), and as many tracks fit as there is room.
pub fn grid_columns(n: usize, w: f32, width: f32, gap: f32) -> (usize, f32) {
    let track = width.max(160f32.min(w));
    let fits = ((w + gap) / (track + gap)).floor().max(1.) as usize;
    (fits.min(n.max(1)), track)
}

/// Where the picture sits in a `w` x `h` box with object-fit: contain (`pictureBox`).
pub fn picture_box(w: f32, h: f32, pw: f32, ph: f32) -> Bounds<f32> {
    let scale = if pw > 0. && ph > 0. {
        (w / pw).min(h / ph)
    } else {
        0.
    };
    let (iw, ih) = if scale > 0. {
        (pw * scale, ph * scale)
    } else {
        (w, h)
    };
    Bounds {
        origin: point((w - iw) / 2., (h - ih) / 2.),
        size: Size {
            width: iw,
            height: ih,
        },
    }
}

/// A spot in a box as 0 to 1 of it, or none outside (`pointIn`, with the picture's box as drawn).
pub fn point_in(at: Point<f32>, b: Bounds<f32>) -> Option<(f32, f32)> {
    if b.size.width <= 0. || b.size.height <= 0. {
        return None;
    }
    let x = (at.x - b.origin.x) / b.size.width;
    let y = (at.y - b.origin.y) / b.size.height;
    ((0. ..=1.).contains(&x) && (0. ..=1.).contains(&y)).then_some((x, y))
}

/// The tile's frame zoomed 2x around `(zx, zy)` of it, like `transform: scale(2)` with that transform-origin: where a
/// box inside it ends up.
fn zoomed(b: Bounds<f32>, w: f32, h: f32, z: Option<(f32, f32)>) -> Bounds<f32> {
    let Some((zx, zy)) = z else { return b };
    let (ox, oy) = (zx * w, zy * h);
    Bounds {
        origin: point(ox + 2. * (b.origin.x - ox), oy + 2. * (b.origin.y - oy)),
        size: Size {
            width: b.size.width * 2.,
            height: b.size.height * 2.,
        },
    }
}

fn f(p: Pixels) -> f32 {
    f32::from(p)
}

/// A zoomed tile dragged: where it started and the zoom then.
struct Drag {
    id: String,
    at: Point<Pixels>,
    z: (f32, f32),
    moved: bool,
}

/// What the share panel keeps between frames.
pub struct ShareView {
    /// The newest picture of each watched share.
    pub screens: HashMap<String, Arc<RenderImage>>,
    /// The share shown full screen, and whether the window was full screen already.
    pub full: Option<(String, bool)>,
    /// Zoomed tiles: the spot zoomed around, 0 to 1 of the tile.
    pub zoom: HashMap<String, (f32, f32)>,
    /// Tiles where the next click points.
    pointing: HashSet<String>,
    drag: Option<Drag>,
    /// The panel's part of the width in Dock, in per cent (in memory only, as on the web).
    dock: f32,
    divider: bool,
    /// The stage's size last frame, the row's bounds and each tile's, for the layout and the mouse.
    stage: Rc<Cell<Size<Pixels>>>,
    row: Rc<Cell<Bounds<Pixels>>>,
    tiles: super::person::Anchors,
    /// The size each share's pictures were asked for, in the call it was asked in.
    sent: (u64, HashMap<String, (u32, u32)>),
    /// Redraws when the next point marker goes.
    marker_timer: Option<(Instant, Task<()>)>,
}

impl Default for ShareView {
    fn default() -> Self {
        ShareView {
            screens: HashMap::new(),
            full: None,
            zoom: HashMap::new(),
            pointing: HashSet::new(),
            drag: None,
            dock: 60.,
            divider: false,
            stage: Rc::default(),
            row: Rc::default(),
            tiles: Rc::default(),
            sent: (0, HashMap::new()),
            marker_timer: None,
        }
    }
}

impl Vesper {
    /// Before each draw: tells the model whether the share view is on screen, swaps in the newest pictures (freeing
    /// the last ones' GPU copies), lets go of shares no longer shown, and keeps timers for the next `watch off` and
    /// the next point marker to go.
    pub(super) fn sync_share(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let m = &self.model;
        self.draws += 1;
        let visible = !self.settings_open
            && !self.minimised
            && m.call.phase == Phase::Joined
            && m.call.room == m.active;
        let cmds = self.model.set_share_visible(visible, Instant::now());
        self.run(cmds);

        for (id, f) in self.model.call.take_frames() {
            if let Some(new) = picture(f)
                && let Some(old) = self.share.screens.insert(id, new)
            {
                cx.drop_image(old, Some(window));
            }
        }
        let shown: HashSet<String> = self
            .model
            .sharers()
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| self.model.call.shown(id))
            .collect();
        let gone: Vec<String> = (self.share.screens.keys())
            .filter(|id| !shown.contains(*id))
            .cloned()
            .collect();
        for id in gone {
            if let Some(old) = self.share.screens.remove(&id) {
                cx.drop_image(old, Some(window));
            }
        }
        let s = &mut self.share;
        s.zoom.retain(|id, _| shown.contains(id));
        s.pointing.retain(|id| shown.contains(id));
        if s.full.as_ref().is_some_and(|(id, _)| !shown.contains(id)) {
            self.leave_full(window, cx);
        }

        let now = Instant::now();
        self.model.call.markers.retain(|m| m.until > now);
        let next = self.model.call.markers.iter().map(|m| m.until).min();
        if next != self.share.marker_timer.as_ref().map(|t| t.0) {
            self.share.marker_timer = next.map(|at| {
                let wait = at.saturating_duration_since(now);
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |v, cx| {
                        v.share.marker_timer = None;
                        cx.notify();
                    });
                });
                (at, task)
            });
        }

        self.sync_watch_timer(cx);
    }

    /// Keeps a timer for the next `watch off`. Also called with no draw, for a window that is not drawn.
    pub(super) fn sync_watch_timer(&mut self, cx: &mut Context<Self>) {
        let due = self.model.call.watch_deadline();
        if due != self.watch_timer.as_ref().map(|t| t.0) {
            self.watch_timer = due.map(|at| {
                let wait = at.saturating_duration_since(Instant::now());
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |v, cx| {
                        v.watch_timer = None;
                        v.act(|m| m.sync_watches(Instant::now()), cx);
                        v.sync_watch_timer(cx);
                    });
                });
                (at, task)
            });
        }
    }

    /// Full screen off (Esc, the button, or the share went): the window goes back to how it was.
    pub(super) fn leave_full(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, was)) = self.share.full.take() else {
            return;
        };
        if !was && window.is_fullscreen() {
            window.toggle_fullscreen();
        }
        self.composer.update(cx, |i, cx| i.focus(window, cx));
    }

    fn toggle_full(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.share.full.is_some() {
            self.leave_full(window, cx);
        } else {
            let was = window.is_fullscreen();
            self.share.full = Some((id.to_string(), was));
            // The keys (Esc) go to the full screen view, the only thing drawn.
            self.key_focus.focus(window);
            if !was {
                window.toggle_fullscreen();
            }
        }
        cx.notify();
    }

    /// Tells the decoder the size each share is drawn at, in device pixels (twice that zoomed), when it changed.
    fn share_sizes(&mut self, sizes: Vec<(String, f32, f32)>, window: &Window) {
        let k = window.scale_factor();
        let call = self.model.call.id;
        let (at, sent) = &mut self.share.sent;
        if *at != call {
            (*at, *sent) = (call, HashMap::new()); // a new call's decoder knows no sizes
        }
        let mut cmds = vec![];
        for (user_id, w, h) in sizes {
            let z = if self.share.zoom.contains_key(&user_id) {
                2.
            } else {
                1.
            };
            let (width, height) = ((w * k * z).round() as u32, (h * k * z).round() as u32);
            if width == 0 || height == 0 || sent.get(&user_id) == Some(&(width, height)) {
                continue;
            }
            sent.insert(user_id.clone(), (width, height));
            cmds.push(Cmd::Voice(VoiceCmd::ShareSize {
                user_id,
                width,
                height,
            }));
        }
        self.run(cmds);
    }

    /// The share shown full screen, filling the window, when there is one.
    pub(super) fn render_full_share(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let (id, _) = self.share.full.clone()?;
        let name = self.model.state.name_of(&id);
        let t = tokens(cx).clone();
        let vp = window.viewport_size();
        let tile = self.share_tile(
            &id,
            &name,
            f(vp.width),
            f(vp.height),
            Tile::Full,
            &t,
            window,
            cx,
        );
        self.share_sizes(vec![(id, f(vp.width), f(vp.height))], window);
        Some(
            div()
                .track_focus(&self.key_focus)
                .size_full()
                .bg(t.bg_app)
                .child(tile),
        )
    }

    /// The row under the room's header: the share panel (or its head line) and `chat`, beside it while the panel is
    /// open (`RoomView.svelte` with `SharePanel.svelte`'s styles), else under it.
    pub(super) fn share_row(
        &mut self,
        chat: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let row = div().relative().flex().flex_1().min_h_0();
        let m = &self.model;
        let c = &m.call;
        let ids: Vec<(String, String)> = if c.phase == Phase::Joined && c.room == m.active {
            m.sharers()
        } else {
            vec![]
        };
        if ids.is_empty() {
            return row.flex_col().child(chat);
        }
        let t = tokens(cx).clone();
        let shown: Vec<&(String, String)> = ids
            .iter()
            .filter(|(id, _)| !c.hidden.contains(id))
            .collect();
        let hidden: Vec<&(String, String)> =
            ids.iter().filter(|(id, _)| c.hidden.contains(id)).collect();
        let open = c.panel_open && !shown.is_empty();
        let layout = m.settings.share_layout;
        let chat_open = c.chat_open;

        let head = self.share_head(&ids, &shown, &hidden, open, layout, &t, cx);
        if !open {
            return row
                .flex_col()
                .child(div().flex_none().px(px(16.)).pt(px(8.)).child(head))
                .child(chat);
        }

        // The stage and its tiles, sized from the stage's size last frame (`fit`).
        let big = c
            .focus
            .as_ref()
            .filter(|f| shown.iter().any(|(id, _)| id == *f))
            .cloned()
            .unwrap_or_else(|| shown[0].0.clone());
        let tiles: Vec<&(String, String)> = if layout == ShareLayout::Focus {
            shown.iter().copied().filter(|(id, _)| *id == big).collect()
        } else {
            shown.clone()
        };
        let stage = self.share.stage.get();
        let (sw, sh) = (f(stage.width), f(stage.height));
        let n = tiles.len();
        let max_cols = if layout == ShareLayout::Dock { 1 } else { n };
        let (_, width) = fit(n, sw, sh, GAP, max_cols);
        let (cols, width) = if layout == ShareLayout::Grid {
            grid_columns(n, sw, width, GAP)
        } else {
            (1, width.min(sw))
        };
        let height = width * 9. / 16.;
        let mut rows = vec![];
        for chunk in tiles.chunks(cols) {
            let r = div()
                .flex()
                .flex_none()
                .gap(px(GAP))
                .w(px(width * cols as f32 + GAP * (cols as f32 - 1.)))
                .children(chunk.iter().map(|(id, name)| {
                    self.share_tile(id, name, width, height, Tile::Big, &t, window, cx)
                }));
            rows.push(r);
        }
        let probe = self.share.stage.clone();
        let stage_el = div()
            .debug_selector(|| "share-stage".into())
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .justify_center()
            .items_center()
            .gap(px(GAP))
            .child(
                canvas(
                    move |b, window, _| {
                        // A new size lays the tiles out again next frame.
                        if probe.replace(b.size) != b.size {
                            window.refresh();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .children(rows);
        let thumbs = (layout == ShareLayout::Focus && shown.len() > 1).then(|| {
            div()
                .flex()
                .flex_none()
                .justify_center()
                .gap(px(GAP))
                .children(shown.iter().filter(|(id, _)| *id != big).map(|(id, name)| {
                    self.share_tile(
                        id,
                        name,
                        THUMB,
                        THUMB * 9. / 16.,
                        Tile::Thumb,
                        &t,
                        window,
                        cx,
                    )
                }))
        });
        let dock = layout == ShareLayout::Dock;
        let mut sizes: Vec<(String, f32, f32)> = tiles
            .iter()
            .map(|(id, _)| (id.clone(), width, height))
            .collect();
        if layout == ShareLayout::Focus {
            sizes.extend(
                shown
                    .iter()
                    .filter(|(id, _)| *id != big)
                    .map(|(id, _)| (id.clone(), THUMB, THUMB * 9. / 16.)),
            );
        }
        let panel = div()
            .debug_selector(|| "share-panel".into())
            .flex()
            .flex_col()
            .min_h_0()
            .min_w_0()
            .gap(px(8.))
            .px(px(16.))
            .pt(px(8.))
            .map(|d| {
                if dock {
                    // `flex: 0 1 <dock>%`.
                    let mut d = d.flex_shrink().flex_basis(relative(self.share.dock / 100.));
                    d.style().flex_grow = Some(0.);
                    d
                } else {
                    d.flex_1()
                }
            })
            .child(head)
            .child(stage_el)
            .children(thumbs);

        // Chat: a 400 px drawer in Grid and Focus (laid out but hidden while closed), a column after the divider in Dock.
        let chat = chat.min_w(px(300.)).map(|d| {
            if dock {
                d
            } else if chat_open {
                d.flex_none()
                    .w(px(400.))
                    .border_l_1()
                    .border_color(t.line_subtle)
            } else {
                d.flex_none()
                    .w(px(400.))
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .invisible()
            }
        });
        let divider = dock.then(|| {
            let on = self.share.divider;
            div()
                .id("share-divider")
                .debug_selector(|| "share-divider".into())
                .flex_none()
                .w(px(9.))
                .h_full()
                .flex()
                .justify_center()
                .cursor_col_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.share.divider = true;
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .child(div().h_full().w(px(if on { 3. } else { 1. })).bg(if on {
                    t.line_strong
                } else {
                    t.line_subtle
                }))
        });
        self.share_sizes(sizes, window);
        let probe = self.share.row.clone();
        row.flex_row()
            .child(
                canvas(move |b, _, _| probe.set(b), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(panel)
            .children(divider)
            .child(chat)
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                if this.share.divider && e.pressed_button == Some(MouseButton::Left) {
                    let r = this.share.row.get();
                    let x = (f(e.position.x) - f(r.origin.x)) / f(r.size.width).max(1.) * 100.;
                    this.share.dock = x.clamp(20., 80.);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.share.divider {
                        this.share.divider = false;
                        cx.notify();
                    }
                }),
            )
    }

    /// The head line: the count, each hidden share with Show, the layouts, Chat and Hide screens; Show alone while
    /// the panel is closed.
    #[allow(clippy::too_many_arguments)]
    fn share_head(
        &self,
        ids: &[(String, String)],
        shown: &[&(String, String)],
        hidden: &[&(String, String)],
        open: bool,
        layout: ShareLayout,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> Div {
        let c = &self.model.call;
        let count = |n: usize| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(icon("monitor").text_color(t.fg_3))
                .child(if n == 1 {
                    "1 screen".to_string()
                } else {
                    format!("{n} screens")
                })
        };
        let ghost = |id: String, label: &'static str| {
            c::Button::new(ElementId::Name(id.into()), label)
                .small()
                .variant(c::Variant::Ghost)
        };
        let head = div()
            .debug_selector(|| "share-head".into())
            .flex()
            .min_h(px(28.))
            .flex_none()
            .flex_wrap()
            .items_center()
            // `gap-x-2` as margins: GPUI's gap and an auto margin together leave the gaps at the end.
            .text_step(TEXT_XS)
            .text_color(t.fg_3);
        let head = if !c.panel_open {
            head.child(count(ids.len())).child(
                div()
                    .ml(px(8.))
                    .debug_selector(|| "share-show-all".into())
                    .child(ghost("share-show-all".into(), "Show").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.act(|m| m.set_panel_open(true, Instant::now()), cx);
                        },
                    ))),
            )
        } else {
            head.when(open, |d| d.child(count(shown.len())))
                .children(hidden.iter().enumerate().map(|(i, (id, name))| {
                    let user = id.clone();
                    div()
                        .when(open || i > 0, |d| d.ml(px(8.)))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_color(t.fg_2)
                        .child(icon("eyeOff"))
                        .child(format!("{name}'s screen is hidden."))
                        .child(
                            div()
                                .debug_selector({
                                    let s = format!("share-show-{id}");
                                    move || s
                                })
                                .child(ghost(format!("share-show-{id}"), "Show").on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        let user = user.clone();
                                        this.act(
                                            |m| m.hide_share(&user, false, Instant::now()),
                                            cx,
                                        );
                                    }),
                                )),
                        )
                }))
                .when(open, |d| {
                    let layouts = [
                        (ShareLayout::Grid, "layoutGrid", "grid"),
                        (ShareLayout::Focus, "galleryVerticalEnd", "focus"),
                        (ShareLayout::Dock, "columns2", "dock"),
                    ];
                    d.child(
                        div()
                            .debug_selector(|| "share-layouts".into())
                            .ml_auto()
                            .flex()
                            .items_center()
                            .children(layouts.map(|(l, i, name)| {
                                div()
                                    .debug_selector(move || format!("share-layout-{name}"))
                                    .child(
                                        c::IconButton::new(
                                            ElementId::Name(format!("share-layout-{name}").into()),
                                            i,
                                        )
                                        .pressed(layout == l)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.change_settings(|s| s.share_layout = l, cx);
                                                cx.notify();
                                            }),
                                        ),
                                    )
                            })),
                    )
                    .when(layout != ShareLayout::Dock, |d| {
                        d.child(
                            div()
                                .ml(px(8.))
                                .debug_selector(|| "share-chat".into())
                                .child(ghost("share-chat".into(), "Chat").icon("room").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.model.call.chat_open = !this.model.call.chat_open;
                                        cx.notify();
                                    }),
                                )),
                        )
                    })
                    .child(
                        div()
                            .ml(px(8.))
                            .debug_selector(|| "share-hide-all".into())
                            .child(ghost("share-hide-all".into(), "Hide screens").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.act(|m| m.set_panel_open(false, Instant::now()), cx);
                                }),
                            )),
                    )
                })
        };
        // A share the server stopped (`SHARE_STOPPED`), until OK.
        head.when(!c.share_notice.is_empty(), |d| {
            d.child(
                div()
                    .ml(px(8.))
                    .debug_selector(|| "share-notice".into())
                    .text_color(t.fg_2)
                    .child(c.share_notice.clone()),
            )
            .child(div().ml(px(8.)).child(
                ghost("share-notice-ok".into(), "OK").on_click(cx.listener(|this, _, _, cx| {
                    this.model.call.share_notice.clear();
                    cx.notify();
                })),
            ))
        })
    }

    /// One share's tile, `w` x `h`. Hovered, its controls fade in (`--dur-2`, none under reduced motion).
    #[allow(clippy::too_many_arguments)]
    fn share_tile(
        &self,
        id: &str,
        name: &str,
        w: f32,
        h: f32,
        kind: Tile,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let c = &self.model.call;
        let s = &self.share;
        let thumb = kind == Tile::Thumb;
        let tile_id = ElementId::Name(format!("share-tile-{id}-{kind:?}").into());
        let mo = &t.motion;
        let (hov, on_hover) =
            motion::transition(tile_id.clone(), mo.or(mo.dur_2, Duration::ZERO), window, cx);
        let accent_of = |u: &str| {
            self.model
                .state
                .users
                .get(u)
                .map_or(String::new(), |u| u.accent.clone())
        };
        let chip = || {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(t.radius)
                .bg(t.bg_app)
                .text_step(TEXT_XS)
                .text_color(t.fg)
        };
        let zoom = s.zoom.get(id).copied().filter(|_| !thumb);
        let pointing = s.pointing.contains(id) && !thumb;
        // The picture's box inside the tile, as drawn (zoomed or not).
        let image = s.screens.get(id).cloned();
        let pic = image.as_ref().map(|i| {
            let size = i.size(0);
            let b = picture_box(w, h, size.width.0 as f32, size.height.0 as f32);
            zoomed(b, w, h, zoom)
        });
        let picture = image.zip(pic).map(|(i, b)| {
            div()
                .debug_selector(|| "share-picture".into())
                .absolute()
                .left(px(b.origin.x))
                .top(px(b.origin.y))
                .w(px(b.size.width))
                .h(px(b.size.height))
                .child(img(ImageSource::Render(i)).size_full())
        });
        let markers = pic.filter(|_| !thumb).map(|b| {
            let mut out = vec![];
            for m in c.markers.iter().filter(|m| m.share == id) {
                let who = self.model.state.name_of(&m.from);
                let slot = c::avatar_slot(&m.from, &accent_of(&m.from));
                out.push(
                    div()
                        .absolute()
                        .left(px(b.origin.x + m.x * b.size.width))
                        .top(px(b.origin.y + m.y * b.size.height))
                        .child(
                            div()
                                .absolute()
                                .left(px(-8.))
                                .top(px(-8.))
                                .size(px(16.))
                                .rounded_full()
                                .border_2()
                                .border_color(t.fg)
                                .bg(t.avatar[slot - 1]),
                        )
                        .child(
                            chip()
                                .absolute()
                                .left(px(12.))
                                .top(px(8.))
                                .px(px(8.))
                                .py(px(2.))
                                .whitespace_nowrap()
                                .child(who),
                        ),
                );
            }
            out
        });

        // The frame takes the clicks: point, move the zoom, drag it; a thumbnail's makes it the big one.
        let user = id.to_string();
        let tiles = s.tiles.clone();
        let key = format!("{id}-{kind:?}");
        let frame = div()
            .id(ElementId::Name(format!("share-frame-{id}-{kind:?}").into()))
            .absolute()
            .inset_0()
            .when(pointing, |d| d.cursor_crosshair())
            .when(zoom.is_some() && !pointing, |d| d.cursor_move())
            .when(thumb, |d| d.cursor_pointer())
            .child({
                let (tiles, key) = (tiles.clone(), key.clone());
                canvas(
                    move |b, _, _| {
                        tiles.borrow_mut().insert(key, b);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0()
            })
            .on_mouse_down(MouseButton::Left, {
                let user = user.clone();
                cx.listener(move |this, e: &gpui::MouseDownEvent, _, _| {
                    let s = &mut this.share;
                    if let Some(&z) = s.zoom.get(&user).filter(|_| !s.pointing.contains(&user)) {
                        s.drag = Some(Drag {
                            id: user.clone(),
                            at: e.position,
                            z,
                            moved: false,
                        });
                    } else {
                        s.drag = None;
                    }
                })
            })
            .on_mouse_move({
                let (user, tiles, key) = (user.clone(), tiles.clone(), key.clone());
                cx.listener(move |this, e: &gpui::MouseMoveEvent, _, cx| {
                    let Some(d) = this.share.drag.as_mut().filter(|d| d.id == user) else {
                        return;
                    };
                    if e.pressed_button != Some(MouseButton::Left) {
                        this.share.drag = None;
                        return;
                    }
                    let Some(b) = tiles.borrow().get(&key).copied() else {
                        return;
                    };
                    let (dx, dy) = (f(e.position.x - d.at.x), f(e.position.y - d.at.y));
                    d.moved |= dx.abs() + dy.abs() > 4.;
                    let z = (
                        (d.z.0 - dx / f(b.size.width).max(1.)).clamp(0., 1.),
                        (d.z.1 - dy / f(b.size.height).max(1.)).clamp(0., 1.),
                    );
                    this.share.zoom.insert(user.clone(), z);
                    cx.notify();
                })
            })
            .on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                if thumb {
                    this.model.call.focus = Some(user.clone());
                    return cx.notify();
                }
                let Some(b) = tiles.borrow().get(&key).copied() else {
                    return;
                };
                let at = e.position();
                let local = point(f(at.x - b.origin.x), f(at.y - b.origin.y));
                let (w, h) = (f(b.size.width), f(b.size.height));
                let s = &mut this.share;
                let moved = s.drag.take().is_some_and(|d| d.moved);
                if s.pointing.contains(&user) {
                    let Some(i) = s.screens.get(&user) else {
                        return;
                    };
                    let size = i.size(0);
                    let pic = picture_box(w, h, size.width.0 as f32, size.height.0 as f32);
                    let Some((x, y)) =
                        point_in(local, zoomed(pic, w, h, s.zoom.get(&user).copied()))
                    else {
                        return; // outside the picture
                    };
                    s.pointing.remove(&user);
                    let cmds = this.model.point_at(&user, x, y);
                    this.run(cmds);
                } else if let Some(&(zx, zy)) = s.zoom.get(&user).filter(|_| !moved) {
                    // The spot clicked of the frame as drawn, which a 2x zoom puts at -zx*w..2w-zx*w.
                    let z = ((local.x + zx * w) / (2. * w), (local.y + zy * h) / (2. * h));
                    s.zoom
                        .insert(user.clone(), (z.0.clamp(0., 1.), z.1.clamp(0., 1.)));
                }
                cx.notify();
            }))
            .children(picture)
            .children(markers.into_iter().flatten());

        let struggle = (!thumb && c.struggling.contains(id)).then(|| {
            chip()
                .debug_selector(|| "share-struggling".into())
                .absolute()
                .top(px(8.))
                .left(px(8.))
                .px(px(8.))
                .py(px(4.))
                .child(icon("warn").text_color(t.warn_fg))
                .child(format!("{name}'s upload is struggling."))
        });
        let full = kind == Tile::Full;
        let toolbar = (!thumb).then(|| {
            let button = |what: &str, icon: &'static str, pressed: bool, f: Box<ToolFn>| {
                let sel = format!("share-{what}-{id}");
                div().debug_selector(move || sel).child(
                    c::IconButton::new(ElementId::Name(format!("share-{what}-{id}").into()), icon)
                        .pressed(pressed)
                        .on_click(cx.listener(move |this, _, window, cx| f(this, window, cx))),
                )
            };
            let (u1, u2, u3, u4) = (
                id.to_string(),
                id.to_string(),
                id.to_string(),
                id.to_string(),
            );
            chip()
                .id(ElementId::Name(format!("share-controls-{id}").into()))
                .absolute()
                .top(px(8.))
                .right(px(8.))
                .gap(px(2.))
                .p(px(2.))
                .opacity(hov)
                .occlude()
                .child(button(
                    "point",
                    "mousePointerClick",
                    pointing,
                    Box::new(move |this, _, cx| {
                        let p = &mut this.share.pointing;
                        if !p.remove(&u1) {
                            p.insert(u1.clone());
                        }
                        cx.notify();
                    }),
                ))
                .child(button(
                    "zoom",
                    "zoomIn",
                    zoom.is_some(),
                    Box::new(move |this, _, cx| {
                        let z = &mut this.share.zoom;
                        if z.remove(&u2).is_none() {
                            z.insert(u2.clone(), (0.5, 0.5));
                        }
                        cx.notify();
                    }),
                ))
                .child(button(
                    "full",
                    "maximize",
                    full,
                    Box::new(move |this, window, cx| {
                        this.toggle_full(&u3, window, cx);
                    }),
                ))
                .child(button(
                    "hide",
                    "eyeOff",
                    false,
                    Box::new(move |this, window, cx| {
                        this.leave_full(window, cx);
                        this.act(|m| m.hide_share(&u4, true, Instant::now()), cx);
                    }),
                ))
        });
        let watched = !thumb;
        div()
            .id(tile_id)
            .when(watched, |d| d.debug_selector(|| "share-view".into()))
            .debug_selector({
                let s = format!("share-tile-{id}");
                move || s
            })
            .relative()
            .flex_none()
            .w(px(w))
            .h(px(h))
            .overflow_hidden()
            .border_1()
            .border_color(t.line_subtle)
            .bg(t.bg_sidebar)
            .map(|d| match kind {
                Tile::Big => d.rounded(t.radius_lg),
                Tile::Thumb => d.rounded(t.radius),
                Tile::Full => d,
            })
            .on_hover(on_hover)
            .child(frame)
            .children(struggle)
            .children(toolbar)
            .child(
                div().absolute().left(px(8.)).bottom(px(8.)).child(
                    chip()
                        .h(px(24.))
                        .pr(px(8.))
                        .pl(px(2.))
                        .whitespace_nowrap()
                        .child(Avatar::new(
                            id.to_string(),
                            name.to_string(),
                            accent_of(id),
                            20,
                        ))
                        .child(name.to_string()),
                ),
            )
    }
}

/// What a tile's toolbar button does.
type ToolFn = dyn Fn(&mut Vesper, &mut Window, &mut Context<Vesper>);

/// Where a tile is drawn: in the stage, as a thumbnail in Focus, or full screen.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tile {
    Big,
    Thumb,
    Full,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_matches_the_web() {
        // geometry.test.ts: one share fills the width when the stage is wide enough, else the height decides.
        assert_eq!(fit(1, 1600., 900., 12., 1), (1, 1600.));
        assert_eq!(fit(1, 1600., 450., 12., 1), (1, 800.));
        // Two in a wide stage go side by side; in a tall one, one above the other.
        assert_eq!(fit(2, 1600., 450., 12., 2).0, 2);
        assert_eq!(fit(2, 600., 900., 12., 2).0, 1);
        // Dock: one column whatever the width.
        assert_eq!(fit(3, 3000., 900., 12., 1).0, 1);
        assert_eq!(fit(0, 100., 100., 12., 0), (1, 100.));
        assert_eq!(fit(1, 0., 0., 12., 1), (1, 0.));
    }

    #[test]
    fn grid_columns_follow_auto_fit() {
        // As wide as fit says, and as many as fit, never more than the tiles.
        assert_eq!(grid_columns(3, 1000., 494., 12.), (2, 494.));
        assert_eq!(grid_columns(1, 1000., 900., 12.), (1, 900.));
        // A track is at least 160 px (or the whole stage).
        assert_eq!(grid_columns(4, 400., 100., 12.), (2, 160.));
        assert_eq!(grid_columns(2, 100., 50., 12.), (1, 100.));
    }

    #[test]
    fn a_1440p_picture_fits_a_smaller_tile_whole() {
        // 2560x1440 in a 1280x720 tile: scaled down, nothing cut. 4:3 in 16:9: letterboxed at the sides.
        let b = picture_box(1280., 720., 2560., 1440.);
        assert_eq!(
            (b.origin.x, b.origin.y, b.size.width, b.size.height),
            (0., 0., 1280., 720.)
        );
        let b = picture_box(1600., 900., 1024., 768.);
        assert_eq!(
            (b.origin.x, b.origin.y, b.size.width, b.size.height),
            (200., 0., 1200., 900.)
        );
        // A 21:9 picture: bars above and below.
        let b = picture_box(1600., 900., 2560., 1080.);
        assert!(b.size.width == 1600. && b.size.height < 900. && b.origin.y > 0.);
        assert_eq!(picture_box(100., 50., 0., 0.).size.width, 100.);
    }

    #[test]
    fn points_and_zoom() {
        let b = picture_box(1600., 900., 1024., 768.);
        assert_eq!(point_in(point(200., 0.), b), Some((0., 0.)));
        assert_eq!(point_in(point(800., 450.), b), Some((0.5, 0.5)));
        assert_eq!(
            point_in(point(100., 450.), b),
            None,
            "in the bar beside the picture"
        );
        // Zoomed 2x around the middle: the middle stays, the box doubles.
        let z = zoomed(b, 1600., 900., Some((0.5, 0.5)));
        assert_eq!((z.size.width, z.size.height), (2400., 1800.));
        assert_eq!(point_in(point(800., 450.), z), Some((0.5, 0.5)));
        // Around the top left corner: that corner stays.
        let z = zoomed(picture_box(160., 90., 16., 9.), 160., 90., Some((0., 0.)));
        assert_eq!((z.origin.x, z.origin.y), (0., 0.));
    }
}
