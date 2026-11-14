//! The main screen: rooms and DMs on the left, the shown room's messages and the composer on the right.

use chrono::{Local, TimeZone};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, Context, Div, FontWeight, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, list, px, rgb,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::Input;
use gpui_component::{h_flex, v_flex};
use vesper_protocol::{RoomInfo, RoomKind};

use super::sign_in::TextLine;
use super::{COMPOSER, Vesper, theme};
use crate::model::{Model, NOT_REMEMBERED, Phase};

/// `14:05` today, `2026-11-13 14:05` before.
pub fn time_label(ms: i64) -> String {
    let Some(t) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    if t.date_naive() == Local::now().date_naive() {
        t.format("%H:%M").to_string()
    } else {
        t.format("%Y-%m-%d %H:%M").to_string()
    }
}

fn pill(text: String, bg: u32, fg: u32) -> Div {
    div()
        .px_1p5()
        .rounded(px(8.))
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .text_size(px(12.))
        .child(text)
}

fn heading(text: &'static str) -> Div {
    div()
        .px_2()
        .pt_3()
        .pb_1()
        .text_size(px(12.))
        .text_color(rgb(theme::FG_3))
        .child(text)
}

/// One message: author and time, the quoted reply, the body as plain text, attachment names.
fn message_row(m: &Model, ix: usize) -> AnyElement {
    let Some(msg) = m.log().and_then(|l| l.messages.get(ix)) else {
        return div().into_any_element();
    };
    let s = &m.state;
    let mut row = v_flex()
        .debug_selector(|| format!("message-{}", msg.id))
        .px_4()
        .py_1p5()
        .gap_0p5()
        .child(
            h_flex()
                .gap_2()
                .items_baseline()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(s.name_of(&msg.author_id)),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(theme::FG_3))
                        .child(time_label(msg.created_at)),
                )
                .when_some_text(
                    if msg.edited_at.is_some() && !msg.deleted {
                        "(edited)"
                    } else {
                        ""
                    },
                    theme::FG_3,
                    "",
                ),
        );
    if let Some(r) = &msg.reply_to {
        let excerpt = if r.excerpt.is_empty() {
            "a deleted message".to_string()
        } else {
            r.excerpt.clone()
        };
        row = row.child(
            div()
                .pl_2()
                .border_l_2()
                .border_color(rgb(theme::LINE))
                .text_size(px(13.))
                .text_color(rgb(theme::FG_3))
                .child(format!("{}: {excerpt}", s.name_of(&r.author_id))),
        );
    }
    row = if msg.deleted {
        row.child(
            div()
                .italic()
                .text_color(rgb(theme::FG_3))
                .child("Message deleted"),
        )
    } else {
        row.when_some_text(&msg.body, theme::FG_2, "")
    };
    for a in &msg.attachments {
        row = row.child(
            div()
                .text_size(px(13.))
                .text_color(rgb(theme::ACCENT_FG))
                .child(format!("Attachment: {}", a.name)),
        );
    }
    row.into_any_element()
}

/// The room's title: `# general`, or the other person's name for a DM.
pub(super) fn title(r: &RoomInfo) -> String {
    match r.kind {
        RoomKind::Room => format!("# {}", r.name),
        RoomKind::Dm => r.name.clone(),
    }
}

