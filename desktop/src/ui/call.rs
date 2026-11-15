//! The call bar at the bottom of the main pane, while in voice: the room, mute, deafen, leave, and the people in the
//! call with a speaking ring and a volume slider each.

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AppContext, Context, Div, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Styled, div, px, rgb,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::slider::{Slider, SliderEvent, SliderState};
use gpui_component::{Disableable, Selectable, h_flex, v_flex};

use super::sign_in::TextLine;
use super::{Vesper, theme};
use crate::model::{HEADPHONES, Model, Phase, REJOINING};
use crate::net::Cmd;

/// A person in the call: name, speaking ring, state, and for the others a volume slider.
fn person(name: String, id: &str, speaking: bool, note: &str, slider: Option<AnyElement>) -> Div {
    let id = id.to_string();
    v_flex()
        .debug_selector(|| format!("call-person-{id}"))
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .when(speaking, |d| d.debug_selector(|| format!("speaking-{id}")))
                        .px_2()
                        .py_0p5()
                        .rounded(px(12.))
                        .border_2()
                        .border_color(rgb(if speaking {
                            theme::ACCENT
                        } else {
                            theme::LINE_SUBTLE
                        }))
                        .bg(rgb(theme::BG_RAISED))
                        .child(name),
                )
                .when_some_text(note, theme::FG_3, ""),
        )
        .children(slider)
}

impl Vesper {
    /// Runs one model action and redraws.
    pub(super) fn act(&mut self, f: impl FnOnce(&mut Model) -> Vec<Cmd>, cx: &mut Context<Self>) {
        let cmds = f(&mut self.model);
        self.run(cmds);
        cx.notify();
    }

