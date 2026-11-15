//! The web app's base components (`web/src/lib/ui/c/`), with the metrics and colours of their `<style>` blocks. Every
//! colour comes from the [`Tokens`](super::theme::Tokens) global, so a style or accent change repaints them.
//!
//! Button and IconButton fade their hover colours in `--dur-1` (`motion::hover`). Not ported yet: focus-visible
//! rings (GPUI 0.2.2 has no focus-visible), the press scale (GPUI scales only an svg), tooltips (`data-tip`), avatar
//! pictures.

use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{Animation, AnimationExt};
use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Entity, Hsla, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, StyleRefinement, Styled,
    Transformation, Window, div, percentage, px, transparent_black,
};
use gpui_component::StyledExt;
use gpui_component::input::{Input, InputState};
use gpui_component::slider::{Slider, SliderState};

use super::icons::{icon, spinner};
use super::motion;
use super::theme::{TEXT_LG, TEXT_SM, TEXT_XS, TextStepExt, tokens};

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type OnBool = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// CSS `truncate`: one line cut with an ellipsis. GPUI's own `truncate()` (no wrap) measures the text once before its
/// width is known and keeps that size, so it clips without an ellipsis; this wraps at the width and keeps one line.
pub trait Ellipsis: Styled + Sized {
    fn ellipsis(self) -> Self {
        self.overflow_hidden().text_ellipsis().line_clamp(1)
    }
}

impl<T: Styled> Ellipsis for T {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Variant {
    /// The accent fill: at most one per view.
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
    DangerGhost,
}

/// `Button.svelte`: 32 px tall (28 small), 12 px sides (8 small), 6 px gap, 1 px border, `--radius`, `--weight-ui`.
/// `loading` hides the label behind a spinner of the same width and ignores clicks.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    variant: Variant,
    small: bool,
    loading: bool,
    disabled: bool,
    icon: Option<&'static str>,
    on_click: Option<OnClick>,
    style: StyleRefinement,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Button {
            id: id.into(),
            label: label.into(),
            variant: Variant::Secondary,
            small: false,
            loading: false,
            disabled: false,
            icon: None,
            on_click: None,
            style: StyleRefinement::default(),
        }
    }
    pub fn variant(mut self, v: Variant) -> Self {
        self.variant = v;
        self
    }
    pub fn primary(self) -> Self {
        self.variant(Variant::Primary)
    }
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
    pub fn loading(mut self, on: bool) -> Self {
        self.loading = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// An icon before the label.
    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (h, on_hover) = motion::hover(self.id.clone(), window, cx);
        let t = tokens(cx);
        let none: Hsla = transparent_black();
        // (fill, border, text) at rest, then on hover.
        let (rest, hover): ((Hsla, Hsla, Hsla), (Hsla, Hsla, Hsla)) = match self.variant {
            Variant::Primary => (
                (t.accent.into(), none, t.on_accent.into()),
                (t.accent_hover.into(), none, t.on_accent.into()),
            ),
            Variant::Secondary => (
                (t.bg_element.into(), t.line.into(), t.fg.into()),
                (t.bg_hover.into(), t.line_strong.into(), t.fg.into()),
            ),
            Variant::Ghost => (
                (none, none, t.fg_2.into()),
                (t.bg_hover.into(), none, t.fg.into()),
            ),
            // color-mix(in oklab, danger-solid, fg 8%), here in sRGB.
            Variant::Danger => (
                (t.danger_solid.into(), none, t.on_danger.into()),
                (
                    mix(t.danger_solid.into(), t.fg.into(), 0.08),
                    none,
                    t.on_danger.into(),
                ),
            ),
            Variant::DangerGhost => (
                (none, none, t.danger_fg.into()),
                (t.danger_subtle.into(), none, t.danger_fg.into()),
            ),
        };
        let disabled = self.disabled && !self.loading;
        let rest = if disabled {
            let fill = if self.variant == Variant::Ghost {
                none
            } else {
                t.bg_element.into()
            };
            let line = if self.variant == Variant::Ghost {
                none
            } else {
                t.line_subtle.into()
            };
            (fill, line, t.fg_disabled.into())
        } else {
            rest
        };
        let live = !disabled && !self.loading;
        let on_click = self.on_click.filter(|_| live);
        // The 100 ms transition of fill, border and text.
        let h = if live { h } else { 0. };
        let rest = (
            mix(rest.0, hover.0, h),
            mix(rest.1, hover.1, h),
            mix(rest.2, hover.2, h),
        );
        let text = if self.loading { none } else { rest.2 };
        div()
            .id(self.id)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .h(px(if self.small { 28. } else { 32. }))
            .px(px(if self.small { 8. } else { 12. }))
            .border_1()
            .border_color(rest.1)
            .rounded(t.radius)
            .bg(rest.0)
            .text_color(text)
            .text_step(TEXT_SM)
            .font_weight(t.weight_ui)
            .whitespace_nowrap()
            .when(live, |d| d.on_hover(on_hover))
            .when_some(on_click, |d, f| d.on_click(move |e, w, cx| f(e, w, cx)))
            .refine_style(&self.style)
            .when_some(self.icon, |d, name| d.child(icon(name).text_color(text)))
            .child(self.label)
            .when(self.loading, |d| {
                d.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(spinner().text_color(rest.2).with_animation(
                            "spin",
                            Animation::new(Duration::from_millis(800)).repeat(),
                            |s, delta| {
                                s.with_transformation(Transformation::rotate(percentage(delta)))
                            },
                        )),
                )
            })
    }
}

