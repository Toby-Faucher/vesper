//! `ImageViewer.svelte`: a picture of the room at fit size in a `wide` dialog, its name as the title, Previous and
//! Next with "2 of 5" when the loaded part of the room has more pictures, Open original and Download. Left and Right
//! move, Esc closes. It fades in (`pop`; GPUI cannot scale a box).
//!
//! The picture is fetched again for the viewer, decoded up to `files::FULL` (the timeline's copy is small), and kept
//! while the viewer is open. shortcut: pictures only; a video still opens in the system's player (the web plays it
//! here). Port a player if GPUI gets one.

use std::collections::HashMap;
use std::time::Instant;

use gpui::{
    AnyElement, App, Context, Div, ImageSource, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, deferred, div, img, px,
};
use vesper_protocol::{Attachment, AttachmentKind};

use super::c::{self, Pic};
use super::icons::icon;
use super::motion::{self, POP};
use super::theme::{TEXT_SM, TEXT_XS, TextStepExt, Tokens, tokens};
use super::{Vesper, chat};
use crate::files::IMAGE_MAX;
use crate::net::Cmd;

/// The key context of the open viewer.
pub const VIEWER: &str = "Viewer";

pub struct Viewer {
    pub items: Vec<Attachment>,
    pub at: usize,
    since: Instant,
    /// Full-size pictures by upload id, while open.
    full: HashMap<String, Pic>,
}

/// The stage's size for a picture of `w` x `h` px: scaled down to fit `max`, never up.
pub fn fit(w: f32, h: f32, max: (f32, f32)) -> (f32, f32) {
    let k = (max.0 / w).min(max.1 / h).min(1.);
    ((w * k).round().max(1.), (h * k).round().max(1.))
}

impl Vesper {
    /// The pictures of the loaded part of the room, oldest first: the ones the timeline draws.
    pub(super) fn room_pictures(&self) -> Vec<Attachment> {
        self.model.log().map_or(vec![], |l| {
            l.messages
                .iter()
                .filter(|m| !m.deleted)
                .flat_map(|m| &m.attachments)
                .filter(|a| a.kind == AttachmentKind::Image && a.size as u64 <= IMAGE_MAX)
                .cloned()
                .collect()
        })
    }

    /// Opens the viewer on picture `id`.
    pub(super) fn open_viewer(&mut self, id: &str, cx: &mut Context<Self>) {
        let items = self.room_pictures();
        let at = items.iter().position(|a| a.id == id).unwrap_or(0);
        if items.is_empty() {
            return;
        }
        self.viewer = Some(Viewer {
            items,
            at,
            since: Instant::now(),
            full: HashMap::new(),
        });
        self.viewer_fetch();
        cx.notify();
    }

    /// Asks for the shown picture once.
    fn viewer_fetch(&mut self) {
        let Some(v) = &mut self.viewer else { return };
        let id = v.items[v.at].id.clone();
        if !v.full.contains_key(&id) {
            v.full.insert(id.clone(), Pic::Loading);
            self.run(vec![Cmd::FetchFull(id)]);
        }
    }

    pub(super) fn viewer_go(&mut self, d: isize, cx: &mut Context<Self>) {
        let Some(v) = &mut self.viewer else {
            return cx.propagate();
        };
        let n = v.items.len();
        if n < 2 {
            return;
        }
        v.at = (v.at as isize + d).rem_euclid(n as isize) as usize;
        self.viewer_fetch();
        cx.notify();
    }

