//! The share panel at the top of the call's room: who is sharing a screen, with Watch, and the watched screen drawn
//! at fit size. Plain text says when a share ended or struggles.

use std::sync::Arc;
use std::time::Instant;

use gpui::prelude::FluentBuilder;
use gpui::{
    Context, Div, ElementId, ImageSource, InteractiveElement, ParentElement, RenderImage, Styled,
    Window, div, img, px, rgb,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{h_flex, v_flex};

use super::sign_in::TextLine;
use super::{Vesper, theme};
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
        let visible =
            !self.settings_open && m.call.phase == Phase::Joined && m.call.room == m.active;
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

        let due = self.model.call.watch_deadline();
        if due != self.watch_timer.as_ref().map(|t| t.0) {
            self.watch_timer = due.map(|at| {
                let wait = at.saturating_duration_since(Instant::now());
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |v, cx| {
                        v.watch_timer = None;
                        v.act(|m| m.sync_watches(Instant::now()), cx);
                    });
                });
                (at, task)
            });
        }
    }

    /// The panel, while in the shown room's call and someone shares or a share just ended.
    pub(super) fn render_share(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let m = &self.model;
        let c = &m.call;
        if c.phase != Phase::Joined || c.room != m.active {
            return None;
        }
        let sharers = m.sharers();
        if sharers.is_empty() && c.share_notice.is_empty() {
            return None;
        }
        let rows: Vec<Div> = sharers
            .iter()
            .map(|(id, name)| {
                let watched = c.watching.as_ref() == Some(id);
                let user = id.clone();
                let button = if watched {
                    Button::new(ElementId::Name(format!("share-unwatch-{id}").into()))
                        .label("Stop watching")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.act(|m| m.watch(None, Instant::now()), cx);
                        }))
                } else {
                    Button::new(ElementId::Name(format!("share-watch-{id}").into()))
                        .primary()
                        .label("Watch")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let user = Some(user.clone());
                            this.act(|m| m.watch(user, Instant::now()), cx);
                        }))
                };
                let selector = format!("share-{}-{id}", if watched { "unwatch" } else { "watch" });
                let struggle = if c.struggling.contains(id) {
                    format!("{name}'s upload is struggling.")
                } else {
                    String::new()
                };
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .child(format!("{name} is sharing their screen.")),
                    )
                    .when_some_text(&struggle, theme::WARN_FG, "share-struggling")
                    .child(div().debug_selector(move || selector).child(button))
            })
            .collect();
        let notice =
            (!c.share_notice.is_empty()).then(|| {
                h_flex()
                    .gap_3()
                    .child(div().flex_1().when_some_text(
                        &c.share_notice,
                        theme::FG_2,
                        "share-notice",
                    ))
                    .child(Button::new("share-notice-ok").ghost().label("OK").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.model.call.share_notice.clear();
                            cx.notify();
                        }),
                    ))
            });
        let watched = c
            .watching
            .as_ref()
            .and_then(|id| sharers.iter().find(|(s, _)| s == id));
        let view = watched.map(|(id, name)| {
            let shown = self.screen.as_ref().filter(|(s, _)| s == id);
            div()
                .debug_selector(|| "share-view".into())
                .flex_1()
                .min_h(px(120.))
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(theme::BG_SIDEBAR))
                .rounded(px(6.))
                .overflow_hidden()
                .child(match shown {
                    // The default object fit is Contain: the whole screen at its own aspect ratio.
                    Some((_, image)) => div()
                        .debug_selector(|| "share-picture".into())
                        .size_full()
                        .child(img(ImageSource::Render(image.clone())).size_full()),
                    None => div()
                        .text_color(rgb(theme::FG_3))
                        .child(format!("Waiting for {name}'s screen...")),
                })
        });
        let watching = view.is_some();
        Some(
            v_flex()
                .debug_selector(|| "share-panel".into())
                // Shares the room view with the messages while a screen is shown.
                .when(watching, |d| d.flex_1().min_h_0())
                .when(!watching, |d| d.flex_none())
                .gap_2()
                .px_4()
                .py_2()
                .border_b_1()
                .border_color(rgb(theme::LINE_SUBTLE))
                .children(rows)
                .children(notice)
                .children(view),
        )
    }
}
