//! The first screen, drawn like the web's `Login.svelte` in `AuthCard.svelte`: the logo over one card, anchored near
//! the top third. The desktop adds the server address field (the web knows its server) and the keyring note.

use std::sync::{Arc, Mutex};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, Div, InteractiveElement, ParentElement, RenderImage, StatefulInteractiveElement,
    Styled, Window, div, img, px,
};

use super::Vesper;
use super::c::{Button, Field, PasswordInput, text_input};
use super::theme::{self, SEMIBOLD, TEXT_SM, TEXT_XL, TextStepExt, tokens};
use crate::model::{INSECURE, NOT_REMEMBERED};

/// The web's logo (`Mark.svelte`): the wordmark, 235x112, drawn at half size, in the accent `fill` like the web's
/// `--logo-filter` (`theme::tint` on each pixel). The last fill's picture is kept, so a frame does not redo it.
pub(super) fn logo(fill: u32) -> Arc<RenderImage> {
    static LOGO: Mutex<Option<(u32, Arc<RenderImage>)>> = Mutex::new(None);
    let mut kept = LOGO.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((f, pic)) = kept.as_ref()
        && *f == fill
    {
        return pic.clone();
    }
    let mut px = image::load_from_memory_with_format(
        include_bytes!("../../../web/src/lib/ui/logo.webp"),
        image::ImageFormat::WebP,
    )
    .expect("the logo is a webp")
    .to_rgba8();
    let m = theme::tint(fill);
    for p in px.pixels_mut() {
        let x = [p[0], p[1], p[2]].map(|v| f64::from(v) / 255.);
        let [r, g, b] = m.map(|row| {
            ((row[0] * x[0] + row[1] * x[1] + row[2] * x[2]).clamp(0., 1.) * 255.).round() as u8
        });
        (p[0], p[1], p[2]) = (b, g, r); // GPUI draws BGRA
    }
    let pic = Arc::new(RenderImage::new(vec![image::Frame::new(px)]));
    *kept = Some((fill, pic.clone()));
    pic
}

impl Vesper {
    pub(super) fn render_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx).clone();
        let m = &self.model;
        let insecure = crate::net::insecure(&self.server.read(cx).value());
        let line = |text: &str, color| {
            div()
                .text_step(TEXT_SM)
                .text_color(color)
                .child(text.to_string())
        };
        let form = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                Field::new("Server address")
                    .child(
                        div()
                            .debug_selector(|| "sign-in-server".into())
                            .child(text_input(&self.server, cx)),
                    )
                    .when(insecure, |f| {
                        f.child(
                            line(INSECURE, t.warn_fg)
                                .mt_1()
                                .debug_selector(|| "sign-in-insecure".into()),
                        )
                    }),
            )
            .child(Field::new("Username").child(text_input(&self.username, cx)))
            .child(Field::new("Password").child(PasswordInput::new(
                &self.password,
                self.password_shown,
                {
                    let this = cx.entity().downgrade();
                    move |shown, _: &mut Window, cx: &mut gpui::App| {
                        let _ = this.update(cx, |v, cx| {
                            v.password_shown = shown;
                            cx.notify();
                        });
                    }
                },
            )))
            .child(
                div().debug_selector(|| "sign-in-button".into()).child(
                    Button::new("sign-in", "Sign in")
                        .primary()
                        .w_full()
                        .loading(m.busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.submit_sign_in(window, cx)),
                        ),
                ),
            );
        let card = div()
            .rounded(t.radius_lg)
            .border_1()
            .border_color(t.line_subtle)
            .bg(t.bg_raised)
            .p_6()
            .child(
                div()
                    .mb_5()
                    .text_step(TEXT_XL)
                    .font_weight(SEMIBOLD)
                    .text_color(t.fg)
                    .child("Sign in"),
            )
            .when(!m.notice.is_empty(), |d| {
                d.child(
                    line(&m.notice, t.warn_fg)
                        .mb_4()
                        .debug_selector(|| "sign-in-notice".into()),
                )
            })
            .child(form)
            .when(!m.error.is_empty(), |d| {
                d.child(
                    line(&m.error, t.danger_fg)
                        .mt_4()
                        .debug_selector(|| "sign-in-error".into()),
                )
            });
        let below = div()
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .mt_4()
            .gap_1()
            .child(line("Need an account? Ask for an invite link.", t.fg_3))
            .child(line(
                "Forgot your password? Ask the owner for a reset link.",
                t.fg_3,
            ))
            .when(m.not_remembered, |d| {
                d.child(
                    div()
                        .debug_selector(|| "sign-in-keyring".into())
                        .child(line(NOT_REMEMBERED, t.fg_3)),
                )
            });
        // pt: max(40px, 22vh), as AuthCard.
        let top = (window.viewport_size().height * 0.22).max(px(40.));
        let page = div()
            .id("sign-in")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .px_4()
            .pb_10()
            .pt(top)
            .child(
                div()
                    .w_full()
                    .max_w(px(360.))
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .mb_5()
                            .child(img(logo(t.fill)).h(px(56.)).w(px(118.)).flex_none()),
                    )
                    .child(card)
                    .child(below),
            );
        div().size_full().child(page)
    }
}