impl Vesper {
    fn room_row(&self, r: &RoomInfo, cx: &mut Context<Self>) -> impl IntoElement {
        let m = &self.model;
        let id = r.id;
        let selected = m.active == Some(id);
        let u = m.state.unread.get(&id);
        let (unread, mentions) = u.map_or((0, 0), |u| (u.unread, u.mentions));
        let voice = m.state.voice_in(id);
        v_flex()
            .id(SharedString::from(format!("room-{id}")))
            .debug_selector(|| format!("room-{id}"))
            .mx_1()
            .px_2()
            .py_1()
            .rounded(px(6.))
            .cursor_pointer()
            .when(selected, |s| s.bg(rgb(theme::BG_SELECTED)))
            .hover(|s| s.bg(rgb(theme::BG_HOVER)))
            .on_click(cx.listener(move |this, _, window, cx| this.open_room(id, window, cx)))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_color(rgb(if unread > 0 || selected {
                                theme::FG
                            } else {
                                theme::FG_2
                            }))
                            .when(unread > 0, |s| s.font_weight(FontWeight::SEMIBOLD))
                            .child(title(r)),
                    )
                    .children(
                        (mentions > 0)
                            .then(|| pill(format!("@{mentions}"), theme::ACCENT, theme::ON_ACCENT)),
                    )
                    .children(
                        (unread > 0)
                            .then(|| pill(unread.to_string(), theme::BG_SELECTED, theme::FG)),
                    ),
            )
            .children((!voice.is_empty()).then(|| {
                div()
                    .pl_3()
                    .text_size(px(12.))
                    .text_color(rgb(theme::FG_3))
                    .child(format!("In voice: {}", voice.join(", ")))
            }))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.model.state;
        let rooms: Vec<_> = s
            .rooms
            .iter()
            .map(|r| self.room_row(r, cx).into_any_element())
            .collect();
        let dms: Vec<_> = s
            .sorted_dms()
            .into_iter()
            .map(|r| self.room_row(r, cx).into_any_element())
            .collect();
        let me =
            s.me.as_ref()
                .map_or(String::new(), |m| m.user.display_name.clone());
        v_flex()
            .w(px(240.))
            .h_full()
            .flex_none()
            .bg(rgb(theme::BG_SIDEBAR))
            .border_r_1()
            .border_color(rgb(theme::LINE_SUBTLE))
            .child(
                v_flex()
                    .id("sidebar-rooms")
                    .flex_1()
                    .overflow_y_scroll()
                    .pb_2()
                    .child(heading("Rooms"))
                    .children(rooms)
                    .children((!dms.is_empty()).then(|| heading("Direct messages")))
                    .children(dms),
            )
            .child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .border_t_1()
                    .border_color(rgb(theme::LINE_SUBTLE))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .text_color(rgb(theme::FG_2))
                            .child(me),
                    )
                    .child(
                        Button::new("sign-out")
                            .ghost()
                            .label("Sign out")
                            .on_click(cx.listener(|this, _, window, cx| this.sign_out(window, cx))),
                    ),
            )
    }

    pub(super) fn render_main(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Div {
        self.sync_list();
        let m = &self.model;
        let room = m.active.and_then(|id| m.state.find_room(id));
        let status = if !m.notice.is_empty() {
            m.notice.as_str()
        } else if !m.online {
            "Offline. Reconnecting..."
        } else {
            ""
        };
        let in_call = m.call.phase != Phase::Idle && m.call.room == m.active;
        let voice = m.active.map(|id| {
            let button = if in_call {
                Button::new("leave-voice")
                    .label("Leave")
                    .on_click(cx.listener(|this, _, _, cx| this.act(Model::leave_voice, cx)))
            } else {
                Button::new("join-voice")
                    .primary()
                    .label("Join voice")
                    .on_click(cx.listener(move |this, _, _, cx| this.act(|m| m.join_voice(id), cx)))
            };
            let selector = if in_call { "leave-voice" } else { "join-voice" };
            div().debug_selector(|| selector.into()).child(button)
        });
        let start = m
            .log()
            .is_some_and(|l| !l.more_before && !l.messages.is_empty());
        let header = h_flex()
            .h(px(48.))
            .flex_none()
            .px_4()
            .gap_3()
            .border_b_1()
            .border_color(rgb(theme::LINE_SUBTLE))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(room.map(title).unwrap_or_default()),
            )
            .when_some_text(status, theme::WARN_FG, "status")
            .child(div().flex_1())
            .children(voice);
        let keyring = m.not_remembered.then(|| {
            h_flex()
                .gap_3()
                .px_4()
                .py_2()
                .bg(rgb(theme::ACCENT_SUBTLE))
                .text_size(px(13.))
                .child(div().flex_1().child(NOT_REMEMBERED))
                .child(
                    Button::new("keyring-ok")
                        .ghost()
                        .label("OK")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.model.not_remembered = false;
                            cx.notify();
                        })),
                )
        });
        let timeline = list(
            self.list.clone(),
            cx.processor(|this, ix: usize, _, _| message_row(&this.model, ix)),
        )
        .flex_1();
        let composer = v_flex()
            .key_context(COMPOSER)
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(Self::new_line))
            .flex_none()
            .p_3()
            .gap_1()
            .debug_selector(|| "composer".into())
            .when_some_text(&m.error, theme::DANGER_FG, "room-error")
            .child(Input::new(&self.composer));
        let call = self.render_call(cx);
        let m = &self.model;
        let main = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(header)
            .children(keyring)
            .when_some_text(
                if m.loading_older {
                    "Loading older messages..."
                } else {
                    ""
                },
                theme::FG_3,
                "",
            )
            .when_some_text(
                if start {
                    "This is the start of the conversation."
                } else {
                    ""
                },
                theme::FG_3,
                "",
            )
            .child(timeline)
            .children(call)
            .child(composer);
        div()
            .size_full()
            .flex()
            .child(self.render_sidebar(cx))
            .child(main)
    }
}