/// `IconButton.svelte`: a 32 px square, `--radius`, the icon in `--fg-2`. `pressed` is a toggle that is on; `danger`
/// draws the icon in the danger colour.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: &'static str,
    pressed: bool,
    danger: bool,
    disabled: bool,
    turn: bool,
    on_click: Option<OnClick>,
}

impl IconButton {
    pub fn new(id: impl Into<ElementId>, icon: &'static str) -> Self {
        IconButton {
            id: id.into(),
            icon,
            pressed: false,
            danger: false,
            disabled: false,
            turn: false,
            on_click: None,
        }
    }
    /// The Settings gear: a third of a turn on hover (its six teeth line up again), `--dur-3`, ease-out. None under
    /// reduced motion.
    pub fn turn(mut self) -> Self {
        self.turn = true;
        self
    }
    pub fn pressed(mut self, on: bool) -> Self {
        self.pressed = on;
        self
    }
    pub fn danger(mut self, on: bool) -> Self {
        self.danger = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (h, on_hover) = motion::hover(self.id.clone(), window, cx);
        let turn = self.turn.then(|| {
            let m = &tokens(cx).motion;
            let dur = m.or(m.dur_3, Duration::ZERO);
            let id = ElementId::NamedChild(Box::new(self.id.clone()), "turn".into());
            motion::transition(id, dur, window, cx)
        });
        let angle = turn.as_ref().map_or(0., |t| t.0 * 120.);
        let turn = turn.map(|t| t.1);
        let t = tokens(cx);
        let fg: Hsla = if self.disabled {
            t.fg_disabled.into()
        } else if self.danger {
            t.danger_fg.into()
        } else if self.pressed {
            t.fg.into()
        } else {
            t.fg_2.into()
        };
        let hover_fg: Hsla = if self.danger {
            t.danger_fg.into()
        } else {
            t.fg.into()
        };
        let live = !self.disabled;
        let h = if live { h } else { 0. };
        let rest_bg: Hsla = if self.pressed {
            t.bg_selected.into()
        } else {
            transparent_black()
        };
        let hover_bg: Hsla = if self.pressed {
            t.bg_selected.into()
        } else {
            t.bg_hover.into()
        };
        let fg = mix(fg, hover_fg, h);
        div()
            .id(self.id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(32.))
            .rounded(t.radius)
            .text_color(fg)
            .bg(mix(rest_bg, hover_bg, h))
            .when(live, |d| {
                d.on_hover(move |on, w, cx| {
                    on_hover(on, w, cx);
                    if let Some(f) = &turn {
                        f(on, w, cx);
                    }
                })
            })
            .when_some(self.on_click.filter(|_| live), |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
            .child(
                icon(self.icon)
                    .text_color(fg)
                    .with_transformation(Transformation::rotate(gpui::Radians(angle.to_radians()))),
            )
    }
}

/// `Field.svelte`: the label (13/20, `--weight-ui`, `--fg`) over the control with a 6 px gap, then the error (12/16,
/// danger) or else the help line (12/16, `--fg-2`). `value` shows at the right of the label.
#[derive(IntoElement)]
pub struct Field {
    label: SharedString,
    value: Option<SharedString>,
    help: Option<SharedString>,
    error: Option<SharedString>,
    children: Vec<AnyElement>,
}

impl Field {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Field {
            label: label.into(),
            value: None,
            help: None,
            error: None,
            children: vec![],
        }
    }
    pub fn value(mut self, v: impl Into<SharedString>) -> Self {
        self.value = Some(v.into());
        self
    }
    pub fn help(mut self, v: impl Into<SharedString>) -> Self {
        self.help = Some(v.into()).filter(|s: &SharedString| !s.is_empty());
        self
    }
    pub fn error(mut self, v: impl Into<SharedString>) -> Self {
        self.error = Some(v.into()).filter(|s: &SharedString| !s.is_empty());
        self
    }
}

