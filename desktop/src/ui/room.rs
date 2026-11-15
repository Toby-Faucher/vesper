//! The main screen's right side: the open room's header (`RoomHeader.svelte`), messages and composer. The sidebar
//! and the shell around both are in `sidebar.rs`.

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, Div, ExternalPaths, InteractiveElement, ParentElement, Styled, Window, div, list,
    px, rgb,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{h_flex, v_flex};
use vesper_protocol::{RoomInfo, RoomKind};

use super::c::{self, Ellipsis};
use super::icons::icon;
use super::theme::{TEXT_BASE, TEXT_SM, TEXT_XS, TextStepExt, tokens};
use super::{COMPOSER, Vesper, theme};
use crate::model::{Model, NOT_REMEMBERED, Phase};

/// `EmptyState.svelte`: the title (16/24, `--weight-strong`), the body (13/20, `--fg-3`, at most 320 px), centred in
/// the pane, `lift` px up (48 for the header in a page, `.center` in Shell.svelte; none in the timeline).
fn empty_state(title: &'static str, body: &'static str, lift: f32, cx: &App) -> Div {
    let t = tokens(cx);
    v_flex().flex_1().justify_center().pb(px(lift)).child(
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
        let banner = self.render_banner(window, cx);
        if self.settings_open {
            let settings = self.render_settings(cx);
            let collapsed = self.collapsed_header(cx);
            let main = self
                .main_pane(window, cx)
                .children(banner)
                .children(collapsed)
                .child(settings);
            let shell = self.shell(main, window, cx);
            return self.overlays(shell, window, cx);
        }
        self.sync_list(cx);
        self.sync_typing(cx);
        let m = &self.model;
        let room = m.active.and_then(|id| m.state.find_room(id));
        let Some(room) = room else {
            // No room open: the web's empty states (it opens the first room itself when there is one).
            let empty = (m.state.rooms.is_empty() && m.online).then(|| {
                empty_state(
                    "No rooms yet.",
                    "The owner can add one in the admin pages.",
                    48.,
                    cx,
                )
            });
            let collapsed = self.collapsed_header(cx);
            let main = self
                .main_pane(window, cx)
                .children(banner)
                .children(collapsed)
                .children(empty);
            let shell = self.shell(main, window, cx);
            return self.overlays(shell, window, cx);
        };
        let room = room.clone();
        if self.placeholder_room != Some(room.id) {
            self.placeholder_room = Some(room.id);
            let text = format!("Message {}", room.name);
            self.composer
                .update(cx, |i, cx| i.set_placeholder(text, window, cx));
        }
        // `No messages yet. Say hello.` once the first page came empty.
        let empty = self.tl.loaded.contains(&room.id)
            && m.log()
                .is_some_and(|l| l.messages.is_empty() && !l.more_before);
        let header = self.room_header(&room, cx);
        let m = &self.model;
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
        let (dy, opacity) = self.swap_pose(cx);
        let timeline = if empty {
            empty_state("No messages yet.", "Say hello.", 0., cx).pb(px(16.))
        } else {
            div().flex().flex_col().flex_1().min_h_0().child(
                list(
                    self.list.clone(),
                    cx.processor(|this, ix: usize, window, cx| this.message_row(ix, window, cx)),
                )
                .flex_1(),
            )
        };
        let timeline = div()
            .relative()
            .top(px(dy))
            .opacity(opacity)
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(timeline);
        let composer = self.render_composer(window, cx);
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
            .debug_selector(|| "composer".into())
            .relative()
            .child(self.probe("composer".into()))
            .children(self.saved_line(cx))
            .child(composer);
        let call_status = self.call_status(cx);
        let share = self.render_share(window, cx);
        let dialog = self.delete_dialog(cx);
        if self.timeline_moving() {
            window.request_animation_frame();
        }
        let main = self
            .main_pane(window, cx)
            .children(banner)
            .child(header)
            .children(keyring)
            .children(call_status)
            .children(share)
            // `pt-3` over the timeline (Chat.svelte).
            .child(div().h(px(12.)).flex_none())
            .child(timeline)
            .child(composer)
            // Files dropped anywhere on the room go to the tray.
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rgb(theme::ACCENT_SUBTLE)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.attach_paths(paths.paths().to_vec(), cx)
            }));
        let shell = self.shell(main, window, cx).children(dialog);
        self.overlays(shell, window, cx)
    }

    /// The main pane's column: while in a call it keeps the call bar's height and 24 px free at its bottom, and the
    /// bar floats there.
    fn main_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let call = self.render_call(window, cx);
        v_flex()
            .relative()
            .flex_1()
            .h_full()
            .min_w_0()
            .when(call.is_some(), |d| d.pb(px(super::call::BAR_H + 24.)))
            .children(call)
    }

    /// What floats over the whole window: the person card, its confirm, the ring prompt.
    fn overlays(&mut self, shell: Div, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let card = self.render_person(window, cx);
        let confirm = self.person_confirm(cx);
        let ring = self.render_ring(window, cx);
        shell.children(card).children(confirm).children(ring)
    }

    /// Where the last saved file went, with Show in folder (the desktop's own: the web's browser shows downloads).
    fn saved_line(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = tokens(cx);
        let p = self.model.saved.as_ref()?;
        let path = p.clone();
        Some(
            h_flex()
                .debug_selector(|| "saved".into())
                .gap(px(8.))
                .px(px(16.))
                .pb(px(4.))
                .text_step(TEXT_XS)
                .text_color(t.fg_2)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .ellipsis()
                        .child(format!("Saved to {}.", p.display())),
                )
                .child(
                    c::Button::new("saved-show", "Show in folder")
                        .small()
                        .variant(c::Variant::Ghost)
                        .on_click(cx.listener(move |_, _, _, cx| cx.reveal_path(&path))),
                )
                .child(
                    c::Button::new("saved-ok", "OK")
                        .small()
                        .variant(c::Variant::Ghost)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.model.saved = None;
                            cx.notify();
                        })),
                ),
        )
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
    /// the web's (the desktop has no message search yet).
    fn room_header(&mut self, room: &RoomInfo, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx).clone();
        let popover = self.render_pins(cx);
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
                    // As the web's: no join while offline.
                    .disabled(!m.online)
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
        let pins_open = m.pins.is_some();
        let pins = (room.kind == RoomKind::Room).then(|| {
            div()
                .relative()
                .debug_selector(|| "pins-button".into())
                .child(
                    c::IconButton::new("pins-button", "pin")
                        .pressed(pins_open)
                        .on_click(cx.listener(|this, _, _, cx| this.act(Model::toggle_pins, cx))),
                )
                .children(popover)
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