    /// A volume slider for `id`, made once and kept.
    fn slider(&mut self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        if !self.sliders.contains_key(id) {
            let start = self.model.volume(id);
            let state = cx.new(|_| {
                SliderState::new()
                    .min(0.)
                    .max(400.)
                    .step(5.)
                    .default_value(start)
            });
            let user = id.to_string();
            let sub = cx.subscribe(&state, move |this, _, e: &SliderEvent, cx| {
                let SliderEvent::Change(v) = e;
                let v = v.start();
                this.act(|m| m.set_volume(&user, v), cx);
                this.save_settings();
            });
            self.sliders.insert(id.to_string(), (state, sub));
        }
        let state = &self.sliders[id].0;
        let percent = self.model.volume(id).round();
        let id = id.to_string();
        h_flex()
            .debug_selector(|| format!("volume-{id}"))
            .gap_2()
            .child(div().w(px(120.)).child(Slider::new(state)))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::FG_3))
                    .child(format!("{percent}%")),
            )
            .into_any_element()
    }

    pub(super) fn render_call(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let c = &self.model.call;
        let headphones = c.headphones.then(|| {
            h_flex()
                .gap_3()
                .text_size(px(13.))
                .child(div().flex_1().child(HEADPHONES))
                .child(
                    Button::new("headphones-ok")
                        .ghost()
                        .label("OK")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.model.call.headphones = false;
                            cx.notify();
                        })),
                )
        });
        if c.phase == Phase::Idle {
            // No call: what ended the last one, or a rejoin on its way.
            let text = if c.rejoin.is_some() {
                REJOINING
            } else {
                c.error.as_str()
            };
            if text.is_empty() {
                return None;
            }
            let rejoin = c.rejoin.is_some();
            return Some(
                h_flex()
                    .debug_selector(|| "call-bar".into())
                    .flex_none()
                    .gap_3()
                    .px_4()
                    .py_2()
                    .border_t_1()
                    .border_color(rgb(theme::LINE_SUBTLE))
                    .bg(rgb(theme::BG_SIDEBAR))
                    .child(div().flex_1().when_some_text(
                        text,
                        if rejoin {
                            theme::WARN_FG
                        } else {
                            theme::DANGER_FG
                        },
                        "call-error",
                    ))
                    .child(
                        Button::new("call-dismiss")
                            .ghost()
                            .label(if rejoin { "Leave" } else { "OK" })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.act(Model::leave_voice, cx);
                                this.model.call.error.clear();
                            })),
                    ),
            );
        }

        let room = c
            .room
            .and_then(|id| self.model.state.find_room(id))
            .map(|r| r.name.clone())
            .unwrap_or_default();
        let status = if c.connected {
            "Connected"
        } else {
            "Connecting..."
        };
        let set = &self.model.settings;
        let ptt = set.push_to_talk.then(|| {
            let key = crate::voice::ptt::key_label(&set.ptt_key);
            if self.model.ptt_open {
                format!("Talking ({key})")
            } else {
                format!("Push to talk: {key}")
            }
        });
        let talking = self.model.ptt_open;
        let (muted, deafened, forced) = (c.muted, c.deafened, c.force_muted);
        let mute = Button::new("call-mute")
            .label(if forced {
                "Muted by a moderator"
            } else if muted {
                "Unmute"
            } else {
                "Mute"
            })
            .selected(muted)
            .disabled(forced)
            .on_click(cx.listener(|this, _, _, cx| this.act(Model::toggle_mute, cx)));
        let deafen = Button::new("call-deafen")
            .label(if deafened { "Undeafen" } else { "Deafen" })
            .selected(deafened)
            .on_click(cx.listener(|this, _, _, cx| this.act(Model::toggle_deafen, cx)));
        let leave = Button::new("call-leave")
            .danger()
            .label("Leave")
            .on_click(cx.listener(|this, _, _, cx| this.act(Model::leave_voice, cx)));

        let s = &self.model.state;
        let me = s.my_id().to_string();
        let my_name =
            s.me.as_ref()
                .map_or(String::new(), |m| m.user.display_name.clone());
        let note = |m: bool, d: bool| match (m, d) {
            (_, true) => "deafened",
            (true, false) => "muted",
            _ => "",
        };
        let mine = person(
            format!("{my_name} (you)"),
            &me,
            c.me_speaking,
            note(muted || forced, deafened),
            None,
        );
        let others: Vec<(String, String, bool, &str)> = s
            .voice
            .iter()
            .filter(|p| Some(p.room_id) == c.room && p.user_id != me)
            .map(|p| {
                let speaking = c.speaking.contains(&p.user_id);
                let note = match note(p.muted || p.force_muted, p.deafened) {
                    "" if p.share.is_some() => "sharing",
                    n => n,
                };
                (p.user_id.clone(), p.name.clone(), speaking, note)
            })
            .collect();
        let people: Vec<Div> = others
            .into_iter()
            .map(|(id, name, speaking, note)| {
                let slider = self.slider(&id, cx);
                person(name, &id, speaking, note, Some(slider))
            })
            .collect();
        let error = self.model.call.error.clone();
        let filter = self.model.call.filter.clone();
        let devices = self.model.call.devices.clone();
        let echo_off = self.model.call.echo_off.clone();

        Some(
            v_flex()
                .debug_selector(|| "call-bar".into())
                .flex_none()
                .gap_2()
                .px_4()
                .py_2()
                .border_t_1()
                .border_color(rgb(theme::LINE_SUBTLE))
                .bg(rgb(theme::BG_SIDEBAR))
                .children(headphones)
                .child(
                    h_flex()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Voice: {room}")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .text_color(rgb(theme::FG_3))
                                .child(status),
                        )
                        .children(ptt.map(|t| {
                            div()
                                .debug_selector(|| "call-ptt".into())
                                .text_size(px(12.))
                                .text_color(rgb(if talking {
                                    theme::ACCENT_FG
                                } else {
                                    theme::FG_3
                                }))
                                .child(t)
                        }))
                        .child(div().debug_selector(|| "call-mute".into()).child(mute))
                        .child(div().debug_selector(|| "call-deafen".into()).child(deafen))
                        .child(div().debug_selector(|| "call-leave".into()).child(leave)),
                )
                .when_some_text(&error, theme::DANGER_FG, "call-error")
                .when_some_text(&devices, theme::WARN_FG, "call-devices")
                .when_some_text(&echo_off, theme::WARN_FG, "call-echo")
                .when_some_text(&filter, theme::FG_3, "call-filter")
                .child(h_flex().flex_wrap().gap_4().child(mine).children(people)),
        )
    }
}