impl ParentElement for Field {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Field {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens(cx);
        let label = div()
            .text_step(TEXT_SM)
            .font_weight(t.weight_ui)
            .text_color(t.fg)
            .child(self.label);
        let head = match self.value {
            None => label,
            Some(v) => div()
                .flex()
                .items_baseline()
                .justify_between()
                .gap_4()
                .child(label)
                .child(div().text_step(TEXT_SM).text_color(t.fg_2).child(v)),
        };
        let below = match (self.error, self.help) {
            (Some(e), _) => Some(div().text_step(TEXT_XS).text_color(t.danger_fg).child(e)),
            (None, Some(h)) => Some(div().text_step(TEXT_XS).text_color(t.fg_2).child(h)),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(head)
            .children(self.children)
            .children(below)
    }
}

/// `TextInput.svelte` (the `.control` look of app.css): 32 px tall, 10 px sides, 1 px `--line` border, `--radius`,
/// `--bg-element`, 13 px text, placeholder in `--fg-3`. Focus draws the border in `--focus` (the web draws a 2 px
/// outline 2 px outside; GPUI has no outline). Hover (`--line-strong`) is not drawn: `Input` takes no hover style.
pub fn text_input(state: &Entity<InputState>, cx: &App) -> Input {
    let t = tokens(cx);
    // Styled::h, not Input::h (the multi-line height).
    Styled::h(Input::new(state), px(32.))
        .px(px(10.))
        .py_0()
        .bg(t.bg_element)
        .border_1()
        .border_color(t.line)
        .rounded(t.radius)
        .text_step(TEXT_SM)
        .text_color(t.fg)
}

/// `PasswordInput.svelte`: a text input with room for a Show toggle at its right (24 px tall, 8 px sides, 12 px
/// `--weight-ui`, `--fg-2`; pressed: `--bg-selected` and `--fg`). The state's masking is set from `shown` on each click;
/// `on_toggle` gets the new value, which the owner keeps and passes back.
#[derive(IntoElement)]
pub struct PasswordInput {
    state: Entity<InputState>,
    shown: bool,
    on_toggle: OnBool,
}

impl PasswordInput {
    pub fn new(
        state: &Entity<InputState>,
        shown: bool,
        on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        PasswordInput {
            state: state.clone(),
            shown,
            on_toggle: Rc::new(on_toggle),
        }
    }
}

impl RenderOnce for PasswordInput {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens(cx);
        let (shown, state, toggle) = (self.shown, self.state.clone(), self.on_toggle);
        let (hover, fg) = (t.bg_hover, t.fg);
        div()
            .relative()
            .child(text_input(&self.state, cx).pr(px(64.)))
            .child(
                div()
                    .id(("show", self.state.entity_id()))
                    .absolute()
                    .top(px(4.))
                    .right(px(4.))
                    .h(px(24.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .rounded(t.radius_sm)
                    .text_step(TEXT_XS)
                    .line_height(px(24.))
                    .font_weight(t.weight_ui)
                    .text_color(if shown { t.fg } else { t.fg_2 })
                    .when(shown, |d| d.bg(t.bg_selected))
                    .hover(move |s| s.bg(hover).text_color(fg))
                    .on_click(move |_, window, cx| {
                        state.update(cx, |s, cx| s.set_masked(shown, window, cx));
                        toggle(!shown, window, cx);
                    })
                    .child("Show"),
            )
    }
}

/// `Switch.svelte`: the label left, a 28x16 track right on a row at least 32 px tall, `help` under the label. Off: the
/// track is `--bg-selected` inside a 1 px `--line-strong` ring and the 12 px knob `--fg-2`; on: `--accent` and
/// `--on-accent`. Locked on (`disabled` and `checked`) stays neutral; disabled is at half opacity.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    label: SharedString,
    help: Option<SharedString>,
    checked: bool,
    disabled: bool,
    on_change: Option<OnBool>,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>, checked: bool) -> Self {
        Switch {
            id: id.into(),
            label: label.into(),
            help: None,
            checked,
            disabled: false,
            on_change: None,
        }
    }
    pub fn help(mut self, v: impl Into<SharedString>) -> Self {
        self.help = Some(v.into());
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn on_change(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens(cx);
        let on = self.checked && !self.disabled;
        let (track, knob) = if on {
            (t.accent, t.on_accent)
        } else {
            (t.bg_selected, t.fg_2)
        };
        let checked = self.checked;
        div()
            .child(
                div()
                    .id(self.id)
                    .flex()
                    .min_h(px(32.))
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .text_step(TEXT_SM)
                    .text_color(if self.disabled { t.fg_disabled } else { t.fg })
                    .when_some(self.on_change.filter(|_| !self.disabled), |d, f| {
                        d.on_click(move |_, w, cx| f(!checked, w, cx))
                    })
                    .child(self.label)
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .w(px(28.))
                            .h(px(16.))
                            .rounded(px(8.))
                            .bg(track)
                            .when(!on, |d| d.border_1().border_color(t.line_strong))
                            .when(self.disabled, |d| d.opacity(0.5))
                            .child(
                                div()
                                    .absolute()
                                    // The ring is a border here, so the knob sits 1 px further in when off.
                                    .top(px(if on { 2. } else { 1. }))
                                    .left(px(
                                        if on { 2. } else { 1. } + if checked { 12. } else { 0. }
                                    ))
                                    .size(px(12.))
                                    .rounded_full()
                                    .bg(knob),
                            ),
                    ),
            )
            .when_some(self.help, |d, h| {
                d.child(
                    div()
                        .mt(px(-4.))
                        .pr(px(44.))
                        .text_step(TEXT_XS)
                        .text_color(t.fg_2)
                        .child(h),
                )
            })
    }
}

