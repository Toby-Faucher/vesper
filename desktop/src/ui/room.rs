//! The main screen's right side: the open room's header (`RoomHeader.svelte`), messages and composer. The sidebar
//! and the shell around both are in `sidebar.rs`.

use chrono::{Local, TimeZone};
use gpui::{
    App, Context, Div, ExternalPaths, InteractiveElement, ParentElement, Styled, Window, div, list,
    px, rgb,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::input::Input;
use gpui_component::{h_flex, v_flex};
use vesper_protocol::{RoomInfo, RoomKind};

use super::c::{self, Ellipsis};
use super::icons::icon;
use super::sign_in::TextLine;
use super::theme::{TEXT_BASE, TEXT_SM, TEXT_XS, TextStepExt, tokens};
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

/// `EmptyState.svelte`: the title (16/24, `--weight-strong`), the body (13/20, `--fg-3`, at most 320 px), centred in
/// the pane, 48 px up for the header (`.center` in Shell.svelte).
fn empty_state(title: &'static str, body: &'static str, cx: &App) -> Div {
    let t = tokens(cx);
    v_flex().flex_1().justify_center().pb(px(48.)).child(
        v_flex()
            .items_center()
            .gap(px(4.))
            .px(px(24.))
            .py(px(48.))
            .text_center()
            .child(
                div()
                    .text_step(TEXT_BASE)
                    .font_weight(t.weight_strong)
                    .text_color(t.fg)
                    .child(title),
            )
            .child(
                div()
                    .max_w(px(320.))
                    .text_step(TEXT_SM)
                    .text_color(t.fg_3)
                    .child(body),
            ),
    )
}

impl Vesper {
    pub(super) fn render_main(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        self.sync_share(window, cx);
        if self.settings_open {
            let settings = self.render_settings(cx);
            let call = self.render_call(cx);
            let collapsed = self.collapsed_header(cx);
            let main = v_flex()
                .flex_1()
                .h_full()
                .min_w_0()
                .children(collapsed)
                .child(settings)
                .children(call);
            return self.shell(main, window, cx);
        }
        self.sync_list();
        self.sync_typing(cx);
        let m = &self.model;
        let room = m.active.and_then(|id| m.state.find_room(id));
        let status = if !m.notice.is_empty() {
            m.notice.as_str()
        } else if !m.online {
            "Offline. Reconnecting..."
        } else {
            ""
        };
        let Some(room) = room else {
            // No room open: the web's empty states (it opens the first room itself when there is one).
            let empty = (m.state.rooms.is_empty() && m.online).then(|| {
                empty_state(
                    "No rooms yet.",
                    "The owner can add one in the admin pages.",
                    cx,
                )
            });
            let collapsed = self.collapsed_header(cx);
            let main = v_flex()
                .flex_1()
                .h_full()
                .min_w_0()
                .children(collapsed)
                .children(empty);
            return self.shell(main, window, cx);
        };
        let start = m
            .log()
            .is_some_and(|l| !l.more_before && !l.messages.is_empty());
        let header = self.room_header(room, status, cx);
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
            cx.processor(|this, ix: usize, _, cx| this.message_row(ix, cx)),
        )
        .flex_1();
        let error = m.error.clone();
        let pins = self.render_pins(cx);
        let above = self.above_composer(cx);
        let composer = v_flex()
            .key_context(COMPOSER)
            .on_action(cx.listener(Self::send))
            .on_action(cx.listener(Self::new_line))
            .on_action(cx.listener(Self::composer_up))
            .on_action(cx.listener(Self::composer_down))
            .on_action(cx.listener(Self::composer_tab))
            .on_action(cx.listener(Self::composer_escape))
            .on_action(cx.listener(Self::paste_file))
            .flex_none()
            .p_3()
            .gap_1()
            .debug_selector(|| "composer".into())
            .when_some_text(&error, theme::DANGER_FG, "room-error")
            .children(above)
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(
                        div().debug_selector(|| "attach".into()).child(
                            Button::new("attach")
                                .ghost()
                                .label("Attach")
                                .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx))),
                        ),
                    )
                    .child(div().flex_1().child(Input::new(&self.composer))),
            );
        let call = self.render_call(cx);
        let share = self.render_share(cx);
        let m = &self.model;
        let main = v_flex()
            .flex_1()
            .h_full()
            .min_w_0()
            .child(header)
            .children(pins)
            .children(keyring)
            .children(share)
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
            .child(composer)
            // Files dropped anywhere on the room go to the tray.
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rgb(theme::ACCENT_SUBTLE)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.attach_paths(paths.paths().to_vec(), cx)
            }));
        self.shell(main, window, cx)
    }

    /// The Show sidebar button the web puts at the left of a page header while the sidebar is hidden.
    fn show_sidebar(&self, cx: &mut Context<Self>) -> Option<Div> {
        self.model.settings.sidebar_collapsed.then(|| {
            div()
                .ml(px(-8.))
                .debug_selector(|| "show-sidebar".into())
                .child(
                    c::IconButton::new("show-sidebar", "sidebar")
                        .on_click(cx.listener(|this, _, _, cx| this.set_collapsed(false, cx))),
                )
        })
    }

    /// `pageHeader('')` in Shell.svelte: drawn only to hold Show sidebar.
    fn collapsed_header(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = tokens(cx);
        let line = t.line_subtle;
        self.show_sidebar(cx).map(|b| {
            h_flex()
                .h(px(48.))
                .flex_none()
                .gap(px(12.))
                .px(px(16.))
                .border_b_1()
                .border_color(line)
                .child(b)
        })
    }

    /// `RoomHeader.svelte`: 48 px, 16 px sides, 12 px gap, a `--line-subtle` rule under it. The name (16/24,
    /// `--weight-strong`), a DM's or a restricted room's note, then Pins and Join voice at the right. Join voice is
    /// not there in this room's call (the call bar has Leave) and is secondary while in a call elsewhere. Search is
    /// the web's (the desktop has no message search yet). The desktop adds the status line (offline, notices).
    fn room_header(&self, room: &RoomInfo, status: &str, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx).clone();
        let m = &self.model;
        let id = room.id;
        let here = m.call.room == Some(id);
        let joined = m.call.phase == Phase::Joined;
        let joining = here && matches!(m.call.phase, Phase::Starting | Phase::Joining);
        let elsewhere = m.call.phase != Phase::Idle && !here;
        let note = match room.kind {
            RoomKind::Dm => Some("Direct message"),
            RoomKind::Room if room.restricted => Some("Restricted"),
            RoomKind::Room => None,
        };
        let join = (!(here && joined)).then(|| {
            div().debug_selector(|| "join-voice".into()).child(
                c::Button::new("join-voice", "Join voice")
                    .variant(if elsewhere {
                        c::Variant::Secondary
                    } else {
                        c::Variant::Primary
                    })
                    .loading(joining)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.act(
                            |m| {
                                let mut cmds = if m.call.phase != Phase::Idle {
                                    m.leave_voice()
                                } else {
                                    vec![]
                                };
                                cmds.extend(m.join_voice(id));
                                cmds
                            },
                            cx,
                        )
                    })),
            )
        });
        let pins = (room.kind == RoomKind::Room).then(|| {
            div().debug_selector(|| "pins-button".into()).child(
                c::IconButton::new("pins-button", "pin")
                    .pressed(m.pins.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.act(Model::toggle_pins, cx))),
            )
        });
        h_flex()
            .h(px(48.))
            .flex_none()
            .gap(px(12.))
            .px(px(16.))
            .border_b_1()
            .border_color(t.line_subtle)
            .children(self.show_sidebar(cx))
            .child(
                div()
                    .min_w_0()
                    .ellipsis()
                    .text_step(TEXT_BASE)
                    .font_weight(t.weight_strong)
                    .text_color(t.fg)
                    .child(room.name.clone()),
            )
            .children(note.map(|n| {
                h_flex()
                    .flex_none()
                    .gap(px(4.))
                    .text_step(TEXT_XS)
                    .text_color(t.fg_2)
                    .child(icon("shield").text_color(t.fg_2))
                    .child(n)
            }))
            .when_some_text(status, theme::WARN_FG, "status")
            .child(
                // flex_1 and justify_end, not ml_auto: taffy drops the gaps before an auto margin.
                h_flex()
                    .flex_1()
                    .justify_end()
                    .gap(px(8.))
                    .children(pins)
                    .children(join),
            )
    }
}
