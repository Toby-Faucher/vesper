//! The first screen, drawn like the web's `Login.svelte` in `AuthCard.svelte`: the logo over one card, anchored near
//! the top third. The desktop adds the server address field (the web knows its server) and the keyring note.

use std::sync::{Arc, OnceLock};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, Div, Image, ImageFormat, InteractiveElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, img, px,
};

use super::Vesper;
use super::c::{Button, Field, PasswordInput, text_input};
use super::theme::{SEMIBOLD, TEXT_SM, TEXT_XL, TextStepExt, tokens};
use crate::model::{INSECURE, NOT_REMEMBERED};

/// The web's logo (`Mark.svelte`): the wordmark, 235x112, drawn at half size.
pub(super) fn logo() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Webp,
            include_bytes!("../../../web/src/lib/ui/logo.webp").to_vec(),
        ))
    })
    .clone()
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
                            .child(img(logo()).h(px(56.)).w(px(118.)).flex_none()),
                    )
                    .child(card)
                    .child(below),
            );
        div().size_full().child(page)
    }
}