/// `Range.svelte` through gpui-component's slider, coloured from the tokens: the filled part `--accent`, the knob
/// `--fg`. shortcut: its knob is 16 px with a ring and its bar 6 px; the web's are 14 px plain and 4 px. Draw our own
/// when the Settings screen is matched.
pub fn range(state: &Entity<SliderState>, cx: &App) -> Slider {
    let t = tokens(cx);
    Slider::new(state).bg(t.accent).text_color(t.fg)
}

/// `avatar.ts`: the slot (1 to 8) of a user's accent, or a hash of the id for any other accent.
pub fn avatar_slot(user_id: &str, accent: &str) -> usize {
    const SLOT_HEX: [&str; 8] = [
        "#ed8d7d", "#e5955d", "#99b860", "#6fc082", "#31c3b5", "#41bae4", "#6db0f4", "#e68aaf",
    ];
    let a = accent.trim().to_lowercase();
    if let Some(i) = SLOT_HEX.iter().position(|s| *s == a) {
        return i + 1;
    }
    // FNV-1a over UTF-16 code units, as JS charCodeAt.
    let h = user_id.encode_utf16().fold(0x811c9dc5u32, |h, c| {
        (h ^ c as u32).wrapping_mul(0x01000193)
    });
    (h % 8) as usize + 1
}