    pub(super) fn close_viewer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(v) = self.viewer.take() {
            for (_, p) in v.full {
                if let Pic::Ready(i) = p {
                    let _ = window.drop_image(i);
                }
            }
        }
        cx.notify();
    }

    /// A full-size picture came; dropped when the viewer closed meanwhile.
    pub(super) fn full_loaded(
        &mut self,
        id: &str,
        picture: Option<crate::files::Picture>,
        window: &mut Window,
    ) {
        let image = picture.and_then(chat::render_image);
        match self.viewer.as_mut().and_then(|v| v.full.get_mut(id)) {
            Some(p) => *p = image.map_or(Pic::Failed, Pic::Ready),
            None => {
                if let Some(i) = image {
                    let _ = window.drop_image(i);
                }
            }
        }
    }

    pub(super) fn render_viewer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let v = self.viewer.as_ref()?;
        let t = tokens(cx).clone();
        let pose = motion::pose(&t.motion, POP, false, v.since, Instant::now());
        if pose.is_some() {
            window.request_animation_frame();
        }
        let a = v.items[v.at].clone();
        let n = v.items.len();
        let vp = window.viewport_size();
        // `dialog.wide`: from 440 up to min(1040, 100vw - 64); the picture at most min(640, 100vh - 220) tall.
        let max_w = (f32::from(vp.width) - 64.).min(1040.);
        let inner_max = (max_w - 42., (f32::from(vp.height) - 220.).min(640.));
        let (stage, width) = match v.full.get(&a.id) {
            Some(Pic::Ready(i)) => {
                let s = i.size(0);
                let (w, h) = fit(s.width.0 as f32, s.height.0 as f32, inner_max);
                let el = img(ImageSource::Render(i.clone())).w(px(w)).h(px(h));
                (
                    stage(&t).bg(gpui::transparent_black()).child(el),
                    (w + 42.).max(440.),
                )
            }
            Some(Pic::Failed) => (
                stage(&t)
                    .text_step(TEXT_XS)
                    .text_color(t.fg_2)
                    .child(chat::IMAGE_FAILED),
                440.,
            ),
            _ => (stage(&t), 440.),
        };
        let (open, save) = (a.clone(), a.clone());
        let nav = (n > 1).then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(
                    div().debug_selector(|| "viewer-prev".into()).child(
                        c::IconButton::new("viewer-prev", "chevronLeft")
                            .on_click(cx.listener(|this, _, _, cx| this.viewer_go(-1, cx))),
                    ),
                )
                .child(
                    div().debug_selector(|| "viewer-next".into()).child(
                        c::IconButton::new("viewer-next", "chevronRight")
                            .on_click(cx.listener(|this, _, _, cx| this.viewer_go(1, cx))),
                    ),
                )
                .child(
                    div()
                        .pl(px(4.))
                        .text_step(TEXT_XS)
                        .text_color(t.fg_2)
                        .child(format!("{} of {n}", v.at + 1)),
                )
        });
        let footer = div()
            .flex()
            .flex_1()
            .items_center()
            .gap(px(8.))
            .children(nav)
            .child(div().flex_1())
            .child(lined(
                "viewer-open",
                "externalLink",
                "Open original",
                &t,
                cx.listener(move |this, _, _, cx| this.open_attachment(&open, cx)),
            ))
            .child(lined(
                "viewer-download",
                "download",
                "Download",
                &t,
                cx.listener(move |this, _, _, cx| this.save_attachment(&save, cx)),
            ));
        let close = cx.listener(|this, _: &gpui::ClickEvent, w, cx| this.close_viewer(w, cx));
        Some(
            deferred(
                div()
                    .absolute()
                    .inset_0()
                    .track_focus(&self.dialog_focus)
                    .key_context(VIEWER)
                    .debug_selector(|| "viewer".into())
                    .opacity(pose.map_or(1., |p| p.opacity))
                    .child(
                        c::Dialog::new(a.name.clone())
                            .width(width.min(max_w))
                            .child(stage)
                            .footer(footer)
                            .on_close(close),
                    ),
            )
            .with_priority(3)
            .into_any_element(),
        )
    }
}

/// `.stage`: at least 96 px tall, the picture centred, `--radius`, `--bg-element` (none once the picture is shown).
fn stage(t: &Tokens) -> Div {
    div()
        .flex()
        .min_h(px(96.))
        .items_center()
        .justify_center()
        .rounded(t.radius)
        .overflow_hidden()
        .bg(t.bg_element)
}

/// `.mini-act.lined`: 32 px, 12 px sides, 6 px gap, a `--line` border (`--line-strong` on hover), `--radius`, 13/20
/// `--fg`, `--weight-ui`, a 16 px icon; `--bg-hover` on hover.
fn lined(
    id: &'static str,
    name: &'static str,
    label: &'static str,
    t: &Tokens,
    f: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let (hover, strong): (gpui::Rgba, gpui::Rgba) = (t.bg_hover, t.line_strong);
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_none()
        .h(px(32.))
        .items_center()
        .gap(px(6.))
        .px(px(12.))
        .border_1()
        .border_color(t.line)
        .rounded(t.radius)
        .text_step(TEXT_SM)
        .font_weight(t.weight_ui)
        .text_color(t.fg)
        .cursor_pointer()
        .hover(move |s| s.bg(hover).border_color(strong))
        .on_click(f)
        .child(icon(name).text_color(t.fg))
        .child(label)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pictures_fit_down_never_up() {
        assert_eq!(super::fit(400., 300., (1000., 640.)), (400., 300.));
        assert_eq!(super::fit(2000., 1000., (1000., 640.)), (1000., 500.));
        assert_eq!(super::fit(1000., 2000., (1000., 640.)), (320., 640.));
    }
}
