//! Sharing my screen from the call bar, as the web's `CallBar.svelte`, `ShareCard.svelte` and `ShareDialog.svelte`:
//! the Share screen button (group rooms only), its dialog (frame rate, bitrate, what to keep when the connection
//! struggles), and while sharing the card with the figures sent, the settings and Stop.
//!
//! On Windows and X11 the dialog also lists the screens and windows to share (the browser's own picker does that on
//! the web, the system's on Wayland). The last choice is kept until the app quits.
//!
//! Not here: the web's upload measurement and "Check your speed" link (the bitrate starts at the web's unmeasured
//! suggestion, 8 Mbps), the Firefox ceiling note, and the card's live preview of the screen.

use gpui::prelude::FluentBuilder;
use gpui::{
    AppContext, Context, ElementId, Entity, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Subscription, div, px,
};
use gpui_component::slider::{SliderEvent, SliderState};
use vesper_protocol::RoomKind;

use super::Vesper;
use super::c::{self, Ellipsis, IconButton};
use super::theme::{TEXT_XS, TextStepExt, tokens};
use crate::model::{Model, Phase};
use crate::settings::{MAX_KBPS, MIN_KBPS, SUGGEST_KBPS};
use crate::voice::capture::{self, Choice, Source};

/// The open share dialog: the choices so far. `edit`: the running share's settings, else a new share.
pub struct Dialog {
    edit: bool,
    fps: u8,
    sharp: bool,
    kbps: Entity<SliderState>,
    /// The screens and windows to choose from (none under Wayland, where the system asks), and the one chosen.
    pub(super) sources: Vec<Choice>,
    pub(super) pick: Option<Source>,
    _sub: Subscription,
}

/// The line chosen when the dialog opens: the last choice while it is still there, else the first (the primary
/// monitor).
pub fn first_pick(sources: &[Choice], last: Option<Source>) -> Option<Source> {
    last.filter(|l| sources.iter().any(|c| c.source == *l))
        .or(sources.first().map(|c| c.source))
}

/// Size, frame rate and bitrate, as a tile's hover and the share card show them (`figures` in `line.ts`).
pub fn figures(w: u32, h: u32, fps: f32, kbps: f32) -> String {
    format!(
        "{w}x{h} · {} fps · {} Mbps",
        fps.round(),
        (kbps / 100.).round() / 10.
    )
}

/// `shareLine` of `web/src/lib/share/line.ts`.
pub fn share_line(sent: Option<(u32, u32, f32, f32)>, watchers: u32) -> String {
    let rest = format!("{watchers} watching");
    match sent {
        // Nobody watching: the encoder is off, so there are no figures and nothing is starting.
        _ if watchers == 0 => format!("Sharing · {rest}"),
        Some((w, h, fps, kbps)) if w > 0 => format!("{} · {rest}", figures(w, h, fps, kbps)),
        _ => format!("Starting · {rest}"),
    }
}

impl Vesper {
    /// Opens the dialog: for a new share with the last choice, or for the running one with its settings.
    pub(super) fn open_share(&mut self, cx: &mut Context<Self>) {
        let s = &self.model.settings;
        let (edit, fps, kbps, sharp) = match self.model.call.mine {
            Some(m) => (true, m.fps, m.kbps, m.sharp),
            None => (
                false,
                if s.share_fps == 30 { 30 } else { 60 },
                s.share_kbps.unwrap_or(SUGGEST_KBPS),
                s.share_sharp,
            ),
        };
        let kbps = cx.new(|_| {
            SliderState::new()
                .max(MAX_KBPS as f32) // before min: min clamps against the default max of 100
                .min(MIN_KBPS as f32)
                .step(500.)
                .default_value(kbps as f32)
        });
        let sub = cx.subscribe(&kbps, |_, _, _: &SliderEvent, cx| cx.notify());
        let sources = if edit { vec![] } else { capture::sources() };
        let pick = first_pick(&sources, self.share_source);
        self.send = Some(Dialog {
            edit,
            fps,
            sharp,
            kbps,
            sources,
            pick,
            _sub: sub,
        });
        cx.notify();
    }