/// `avatar.ts` initials: the first letter of up to two words, upper case.
/// shortcut: a letter is a `char`, not a grapheme; a flag or a joined emoji as a first letter shows its first part.
pub fn initials(name: &str) -> String {
    name.split_whitespace()
        .take(2)
        .filter_map(|w| w.chars().next())
        .collect::<String>()
        .to_uppercase()
}

/// `Avatar.svelte`: initials on the user's slot colour in `--on-avatar`, `--weight-ui`; 20, 32 or 40 px (12/16, 13/20,
/// 16/24 text; one letter at 20). `speaking` draws a 2 px `--bg-sidebar` gap and a 2 px accent ring; muted or deafened
/// a danger glyph at the bottom right at 32 and 40.
#[derive(IntoElement)]
pub struct Avatar {
    user_id: SharedString,
    name: SharedString,
    accent: SharedString,
    size: f32,
    speaking: bool,
    muted: bool,
    deafened: bool,
}

impl Avatar {
    pub fn new(
        user_id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        accent: impl Into<SharedString>,
        size: u32,
    ) -> Self {
        debug_assert!(matches!(size, 20 | 32 | 40));
        Avatar {
            user_id: user_id.into(),
            name: name.into(),
            accent: accent.into(),
            size: size as f32,
            speaking: false,
            muted: false,
            deafened: false,
        }
    }
    pub fn speaking(mut self, on: bool) -> Self {
        self.speaking = on;
        self
    }
    pub fn muted(mut self, on: bool) -> Self {
        self.muted = on;
        self
    }
    pub fn deafened(mut self, on: bool) -> Self {
        self.deafened = on;
        self
    }
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens(cx);
        let s = self.size;
        let mut text = initials(&self.name);
        if text.is_empty() {
            text = "?".into();
        }
        if s == 20. {
            text = text.chars().take(1).collect();
        }
        let step = match s as u32 {
            20 => TEXT_XS,
            32 => TEXT_SM,
            _ => TEXT_LG,
        };
        let glyph = match (s as u32, self.deafened, self.muted) {
            (20, ..) => None,
            (_, true, _) => Some("headphonesOff"),
            (_, _, true) => Some("micOff"),
            _ => None,
        };
        let (gw, iw, off) = if s == 32. {
            (18., 12., -4.)
        } else {
            (20., 14., -3.)
        };
        div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(s))
            .rounded_full()
            .bg(t.avatar[avatar_slot(&self.user_id, &self.accent) - 1])
            .text_color(t.on_avatar)
            .text_step(step)
            .font_weight(t.weight_ui)
            .when(self.speaking, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-4.))
                        .left(px(-4.))
                        .size(px(s + 8.))
                        .rounded_full()
                        .border_2()
                        .border_color(t.accent)
                        .child(
                            div()
                                .size_full()
                                .rounded_full()
                                .border_2()
                                .border_color(t.bg_sidebar),
                        ),
                )
            })
            .child(text)
            .when_some(glyph, |d, g| {
                d.child(
                    div()
                        .absolute()
                        .right(px(off))
                        .bottom(px(off))
                        .size(px(gw))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(t.bg_sidebar)
                        .text_color(t.danger_fg)
                        .child(icon(g).size(px(iw)).text_color(t.danger_fg)),
                )
            })
    }
}

