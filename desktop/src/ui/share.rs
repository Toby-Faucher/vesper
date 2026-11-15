//! The share panel at the top of the call's room (`SharePanel.svelte`, `ShareTile.svelte`): the screens shared, a
//! tile each, the watched one drawn at fit size. A line says when a share ended; a chip on the tile when it struggles.
//!
//! Not here: the layouts (Grid, Focus, Dock), chat beside the screens, hiding a share, point, zoom, full screen and
//! pop out, the figures line. The desktop watches one share at a time (Watch on the others).

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder;
use gpui::{
    Context, Div, ElementId, ImageSource, InteractiveElement, ParentElement, RenderImage,
    StatefulInteractiveElement, Styled, Window, div, img, px,
};

use super::Vesper;
use super::c::{self, Avatar};
use super::icons::icon;
use super::motion;
use super::theme::{TEXT_XS, TextStepExt, tokens};
use crate::model::Phase;
use crate::voice::Frame;

/// A decoded picture as GPUI's image. The pixels move over without a copy when nothing else holds them.
pub fn picture(f: Frame) -> Option<Arc<RenderImage>> {
    let bgra = Arc::try_unwrap(f.bgra).unwrap_or_else(|a| (*a).clone());
    let buf = image::RgbaImage::from_raw(f.width, f.height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
}

impl Vesper {
    /// Before each draw: tells the model whether the share view is on screen, swaps in the newest picture (freeing
    /// the last one's GPU copy), and keeps a timer for the next `watch off`.
    pub(super) fn sync_share(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let m = &self.model;
        self.draws += 1;
        let visible = !self.settings_open
            && !self.minimised
            && m.call.phase == Phase::Joined
            && m.call.room == m.active;
        let cmds = self.model.set_share_visible(visible, Instant::now());
        self.run(cmds);

        let c = &mut self.model.call;
        let new = c
            .take_frame()
            .and_then(|f| Some((f.user_id.clone(), picture(f)?)));
        let stale = self
            .screen
            .as_ref()
            .is_some_and(|(id, _)| c.watching.as_ref() != Some(id));
        if (new.is_some() || stale)
            && let Some((_, old)) = std::mem::replace(&mut self.screen, new)
        {
            cx.drop_image(old, Some(window));
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

    /// The panel (`SharePanel.svelte`), while in the shown room's call and someone shares or a share just ended:
    /// a head line with the count, then a tile per share (`ShareTile.svelte`: 16:9, `--bg-sidebar`, the sharer's
    /// name in a chip at the bottom left). The watched one is big with its picture; the others are thumbnails with
    /// Watch. The desktop watches one share at a time, so where the web shows every screen it shows Watch.
    pub(super) fn render_share(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let t = tokens(cx).clone();
        let m = &self.model;
        let c = &m.call;
        if c.phase != Phase::Joined || c.room != m.active {
            return None;
        }
        let sharers = m.sharers();
        if sharers.is_empty() && c.share_notice.is_empty() {
            return None;
        }
        let watching = c
            .watching
            .as_ref()
            .is_some_and(|id| sharers.iter().any(|(s, _)| s == id));
        let head = div()
            .flex()
            .min_h(px(28.))
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap_x(px(8.))
            .text_step(TEXT_XS)
            .text_color(t.fg_3)
            .when(!sharers.is_empty(), |d| {
                let n = sharers.len();
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon("monitor").text_color(t.fg_3))
                        .child(if n == 1 {
                            "1 screen".to_string()
                        } else {
                            format!("{n} screens")
                        }),
                )
            })
            .when(!c.share_notice.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| "share-notice".into())
                        .text_color(t.fg_2)
                        .child(c.share_notice.clone()),
                )
                .child(
                    c::Button::new("share-notice-ok", "OK")
                        .small()
                        .variant(c::Variant::Ghost)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.model.call.share_notice.clear();
                            cx.notify();
                        })),
                )
            });
        let tiles: Vec<gpui::Stateful<Div>> = sharers
            .iter()
            .map(|(id, name)| self.share_tile(id, name, watching, &t, window, cx))
            .collect();
        Some(
            div()
                .debug_selector(|| "share-panel".into())
                .flex()
                .flex_col()
                // Shares the room view with the messages while a screen is shown.
                .when(watching, |d| d.flex_1().min_h_0())
                .when(!watching, |d| d.flex_none())
                .gap(px(8.))
                .px(px(16.))
                .pt(px(8.))
                .child(head)
                .when(!tiles.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .when(watching, |d| d.flex_1().min_h_0())
                            .min_w_0()
                            .justify_center()
                            .items_center()
                            .gap(px(12.))
                            .children(tiles),
                    )
                }),
        )
    }

    /// One share's tile. Hovered, its controls fade in (`--dur-2`, none under reduced motion).
    fn share_tile(
        &self,
        id: &str,
        name: &str,
        big: bool,
        t: &super::theme::Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let c = &self.model.call;
        let watched = c.watching.as_deref() == Some(id);
        let tile_id = ElementId::Name(format!("share-tile-{id}").into());
        let mo = &t.motion;
        let (h, on_hover) =
            motion::transition(tile_id.clone(), mo.or(mo.dur_2, Duration::ZERO), window, cx);
        let accent = self
            .model
            .state
            .users
            .get(id)
            .map_or(String::new(), |u| u.accent.clone());
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
        let user = id.to_string();
        let sel = format!("share-{}-{id}", if watched { "unwatch" } else { "watch" });
        let body = if watched {
            let shown = self.screen.as_ref().filter(|(s, _)| s == id);
            match shown {
                // The default object fit is Contain: the whole screen at its own aspect ratio.
                Some((_, image)) => div()
                    .debug_selector(|| "share-picture".into())
                    .absolute()
                    .inset_0()
                    .child(img(ImageSource::Render(image.clone())).size_full()),
                None => div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(t.fg_3)
                    .child(format!("Waiting for {name}'s screen...")),
            }
        } else {
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div().debug_selector(move || sel).child(
                        c::Button::new(
                            ElementId::Name(format!("share-watch-{id}").into()),
                            "Watch",
                        )
                        .primary()
                        .small()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let user = Some(user.clone());
                            this.act(|m| m.watch(user, Instant::now()), cx);
                        })),
                    ),
                )
        };
        let struggle = c.struggling.contains(id).then(|| {
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
        let unwatch_sel = format!("share-unwatch-{id}");
        let controls = watched.then(|| {
            chip()
                .absolute()
                .top(px(8.))
                .right(px(8.))
                .p(px(2.))
                .opacity(h)
                .child(
                    div().debug_selector(move || unwatch_sel).child(
                        c::IconButton::new(
                            ElementId::Name(format!("share-unwatch-{id}").into()),
                            "eyeOff",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.act(|m| m.watch(None, Instant::now()), cx);
                        })),
                    ),
                )
        });
        div()
            .id(tile_id)
            .when(watched, |d| d.debug_selector(|| "share-view".into()))
            .relative()
            .overflow_hidden()
            .border_1()
            .border_color(t.line_subtle)
            .bg(t.bg_sidebar)
            .rounded(t.radius_lg)
            .map(|d| {
                // `aspect-video` (GPUI has the style but no method for it).
                let mut d = if big && watched {
                    d.h_full().max_w_full()
                } else {
                    d.w(px(240.)).flex_none()
                };
                d.style().aspect_ratio = Some(16. / 9.);
                d
            })
            .on_hover(on_hover)
            .child(body)
            .children(struggle)
            .children(controls)
            .child(
                div().absolute().left(px(8.)).bottom(px(8.)).child(
                    chip()
                        .h(px(24.))
                        .pr(px(8.))
                        .pl(px(2.))
                        .whitespace_nowrap()
                        .child(Avatar::new(id.to_string(), name.to_string(), accent, 20))
                        .child(name.to_string()),
                ),
            )
    }
}