    pub(super) fn close_share(&mut self, cx: &mut Context<Self>) {
        self.send = None;
        self.page.select = None;
        cx.notify();
    }

    /// Start sharing, or Save: the choice is kept for next time, as the web's `saveShareSettings`.
    pub(super) fn go_share(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.send.take() else { return };
        self.page.select = None;
        let kbps = (d.kbps.read(cx).value().start().round() as u32).clamp(MIN_KBPS, MAX_KBPS);
        let (fps, sharp, pick) = (d.fps, d.sharp, d.pick);
        self.share_source = pick.or(self.share_source);
        self.change_settings(
            |s| {
                (s.share_fps, s.share_kbps, s.share_sharp) = (u32::from(fps), Some(kbps), sharp);
            },
            cx,
        );
        self.act(|m| m.share(fps, kbps, sharp, pick), cx);
    }

    /// The call bar's share control: the button, or my running share's card.
    pub(super) fn share_control(&mut self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let m = &self.model;
        let c = &m.call;
        let dm = c
            .room
            .and_then(|id| m.state.find_room(id))
            .is_none_or(|r| r.kind == RoomKind::Dm);
        if dm || c.phase != Phase::Joined {
            return None;
        }
        let Some(mine) = c.mine else {
            return Some(
                div()
                    .debug_selector(|| "call-share".into())
                    .child(
                        IconButton::new("call-share", "monitorUp")
                            .on_click(cx.listener(|this, _, _, cx| this.open_share(cx))),
                    )
                    .into_any_element(),
            );
        };
        let t = tokens(cx);
        Some(
            div()
                .debug_selector(|| "share-card".into())
                .flex()
                .min_w_0()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .debug_selector(|| "share-line".into())
                        // Its own width: a shrinking line made the bar squeeze the room's name away.
                        .flex_none()
                        .whitespace_nowrap()
                        .text_step(TEXT_XS)
                        .text_color(t.fg_2)
                        .child(share_line(mine.figures, mine.watchers)),
                )
                .child(
                    div().debug_selector(|| "share-settings".into()).child(
                        IconButton::new("share-settings", "settings")
                            .on_click(cx.listener(|this, _, _, cx| this.open_share(cx))),
                    ),
                )
                .child(
                    div().debug_selector(|| "share-stop".into()).child(
                        c::Button::new("share-stop", "Stop")
                            .variant(c::Variant::DangerGhost)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.act(Model::stop_share, cx)),
                            ),
                    ),
                )
                .into_any_element(),
        )
    }

    /// The share dialog, over the whole window.
    pub(super) fn share_dialog(&mut self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let d = self.send.as_ref()?;
        let (edit, fps, sharp, slider) = (d.edit, d.fps, d.sharp, d.kbps.clone());
        let (sources, pick) = (d.sources.clone(), d.pick);
        let kbps = slider.read(cx).value().start();
        let mbps: SharedString = format!("{} Mbps", (kbps / 100.).round() / 10.).into();
        let fps_select = self.select(
            "share-fps",
            vec!["30 fps".into(), "60 fps".into()],
            Some(usize::from(fps == 60)),
            400.,
            cx,
            |this, i, _, _| {
                if let Some(d) = this.send.as_mut() {
                    d.fps = if i == 0 { 30 } else { 60 };
                }
            },
        );
        let prefer = self.select(
            "share-prefer",
            vec!["Keep text sharp".into(), "Keep motion smooth".into()],
            Some(usize::from(!sharp)),
            400.,
            cx,
            |this, i, _, _| {
                if let Some(d) = this.send.as_mut() {
                    d.sharp = i == 0;
                }
            },
        );
        let picker = (!sources.is_empty()).then(|| {
            let rows = sources
                .iter()
                .enumerate()
                .map(|(i, choice)| {
                    let source = choice.source;
                    c::radio_row(
                        ElementId::Name(format!("share-source-{i}").into()),
                        pick == Some(source),
                        cx,
                    )
                    .h(px(36.))
                    .debug_selector(move || format!("share-source-{i}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(d) = this.send.as_mut() {
                            d.pick = Some(source);
                        }
                        cx.notify();
                    }))
                    .child(div().min_w_0().ellipsis().child(choice.name.clone()))
                })
                .collect();
            c::Field::new("What to share").child(
                c::radio_list(rows, cx)
                    .debug_selector(|| "share-sources".into())
                    .max_w_full()
                    .id("share-sources")
                    .max_h(px(216.))
                    .overflow_y_scroll(),
            )
        });
        let t = tokens(cx);
        let close = cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.close_share(cx));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .children(picker)
            .child(c::Field::new("Frame rate").child(fps_select))
            .child(
                c::Field::new("Bitrate").value(mbps).child(
                    div()
                        .debug_selector(|| "share-bitrate".into())
                        .child(c::range(&slider, cx)),
                ),
            )
            .child(c::Field::new("When the connection struggles").child(prefer))
            .when(!edit && sources.is_empty(), |d| {
                d.child(
                    div()
                        .text_step(TEXT_XS)
                        .text_color(t.fg_3)
                        .child("On Wayland your system asks which screen or window to share."),
                )
            });
        Some(
            div()
                .absolute()
                .inset_0()
                .track_focus(&self.dialog_focus)
                .key_context(super::DIALOG)
                .debug_selector(|| "share-dialog".into())
                .child(
                    c::Dialog::new(if edit {
                        "Share settings"
                    } else {
                        "Share your screen"
                    })
                    .child(body)
                    .footer(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                c::Button::new("share-cancel", "Cancel")
                                    .variant(c::Variant::Ghost)
                                    .on_click(cx.listener(|this, _, _, cx| this.close_share(cx))),
                            )
                            .child(
                                div().debug_selector(|| "share-go".into()).child(
                                    c::Button::new(
                                        "share-go",
                                        if edit { "Save" } else { "Start sharing" },
                                    )
                                    .primary()
                                    .on_click(cx.listener(|this, _, _, cx| this.go_share(cx))),
                                ),
                            ),
                    )
                    .on_close(close),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{first_pick, share_line};
    use crate::voice::capture::{Choice, Source};

    #[test]
    fn the_share_line_reads_like_the_web() {
        assert_eq!(share_line(None, 0), "Sharing · 0 watching");
        assert_eq!(
            share_line(Some((1920, 1080, 30., 7_960.)), 0),
            "Sharing · 0 watching"
        );
        assert_eq!(share_line(None, 1), "Starting · 1 watching");
        assert_eq!(
            share_line(Some((1920, 1080, 29.6, 7_960.)), 2),
            "1920x1080 · 30 fps · 8 Mbps · 2 watching"
        );
        assert_eq!(
            share_line(Some((1280, 720, 60., 2_440.)), 1),
            "1280x720 · 60 fps · 2.4 Mbps · 1 watching"
        );
    }

    #[test]
    fn the_dialog_starts_on_the_last_choice_while_it_is_there() {
        let list = |s: &[Source]| -> Vec<Choice> {
            s.iter()
                .map(|&source| Choice {
                    source,
                    name: String::new(),
                })
                .collect()
        };
        let (screen, window) = (Source::Monitor(1), Source::Window(42));
        let sources = list(&[screen, window]);
        assert_eq!(
            first_pick(&sources, None),
            Some(screen),
            "the first, the primary"
        );
        assert_eq!(first_pick(&sources, Some(window)), Some(window));
        assert_eq!(
            first_pick(&sources, Some(Source::Window(7))),
            Some(screen),
            "a closed window is gone"
        );
        assert_eq!(first_pick(&[], Some(window)), None, "Wayland: no list");
    }
}