/// `Badge.svelte`: an unread count (`--bg-selected`, `--fg-2`) or the `@n` mention pill (`--accent`, `--on-accent`):
/// 16 px tall, at least 16 wide, 6 px sides, fully round, 12/16 `--weight-strong`; above 99 it reads `99+`. `pose` is
/// the `bump` preset while it pops in: the web scales it from 0.6, here it is drawn smaller in the middle of its own
/// full-size box, so nothing around it moves.
#[derive(IntoElement)]
pub struct Badge {
    mention: bool,
    n: i64,
    pose: Option<motion::Pose>,
}

impl Badge {
    pub fn new(mention: bool, n: i64) -> Self {
        Badge {
            mention,
            n,
            pose: None,
        }
    }
    pub fn pose(mut self, p: Option<motion::Pose>) -> Self {
        self.pose = p;
        self
    }
    /// What it reads, which is also what a bump is keyed on.
    pub fn text(mention: bool, n: i64) -> String {
        let n = if n > 99 { "99+".into() } else { n.to_string() };
        if mention { format!("@{n}") } else { n }
    }
}

impl RenderOnce for Badge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens(cx);
        let (bg, fg) = if self.mention {
            (t.accent, t.on_accent)
        } else {
            (t.bg_selected, t.fg_2)
        };
        let text = Badge::text(self.mention, self.n);
        let pill = |k: f32| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(px(16. * k))
                .min_w(px(16. * k))
                .px(px(6. * k))
                .rounded_full()
                .bg(bg)
                .text_color(fg)
                .text_size(px(12. * k))
                .line_height(px(16. * k))
                .font_weight(t.weight_strong)
                .whitespace_nowrap()
                .child(text.clone())
        };
        match self.pose {
            None => pill(1.).into_any_element(),
            Some(p) => div()
                .relative()
                .flex_none()
                .child(pill(1.).invisible())
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(p.opacity)
                        .child(pill(0.6 + 0.4 * p.t)),
                )
                .into_any_element(),
        }
    }
}

/// `Popover.svelte`'s box: 176 to 320 px wide, 4 px padding, 1 px `--line-subtle`, `--radius-lg`, `--bg-raised`,
/// 13/20 text, `--shadow-float`. Place it with `gpui::anchored()`.
pub fn popover_frame(cx: &App) -> Div {
    let t = tokens(cx);
    div()
        .min_w(px(176.))
        .max_w(px(320.))
        .p(px(4.))
        .border_1()
        .border_color(t.line_subtle)
        .rounded(t.radius_lg)
        .bg(t.bg_raised)
        .text_color(t.fg)
        .text_step(TEXT_SM)
        .shadow(t.shadow_float.clone())
}

/// `MenuItem.svelte`: a 32 px row, 8 px sides and gap, `--radius-sm`, the icon in `--fg-2`, hover `--bg-hover`;
/// `danger` in the danger colour, `disabled` in `--fg-disabled`.
#[derive(IntoElement)]
pub struct MenuItem {
    id: ElementId,
    label: SharedString,
    icon: Option<&'static str>,
    danger: bool,
    disabled: bool,
    on_select: Option<OnClick>,
}

impl MenuItem {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        MenuItem {
            id: id.into(),
            label: label.into(),
            icon: None,
            danger: false,
            disabled: false,
            on_select: None,
        }
    }
    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }
    pub fn danger(mut self, on: bool) -> Self {
        self.danger = on;
        self
    }
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    pub fn on_select(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for MenuItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens(cx);
        let (fg, icon_fg) = if self.disabled {
            (t.fg_disabled, t.fg_disabled)
        } else if self.danger {
            (t.danger_fg, t.danger_fg)
        } else {
            (t.fg, t.fg_2)
        };
        let hover = t.bg_hover;
        let live = !self.disabled;
        div()
            .id(self.id)
            .flex()
            .w_full()
            .h(px(32.))
            .items_center()
            .gap(px(8.))
            .px(px(8.))
            .rounded(t.radius_sm)
            .text_color(fg)
            .when(live, |d| d.hover(move |s| s.bg(hover)))
            .when_some(self.on_select.filter(|_| live), |d, f| {
                d.on_click(move |e, w, cx| f(e, w, cx))
            })
            .when_some(self.icon, |d, n| d.child(icon(n).text_color(icon_fg)))
            .child(div().truncate().child(self.label))
    }
}

