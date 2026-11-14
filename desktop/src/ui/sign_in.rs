//! The first screen: server address, username, password.

use gpui::{Context, Div, InteractiveElement, ParentElement, Styled, Window, div, px, rgb};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::Input;
use gpui_component::v_flex;

use super::{Vesper, theme};
use crate::model::NOT_REMEMBERED;

fn label(text: &'static str) -> Div {
    div()
        .text_size(px(13.))
        .text_color(rgb(theme::FG_2))
        .child(text)
}

impl Vesper {
    pub(super) fn render_sign_in(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Div {
        let m = &self.model;
        let card = v_flex()
            .w(px(380.))
            .gap_2()
            .p_6()
            .bg(rgb(theme::BG_RAISED))
            .border_1()
            .border_color(rgb(theme::LINE_SUBTLE))
            .rounded(px(10.))
            .child(
                div()
                    .text_size(px(20.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Sign in to Vesper"),
            )
            .when_some_text(&m.notice, theme::FG_2, "sign-in-notice")
            .child(label("Server address"))
            .child(
                div()
                    .debug_selector(|| "sign-in-server".into())
                    .child(Input::new(&self.server)),
            )
            .child(label("Username"))
            .child(Input::new(&self.username))
            .child(label("Password"))
            .child(Input::new(&self.password))
            .when_some_text(&m.error, theme::DANGER_FG, "sign-in-error")
            .child(
                div()
                    .mt_2()
                    .debug_selector(|| "sign-in-button".into())
                    .child(
                        Button::new("sign-in")
                            .primary()
                            .w_full()
                            .label("Sign in")
                            .loading(m.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.submit_sign_in(window, cx)),
                            ),
                    ),
            )
            .when_some_text(
                if m.not_remembered { NOT_REMEMBERED } else { "" },
                theme::FG_3,
                "sign-in-keyring",
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::FG_3))
                    .child("New here? Use your invite link in a browser to make an account."),
            );
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(card)
    }
}

/// A line of text in `color`, left out when empty.
pub(super) trait TextLine: ParentElement + Sized {
    fn when_some_text(self, text: &str, color: u32, selector: &'static str) -> Self {
        if text.is_empty() {
            return self;
        }
        self.child(
            div()
                .debug_selector(|| selector.into())
                .text_size(px(13.))
                .text_color(rgb(color))
                .child(text.to_string()),
        )
    }
}

impl<T: ParentElement + Sized> TextLine for T {}
