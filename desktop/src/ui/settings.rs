//! The Settings view, from the sidebar: the noise filter, voice activity or push to talk and its key, the
//! microphone and speakers, the Opus bitrate, and start with the computer. Saved as JSON on each change.

use gpui::{
    AppContext, Context, Div, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div, px,
    rgb,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::slider::{Slider, SliderEvent, SliderState};
use gpui_component::{Selectable, h_flex, v_flex};

use super::sign_in::TextLine;
use super::{Vesper, theme};
use crate::settings::{self, Denoise, Settings};
use crate::voice::ptt::{self, KeyMsg};

pub const APPLIES_NEXT_CALL: &str =
    "The noise filter, devices and bitrate apply from your next call.";
pub const CAPTURING: &str = "Press the key to use. Esc cancels.";

fn section(title: &'static str) -> Div {
    div()
        .pt_4()
        .pb_1()
        .font_weight(FontWeight::SEMIBOLD)
        .child(title)
}

fn help(text: &str) -> Div {
    div()
        .text_size(px(12.))
        .text_color(rgb(theme::FG_3))
        .child(text.to_string())
}

impl Vesper {
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_open = true;
        self.capturing_key = false;
        if self.settings_file.is_some() {
            // cpal can take a moment to list devices; not on the UI thread.
            let list = cx.background_spawn(async { crate::voice::device_names() });
            cx.spawn_in(window, async move |this, cx| {
                let names = list.await;
                let _ = this.update(cx, |v, cx| {
                    v.devices = names;
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    /// Changes the settings: into the model (push to talk applies at once), to the key thread, and to the file.
    pub(super) fn change_settings(
        &mut self,
        f: impl FnOnce(&mut Settings),
        cx: &mut Context<Self>,
    ) {
        let mut s = self.model.settings.clone();
        f(&mut s);
        let autostart = (s.start_with_computer != self.model.settings.start_with_computer)
            .then_some(s.start_with_computer);
        self.settings_error.clear();
        let cmds = self.model.set_settings(s);
        self.run(cmds);
        self.watch_key();
        if let Some(path) = self.settings_file.clone() {
            if let Some(on) = autostart
                && let Err(e) = settings::set_start_with_computer(on)
            {
                self.model.settings.start_with_computer = !on;
                self.settings_error = e;
            }
            if let Err(e) = self.model.settings.save(&path) {
                self.settings_error = format!("Could not save the settings: {e}");
            }
        }
        cx.notify();
    }

    /// Tells the key thread which key to watch, if push to talk is on.
    pub(super) fn watch_key(&self) {
        let s = &self.model.settings;
        let key = s.push_to_talk.then(|| ptt::parse_key(&s.ptt_key)).flatten();
        if let Some(tx) = &self.ptt_tx {
            let _ = tx.send(KeyMsg::Watch(key));
        }
    }

    /// A key pressed while choosing the push-to-talk key.
    pub(super) fn capture_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        if !self.capturing_key || e.is_held {
            return;
        }
        cx.stop_propagation();
        let k = &e.keystroke;
        if k.key == "escape" {
            (self.capturing_key, self.key_problem) = (false, "");
        } else if k.modifiers.control || k.modifiers.alt || k.modifiers.platform {
            self.key_problem = "Pick a key that is not Ctrl, Alt, Shift or Meta.";
        } else {
            match ptt::key_from_window(&k.key) {
                Ok(name) => {
                    (self.capturing_key, self.key_problem) = (false, "");
                    self.change_settings(|s| s.ptt_key = name, cx);
                }
                Err(why) => self.key_problem = why,
            }
        }
        cx.notify();
    }

    fn choice(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        selected: bool,
        cx: &mut Context<Self>,
        f: impl Fn(&mut Settings) + 'static,
    ) -> impl IntoElement {
        let id: SharedString = id.into();
        let selector = id.to_string();
        div().debug_selector(move || selector).child(
            Button::new(id)
                .label(label)
                .selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| this.change_settings(&f, cx))),
        )
    }

    /// A device list: the system default, then each device by name. A saved name that is not there shows as missing.
    fn devices_list(
        &self,
        prefix: &'static str,
        names: &[String],
        chosen: Option<&str>,
        cx: &mut Context<Self>,
        set: fn(&mut Settings, Option<String>),
    ) -> Div {
        let mut list = v_flex().gap_1().child(self.choice(
            prefix,
            "System default",
            chosen.is_none(),
            cx,
            move |s| set(s, None),
        ));
        for (i, name) in names.iter().enumerate() {
            let n = name.clone();
            list = list.child(self.choice(
                format!("{prefix}-{i}"),
                name.clone(),
                chosen == Some(name.as_str()),
                cx,
                move |s| set(s, Some(n.clone())),
            ));
        }
        if let Some(c) = chosen.filter(|c| !names.iter().any(|n| n == c)) {
            list = list.child(help(&format!(
                "{c} is not connected, so the default is used."
            )));
        }
        list
    }

    fn bitrate_slider(&mut self, cx: &mut Context<Self>) -> Div {
        if self.bitrate.is_none() {
            let start = self.model.settings.bitrate_kbps as f32;
            let state = cx.new(|_| {
                SliderState::new()
                    .min(32.)
                    .max(510.)
                    .step(1.)
                    .default_value(start)
            });
            let sub = cx.subscribe(&state, |this, _, e: &SliderEvent, cx| {
                let SliderEvent::Change(v) = e;
                let kbps = v.start().round() as u32;
                if kbps != this.model.settings.bitrate_kbps {
                    this.change_settings(|s| s.bitrate_kbps = kbps, cx);
                }
            });
            self.bitrate = Some((state, sub));
        }
        let state = &self.bitrate.as_ref().expect("made above").0;
        h_flex()
            .debug_selector(|| "settings-bitrate".into())
            .gap_3()
            .child(div().w(px(240.)).child(Slider::new(state)))
            .child(format!("{} kbps", self.model.settings.bitrate_kbps))
    }

    pub(super) fn render_settings(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let s = self.model.settings.clone();
        let filter_help = match s.denoise {
            Denoise::DeepFilter => {
                "Strongest. Runs on your computer. Falls back to the noise gate if it cannot keep up."
            }
            Denoise::Gate => {
                "Lighter. Mutes your mic between words, when it is quieter than -50 dB."
            }
            Denoise::Off => "No filtering.",
        };
        let key = if self.capturing_key {
            "Press a key...".to_string()
        } else {
            format!("Key: {}", ptt::key_label(&s.ptt_key))
        };
        let ptt_settings = s.push_to_talk.then(|| {
            v_flex()
                .gap_1()
                .child(
                    h_flex().gap_2().child(
                        div().debug_selector(|| "settings-ptt-key".into()).child(
                            Button::new("settings-ptt-key")
                                .label(key)
                                .selected(self.capturing_key)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    (this.capturing_key, this.key_problem) = (true, "");
                                    window.focus(&this.key_focus);
                                    cx.notify();
                                })),
                        ),
                    ),
                )
                .when_some_text(
                    if self.capturing_key { CAPTURING } else { "" },
                    theme::FG_2,
                    "settings-capturing",
                )
                .when_some_text(self.key_problem, theme::WARN_FG, "settings-key-problem")
                .child(help(&format!(
                    "Your mic sends while you hold the key, and for {} ms after you let go.",
                    ptt::RELEASE_DELAY.as_millis()
                )))
                .when_some_text(
                    self.ptt_note.unwrap_or(""),
                    theme::WARN_FG,
                    "settings-ptt-note",
                )
        });
        let (inputs, outputs) = self.devices.clone();
        let mic = self.devices_list(
            "settings-mic",
            &inputs,
            s.input_device.as_deref(),
            cx,
            |s, n| s.input_device = n,
        );
        let speakers = self.devices_list(
            "settings-speakers",
            &outputs,
            s.output_device.as_deref(),
            cx,
            |s, n| s.output_device = n,
        );
        let bitrate = self.bitrate_slider(cx);
        let error = self.settings_error.clone();

        v_flex()
            .id("settings")
            .debug_selector(|| "settings".into())
            .track_focus(&self.key_focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.capture_key(e, cx)))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_6()
            .py_4()
            .gap_1()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(20.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    )
                    .child(
                        div().debug_selector(|| "settings-close".into()).child(
                            Button::new("settings-close")
                                .ghost()
                                .label("Close")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    (this.settings_open, this.capturing_key) = (false, false);
                                    this.composer.update(cx, |i, cx| i.focus(window, cx));
                                    cx.notify();
                                })),
                        ),
                    ),
            )
            .child(help(APPLIES_NEXT_CALL))
            .when_some_text(&error, theme::DANGER_FG, "settings-error")
            .child(section("Noise filter"))
            .child(
                h_flex()
                    .gap_2()
                    .child(self.choice(
                        "settings-deepfilter",
                        "DeepFilterNet",
                        s.denoise == Denoise::DeepFilter,
                        cx,
                        |s| s.denoise = Denoise::DeepFilter,
                    ))
                    .child(self.choice(
                        "settings-gate",
                        "Noise gate",
                        s.denoise == Denoise::Gate,
                        cx,
                        |s| s.denoise = Denoise::Gate,
                    ))
                    .child(self.choice(
                        "settings-off",
                        "Off",
                        s.denoise == Denoise::Off,
                        cx,
                        |s| s.denoise = Denoise::Off,
                    )),
            )
            .child(help(filter_help))
            .child(section("Input mode"))
            .child(
                h_flex()
                    .gap_2()
                    .child(self.choice(
                        "settings-voice-activity",
                        "Voice activity",
                        !s.push_to_talk,
                        cx,
                        |s| s.push_to_talk = false,
                    ))
                    .child(
                        self.choice("settings-ptt", "Push to talk", s.push_to_talk, cx, |s| {
                            s.push_to_talk = true
                        }),
                    ),
            )
            .children(ptt_settings)
            .child(section("Microphone"))
            .child(mic)
            .child(section("Speakers"))
            .child(speakers)
            .child(section("Voice quality"))
            .child(bitrate)
            .child(help(
                "64 kbps is plenty for voice. More uses more of your upload.",
            ))
            .child(section("Start with the computer"))
            .child(
                h_flex()
                    .gap_2()
                    .child(self.choice(
                        "settings-autostart-on",
                        "On",
                        s.start_with_computer,
                        cx,
                        |s| s.start_with_computer = true,
                    ))
                    .child(self.choice(
                        "settings-autostart-off",
                        "Off",
                        !s.start_with_computer,
                        cx,
                        |s| s.start_with_computer = false,
                    )),
            )
    }
}