/// `Dialog.svelte`: over a backdrop of `--bg-app` at 72 %, a 440 px box (1 px `--line-subtle`, `--radius-lg`,
/// `--bg-raised`, `--shadow-float`): the title (16/24, `--weight-strong`), the body (13/20, `--fg-2`), an optional
/// footer (right-aligned, 8 px gap, a `--line-subtle` rule above) and a Close button at the top right. Put it last in
/// a `relative` parent the size of the window.
/// shortcut: centred, not the web's `margin: auto` in a scrolling top layer; no focus trap or Esc yet.
#[derive(IntoElement)]
pub struct Dialog {
    title: SharedString,
    body: Vec<AnyElement>,
    footer: Option<AnyElement>,
    on_close: Option<OnClick>,
}

impl Dialog {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Dialog {
            title: title.into(),
            body: vec![],
            footer: None,
            on_close: None,
        }
    }
    pub fn footer(mut self, f: impl IntoElement) -> Self {
        self.footer = Some(f.into_any_element());
        self
    }
    pub fn on_close(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl RenderOnce for Dialog {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens(cx);
        let mut close = IconButton::new("dialog-close", "x");
        if let Some(f) = self.on_close.clone() {
            close = close.on_click(move |e, w, cx| f(e, w, cx));
        }
        let mut backdrop: Hsla = t.bg_app.into();
        backdrop.a = 0.72;
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(backdrop)
            .child(
                div()
                    .id("dialog")
                    .relative()
                    .flex()
                    .flex_col()
                    .w(px(440.))
                    .border_1()
                    .border_color(t.line_subtle)
                    .rounded(t.radius_lg)
                    .bg(t.bg_raised)
                    .text_color(t.fg)
                    .shadow(t.shadow_float.clone())
                    .child(
                        div()
                            .pl(px(20.))
                            .pr(px(56.))
                            .pt(px(16.))
                            .pb(px(8.))
                            .text_step(TEXT_LG)
                            .font_weight(t.weight_strong)
                            .child(self.title),
                    )
                    .child(
                        div()
                            .px(px(20.))
                            .pt(px(8.))
                            .pb(px(20.))
                            .text_step(TEXT_SM)
                            .text_color(t.fg_2)
                            .children(self.body),
                    )
                    .when_some(self.footer, |d, f| {
                        d.child(
                            div()
                                .flex()
                                .justify_end()
                                .gap_2()
                                .border_t_1()
                                .border_color(t.line_subtle)
                                .px(px(20.))
                                .py(px(12.))
                                .child(f),
                        )
                    })
                    .child(div().absolute().top(px(12.)).right(px(12.)).child(close)),
            )
    }
}

/// `a` moved `t` of the way to `b`, in sRGB (CSS transitions mix legacy colours there too).
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    let l = |x: f32, y: f32| x + (y - x) * t;
    gpui::Rgba {
        r: l(a.r, b.r),
        g: l(a.g, b.g),
        b: l(a.b, b.b),
        a: l(a.a, b.a),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_slot_and_initials_follow_avatar_ts() {
        assert_eq!(avatar_slot("x", "#31C3B5 "), 5);
        // FNV-1a("") = 0x811c9dc5, % 8 = 5, so slot 6; and a fixed id stays put.
        assert_eq!(avatar_slot("", "#7c8cff"), 6);
        assert_eq!(avatar_slot("abc", "#7c8cff"), avatar_slot("abc", "nope"));
        assert_eq!(initials("bartholomew  featherstonehaugh smith"), "BF");
        assert_eq!(initials("  "), "");
    }
}
