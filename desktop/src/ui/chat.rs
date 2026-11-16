//! The chat parts of the room view, after `Timeline.svelte`, `MessageItem.svelte`, `Attachments.svelte`,
//! `Composer.svelte`, `UploadTray.svelte` and `PinsPopover.svelte`: message rows (Markdown, replies, files, reactions,
//! the toolbar on hover with its menu, editing and the delete dialog), day rows and the New row, the pins list, and the
//! composer's parts (the line for typing and errors, the reply bar, the @ list, the upload tray). Names, bodies and file
//! names come from other people: drawn as text only.
//!
//! Motion (`motion.ts` presets and the CSS transitions): a new message rises in (`rise`, three at most, none on a room
//! open or an older page), the list swaps in on a room change (`swap`), a jumped-to message flashes, a reaction bumps,
//! a row's highlight fades in over `--dur-2` and out over `--dur-3`, the New row fades then folds 4 s after the room
//! is read, the menus and the @ list fade in (`pop`; GPUI cannot scale a box, so only the opacity moves) and the
//! typing dots pulse.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Local;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ClickEvent, ClipboardEntry, ClipboardItem, Context, Corner, Div, ElementId,
    Focusable, Hsla, ImageFormat, ImageSource, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, RenderImage, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    StyledText, Window, anchored, deferred, div, img, linear_color_stop, linear_gradient, px,
    transparent_black,
};
use gpui_component::input::{Input, RopeExt};
use vesper_protocol::{Attachment, AttachmentKind, Message, UserInfo};

use super::c::{self, Avatar, Ellipsis, MenuItem, mix, popover_frame};
use super::icons::icon;
use super::motion::{self, POP, RISE};
use super::select::{SelText, Selection};
use super::theme::{TEXT_SM, TEXT_XS, TextStepExt, Tokens, tokens};
use super::{
    CancelEdit, ComposerDown, ComposerEscape, ComposerTab, ComposerUp, EDIT_BOX, NewLine,
    PasteFile, SaveEdit, Vesper, md,
};
use crate::files::{self, IMAGE_MAX, Picture};
use crate::markdown::{parse, plain};
use crate::model::EMOJIS;
use crate::net::Cmd;
use crate::timeline::{
    BOX, Locale, RowHead, arrivals, clock, file_size, fit, is_long, mention_query, mentionable,
    type_label, typing_line,
};
use crate::uploads::UpState;

/// Decoded pictures kept for the timeline; the oldest goes first.
pub const IMAGE_CACHE: usize = 64;
pub(super) const IMAGE_FAILED: &str = "The image did not load.";
/// The New row goes this long after the room is read (`FADE_AFTER_MS`).
const FADE_AFTER: Duration = Duration::from_millis(4000);
/// The server's limit of pins per room.
const MAX_PINS: usize = 50;

type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// An inline picture's state.
pub enum Img {
    Loading,
    Failed,
    Ready(Arc<RenderImage>),
}

/// The timeline's own state: what moves, the New row, and what is open.
#[derive(Default)]
pub struct Tl {
    /// Rooms whose first page came (an empty room too).
    pub loaded: HashSet<i64>,
    /// The room drawn last.
    room: Option<i64>,
    /// The newest message id seen in the shown room; None right after a room change, so history is no arrival.
    last_id: Option<i64>,
    /// Messages rising in, since when.
    rose: HashMap<i64, Instant>,
    /// The `swap` of a room change: when, and whether it waits for the new room's first page.
    swap: Option<Instant>,
    swap_due: bool,
    /// A jumped-to message's flash.
    pub flash: Option<(i64, Instant)>,
    /// Each message's reactions (emoji, count) as last seen, and the bumps running by (message, emoji): when, and
    /// whether the whole pill came (else only its count changed).
    reacts: HashMap<i64, Vec<(String, usize)>>,
    bumps: HashMap<(i64, String), (Instant, bool)>,
    /// The New row: its room and the read marker when the room opened, and when the room was read since.
    read_at: Option<(i64, i64)>,
    read_done: Option<Instant>,
    /// Long messages opened with Show more.
    expanded: HashSet<i64>,
    /// The message whose More menu is open.
    pub menu: Option<i64>,
    /// A code block's Copied: message, block, since.
    copied: Option<(i64, usize, Instant)>,
    /// The formatting help is open.
    pub help: bool,
    /// When each popup (menu, picker, @ list, typing mark) showed, by name: this frame's and the last frame's.
    pops: HashMap<String, Instant>,
    pops_last: HashMap<String, Instant>,
    /// Something is moving: draw the next frame.
    moving: bool,
}

impl Tl {
    /// Since when the popup `name` shows (now, if it was not there last frame).
    fn shown_since(&mut self, name: String, now: Instant) -> Instant {
        let at = self.pops_last.get(&name).copied().unwrap_or(now);
        self.pops.insert(name, at);
        at
    }

    /// The `pop` pose of a popup shown since `at`: its opacity, 1 once done.
    fn pop(&mut self, m: &super::theme::Motion, at: Instant, now: Instant) -> f32 {
        match motion::pose(m, POP, false, at, now) {
            Some(p) => {
                self.moving = true;
                p.opacity
            }
            None => 1.,
        }
    }
}

#[cfg(test)]
impl Tl {
    pub fn rising(&self) -> Vec<i64> {
        let mut v: Vec<i64> = self.rose.keys().copied().collect();
        v.sort();
        v
    }
    pub fn flashing(&self) -> Option<i64> {
        self.flash.map(|f| f.0)
    }
    pub fn bumped(&self) -> Vec<(i64, String, bool)> {
        self.bumps
            .iter()
            .map(|(k, v)| (k.0, k.1.clone(), v.1))
            .collect()
    }
    pub fn read_done(&self) -> bool {
        self.read_done.is_some()
    }
}

/// The row's text on `--bg-app`: what a translucent fill looks like there (for the long message's fade).
fn over_app(t: &Tokens, fill: Hsla) -> Hsla {
    mix(t.bg_app.into(), Hsla { a: 1., ..fill }, fill.a)
}

/// A `.tb` of the message toolbar: 24 px, `--radius-sm`, the icon in `--fg-2`, hover `--bg-hover` and `--fg`;
/// `pressed` (its menu is open) `--bg-selected`.
#[derive(IntoElement)]
struct Tool {
    id: SharedString,
    icon: &'static str,
    pressed: bool,
    on_click: OnClick,
}

impl RenderOnce for Tool {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = tokens(cx);
        let (bg, fg): (Hsla, Hsla) = if self.pressed {
            (t.bg_selected.into(), t.fg.into())
        } else {
            (transparent_black(), t.fg_2.into())
        };
        let (hover_bg, hover_fg) = (t.bg_hover, t.fg);
        let f = self.on_click;
        let sel = self.id.clone();
        div()
            .id(ElementId::Name(self.id))
            .debug_selector(move || sel.to_string())
            .group("tb")
            .flex()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded(t.radius_sm)
            .bg(bg)
            .when(!self.pressed, |d| d.hover(move |s| s.bg(hover_bg)))
            .on_click(move |e, w, cx| f(e, w, cx))
            .child(
                icon(self.icon)
                    .text_color(fg)
                    .group_hover("tb", move |s| s.text_color(hover_fg)),
            )
    }
}

impl Vesper {
    /// Before the list draws: the New row's place, the room change, arrivals and reaction bumps. `heads` must be made
    /// already; it returns the New row's read marker for them.
    pub(super) fn sync_timeline(&mut self, now: Instant, calm: bool, cx: &mut Context<Self>) {
        let m = &self.model;
        let room = m.active;
        let tl = &mut self.tl;
        std::mem::swap(&mut tl.pops, &mut tl.pops_last);
        tl.pops.clear();
        tl.moving = false;
        if room != tl.room {
            tl.swap_due = tl.room.is_some();
            tl.room = room;
            (tl.last_id, tl.swap, tl.flash, tl.menu, tl.read_done) = (None, None, None, None, None);
            tl.rose.clear();
            tl.reacts.clear();
            tl.bumps.clear();
            tl.expanded.clear();
            tl.read_at = room.map(|r| (r, m.state.unread.get(&r).map_or(0, |u| u.last_read_id)));
        }
        let Some(room) = room else { return };
        let msgs = m.log().map_or(&[][..], |l| &l.messages[..]);
        let loaded = tl.loaded.contains(&room);
        if tl.swap_due && loaded {
            tl.swap_due = false;
            tl.swap = (!calm && !msgs.is_empty()).then_some(now);
        }
        if loaded {
            let ids: Vec<i64> = msgs.iter().map(|x| x.id).collect();
            if !calm {
                for id in arrivals(tl.last_id, &ids) {
                    tl.rose.insert(id, now);
                }
            }
            tl.last_id = Some(ids.last().copied().unwrap_or(0));
        }
        for x in msgs {
            let seen = tl.reacts.get(&x.id);
            let same = seen.is_some_and(|s| {
                s.len() == x.reactions.len()
                    && s.iter()
                        .zip(&x.reactions)
                        .all(|(a, b)| a.0 == b.emoji && a.1 == b.user_ids.len())
            });
            if same {
                continue;
            }
            if let Some(prev) = seen.filter(|_| !calm) {
                for r in &x.reactions {
                    let whole = match prev.iter().find(|p| p.0 == r.emoji) {
                        None => true,
                        Some(p) if p.1 != r.user_ids.len() => false,
                        Some(_) => continue,
                    };
                    tl.bumps.insert((x.id, r.emoji.clone()), (now, whole));
                }
            }
            let now_r = x
                .reactions
                .iter()
                .map(|r| (r.emoji.clone(), r.user_ids.len()))
                .collect();
            tl.reacts.insert(x.id, now_r);
        }
        let keep = Duration::from_secs(1);
        tl.rose.retain(|_, at| now.duration_since(*at) < keep);
        tl.bumps.retain(|_, (at, _)| now.duration_since(*at) < keep);
        // Read since it opened: the New row goes FADE_AFTER later, so a room read at once still shows it for a while.
        if let (Some((r, at)), None, Some(last)) = (tl.read_at, tl.read_done, msgs.last()) {
            let read = m.state.unread.get(&r).map_or(0, |u| u.last_read_id);
            if r == room && read >= last.id && read > at {
                tl.read_done = Some(now);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(FADE_AFTER).await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
        }
    }

    /// The read marker the New row goes after, when its first unread message is in the loaded page (`newFrom`).
    pub(super) fn new_row_after(&self, cx: &App) -> Option<i64> {
        let (room, at) = self.tl.read_at?;
        let m = &self.model;
        let log = m.log().filter(|_| m.active == Some(room))?;
        let first = log.messages.first()?;
        if log.more_before && first.id > at + 1 {
            return None;
        }
        // Folded away: gone from the rows for good.
        let mo = &tokens(cx).motion;
        let gone = self
            .tl
            .read_done
            .is_some_and(|d| d.elapsed() > FADE_AFTER + mo.or(mo.dur_3, Duration::ZERO) * 2);
        (!gone).then_some(at)
    }

    /// The `swap` pose of the list after a room change: (offset in px, opacity).
    pub(super) fn swap_pose(&mut self, cx: &App) -> (f32, f32) {
        let mo = &tokens(cx).motion;
        let p = self
            .tl
            .swap
            .and_then(|at| motion::pose(mo, motion::SWAP, false, at, Instant::now()));
        match p {
            Some(p) => {
                self.tl.moving = true;
                (4. * (1. - p.t), p.opacity)
            }
            None => (0., 1.),
        }
    }

    /// Something in the timeline moves: the next frame is drawn.
    pub(super) fn timeline_moving(&self) -> bool {
        self.tl.moving
    }

    /// One list row: the Load older button above the first message, the day row, the New row and the message.
    pub(super) fn message_row(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(msg) = self.model.log().and_then(|l| l.messages.get(ix)).cloned() else {
            return div().into_any_element();
        };
        let head = self.heads.get(ix).cloned().unwrap_or(RowHead {
            day: None,
            head: true,
            new: false,
        });
        let t = tokens(cx).clone();
        let id = msg.id;
        let count = self.model.log().map_or(0, |l| l.messages.len());
        let more_before = self.model.log().is_some_and(|l| l.more_before);
        let older = (ix == 0 && more_before).then(|| {
            div().py(px(8.)).pl(px(68.)).child(
                c::Button::new("load-older", "Load older messages")
                    .small()
                    .loading(self.model.loading_older)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.act(|m| m.scrolled(true, m.at_bottom), cx)
                    })),
            )
        });
        let day = head.day.clone().map(|label| {
            div()
                .debug_selector(move || format!("day-{id}"))
                .flex()
                .items_center()
                .gap(px(12.))
                .mt(px(16.))
                .ml(px(68.))
                .mr(px(16.))
                .text_step(TEXT_XS)
                .font_weight(t.weight_ui)
                .text_color(t.fg_3)
                .child(label)
                .child(div().flex_1().h(px(1.)).bg(t.line_subtle))
        });
        let new = head.new.then(|| self.new_row(&t, cx)).flatten();
        let row = self.message(&msg, head.head, window, cx);
        // Rows draw after the view's render asked for frames: a row that moves asks for the next one itself.
        if self.tl.moving {
            window.request_animation_frame();
        }
        div()
            .flex()
            .flex_col()
            .w_full()
            .when(ix == 0, |d| d.pt(px(8.)))
            .when(ix + 1 == count, |d| d.pb(px(16.)))
            .children(older)
            .children(day)
            .children(new)
            .child(row)
            .into_any_element()
    }

    /// `.new`: a 24 px row 8 px under the one before, 68 px in: an `--accent-line` rule, "New" and Mark as read. Read,
    /// it waits FADE_AFTER, fades over `--dur-3`, then folds its height and margin over `--dur-3`.
    fn new_row(&mut self, t: &Tokens, cx: &mut Context<Self>) -> Option<Div> {
        let mo = &t.motion;
        let (mut opacity, mut fold) = (1., 0.);
        if let Some(d) = self.tl.read_done {
            let start = d + FADE_AFTER;
            let now = Instant::now();
            if now >= start {
                let dur = mo.or(mo.dur_3, Duration::ZERO);
                let e = motion::ease(mo.ease_out);
                opacity = 1. - e(motion::fraction(start, now, dur));
                fold = e(motion::fraction(start + dur, now, dur));
                if fold >= 1. {
                    return None;
                }
                self.tl.moving = true;
            }
        }
        let room = self.model.active?;
        let last = self.model.log()?.messages.last()?.id;
        Some(
            div()
                .debug_selector(|| "new-row".into())
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(24. * (1. - fold)))
                .mt(px(8. * (1. - fold)))
                .ml(px(68.))
                .mr(px(16.))
                .overflow_hidden()
                .opacity(opacity)
                .child(div().flex_1().h(px(1.)).bg(t.accent_line))
                .child(
                    div()
                        .text_step(TEXT_XS)
                        .font_weight(t.weight_ui)
                        .text_color(t.accent_fg)
                        .child("New"),
                )
                .child({
                    let hover = (t.bg_hover, t.fg);
                    div()
                        .id("mark-read")
                        .px(px(8.))
                        .rounded(t.radius_sm)
                        .text_color(t.fg_2)
                        .text_size(px(12.))
                        .line_height(px(24.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover.0).text_color(hover.1))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.run(vec![Cmd::MarkRead { room, id: last }]);
                            cx.notify();
                        }))
                        .child("Mark as read")
                }),
        )
    }

    /// `article.msg`: a 40 px gutter (the 32 px avatar on a head row, else the time on hover), 12 px gap and the
    /// content, 4 px 16 px padding (a head row 8 px more above, inside and out), 14/20 text.
    fn message(
        &mut self,
        msg: &Message,
        head: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let t = tokens(cx).clone();
        let mo = t.motion.clone();
        let now = Instant::now();
        let loc = Locale::system();
        let id = msg.id;
        let menu_open = self.tl.menu == Some(id) || self.model.picking == Some(id);
        let editing = self.model.editing == Some(id);
        let (h, on_hover) = motion::fade(
            SharedString::from(format!("hl-{id}")),
            mo.or(mo.dur_2, Duration::ZERO),
            mo.or(mo.dur_3, Duration::ZERO),
            window,
            cx,
        );
        let lit = if editing || menu_open { 1. } else { h };
        let s = &self.model.state;
        let mentioned = !msg.deleted && msg.mentions.iter().any(|u| u == s.my_id());
        // The fill: a mention's faint amber, the highlight over it, or a jump's flash in place of both.
        let rest: Hsla = if mentioned {
            Hsla {
                a: 0.55,
                ..t.accent_subtle.into()
            }
        } else {
            transparent_black()
        };
        let lit_bg: Hsla = if mentioned {
            t.accent_subtle.into()
        } else {
            t.bg_hover.into()
        };
        let mut bg = mix(rest, lit_bg, lit);
        if let Some(f) = self
            .tl
            .flash
            .filter(|f| f.0 == id)
            .and_then(|(_, at)| motion::flash(&mo, at, now))
        {
            bg = mix(transparent_black(), t.accent_subtle.into(), f);
            self.tl.moving = true;
        }
        let quiet = mix(
            t.fg_3.into(),
            t.fg_2.into(),
            if mentioned { 1. } else { lit },
        );
        let author = s.users.get(&msg.author_id);
        let name = s.name_of(&msg.author_id);
        let accent = author.map_or(String::new(), |u| u.accent.clone());
        let rise = self
            .tl
            .rose
            .get(&id)
            .and_then(|at| motion::pose(&mo, RISE, false, *at, now));
        if rise.is_some() {
            self.tl.moving = true;
        }

        let quote = msg.reply_to.as_ref().map(|r| {
            let ex = plain(&parse(&r.excerpt));
            let ex = if ex.is_empty() {
                "a deleted message".into()
            } else {
                ex
            };
            let who = s.name_of(&r.author_id);
            let text = format!("Reply to {who}: {ex}");
            let name_at = "Reply to ".len()..("Reply to ".len() + who.len());
            let target = r.id;
            let (qh, q_hover) =
                motion::hover(SharedString::from(format!("quote-{id}")), window, cx);
            let fg = mix(quiet, t.fg.into(), qh);
            div()
                .id(SharedString::from(format!("quote-{id}")))
                .debug_selector(move || format!("quote-{id}"))
                .flex()
                .items_center()
                .gap(px(6.))
                .ml(px(52.))
                .mb(px(2.))
                .text_step(TEXT_XS)
                .text_color(fg)
                .cursor_pointer()
                .on_hover(q_hover)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.model.jump_to(target) {
                        cx.notify();
                    }
                }))
                .child(icon("reply").text_color(fg))
                .child(
                    div()
                        .min_w_0()
                        .ellipsis()
                        .child(StyledText::new(text).with_highlights([(
                            name_at,
                            gpui::HighlightStyle {
                                color: Some(t.fg_2.into()),
                                ..Default::default()
                            },
                        )])),
                )
        });

        // The name and the picture open the author's card, next to the name (PersonButton, `.face`).
        let anchor = format!("name-{id}");
        let open = {
            let (uid, key) = (msg.author_id.clone(), anchor.clone());
            let sel = format!("author-{id}");
            move |this: &mut Self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                // A drag over the name selected it: that is not a click on it.
                if !Selection::dragged_in(&sel, cx) {
                    this.open_person(uid.clone(), key.clone(), false, cx)
                }
            }
        };
        let gutter = if head {
            div().w(px(40.)).flex_none().child(
                div()
                    .id(SharedString::from(format!("face-{id}")))
                    .debug_selector(move || format!("face-{id}"))
                    .cursor_pointer()
                    .on_click(cx.listener(open.clone()))
                    .child(Avatar::new(msg.author_id.clone(), name.clone(), accent, 32)),
            )
        } else {
            div()
                .w(px(40.))
                .flex_none()
                .flex()
                .justify_end()
                .mt(px(2.))
                .text_step(TEXT_XS)
                .text_color(t.fg_2)
                .opacity(lit)
                .whitespace_nowrap()
                .child(clock(msg.created_at, loc, true))
        };

        let mut content = div().flex().flex_col().flex_1().min_w_0();
        if head {
            content = content.child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    // PersonButton's `font: inherit` wins over its `font-medium`: the name is in the row's weight.
                    .child(
                        div()
                            .id(SharedString::from(anchor.clone()))
                            .debug_selector(move || format!("name-{id}"))
                            .relative()
                            .text_color(t.fg)
                            .cursor_pointer()
                            .hover(|s| s.underline())
                            .on_click(cx.listener(open))
                            .child(self.probe(anchor))
                            .child(SelText::new(format!("author-{id}"), name).pointer()),
                    )
                    // Baseline to baseline with the 14/20 name: 3 px down.
                    .child(
                        div()
                            .mt(px(3.))
                            .text_step(TEXT_XS)
                            .text_color(quiet)
                            .child(clock(msg.created_at, loc, false)),
                    )
                    .when(msg.pinned, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .mt(px(3.))
                                .text_step(TEXT_XS)
                                .text_color(quiet)
                                .child(icon("pin").size(px(12.)).text_color(quiet))
                                .child("Pinned"),
                        )
                    }),
            );
        }
        if msg.deleted {
            content = content.child(div().italic().text_color(quiet).child("Message deleted"));
        } else if editing {
            content = content.child(self.edit_box(cx));
        } else {
            content = self.body(content, msg, head, quiet, bg, &t, cx);
        }

        let toolbar = (!msg.deleted && !editing).then(|| self.toolbar(msg, head, lit, now, cx));
        div()
            .id(SharedString::from(format!("msg-{id}")))
            .debug_selector(move || format!("message-{id}"))
            .relative()
            .when_some(rise, |d, p| d.top(px(6. * (1. - p.t))).opacity(p.opacity))
            .flex()
            .flex_col()
            .w_full()
            .px(px(16.))
            .pt(px(if head { 8. } else { 4. }))
            .pb(px(4.))
            .when(head, |d| d.mt(px(8.)))
            .text_size(px(14.))
            .line_height(px(20.))
            .text_color(t.fg)
            .bg(bg)
            .on_hover(on_hover)
            .when(mentioned, |d| {
                d.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.))
                        .bg(t.accent),
                )
            })
            .children(quote)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(gutter)
                    .child(content),
            )
            .children(toolbar)
    }

    /// The text (clamped past 20 lines behind Show more), the (edited) and Pinned note at the end of its last line, the
    /// files and the reactions.
    #[allow(clippy::too_many_arguments)]
    fn body(
        &mut self,
        content: Div,
        msg: &Message,
        head: bool,
        quiet: Hsla,
        bg: Hsla,
        t: &Tokens,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = msg.id;
        let names: Vec<&str> = msg.attachments.iter().map(|a| a.name.as_str()).collect();
        // A message of files only carries their names as its body: not shown twice.
        let filler = !msg.attachments.is_empty() && msg.body == names.join(", ");
        let edited = msg.edited_at.is_some();
        let note = match (edited, !head && msg.pinned) {
            (true, true) => Some("(edited) Pinned"),
            (true, false) => Some("(edited)"),
            (false, true) => Some("Pinned"),
            _ => None,
        }
        .map(|n| {
            div()
                .flex_none()
                .ml(px(4.))
                .mt(px(3.))
                .text_step(TEXT_XS)
                .text_color(quiet)
                .whitespace_nowrap()
                .child(n)
        });
        let long = is_long(&msg.body);
        let open = self.tl.expanded.contains(&id);
        let mut text: Vec<AnyElement> = vec![];
        let mut inline = false;
        if !filler {
            let nodes = parse(&msg.body);
            inline = md::ends_inline(&nodes);
            let this = cx.weak_entity();
            let ctx = md::Ctx {
                me: self
                    .model
                    .state
                    .me
                    .as_ref()
                    .map_or(String::new(), |m| m.user.username.clone()),
                known: self
                    .model
                    .state
                    .users
                    .values()
                    .map(|u| u.username.to_lowercase())
                    .collect(),
                t: t.clone(),
                id,
                copied: self
                    .tl
                    .copied
                    .filter(|c| c.0 == id && c.2.elapsed() < Duration::from_secs(2))
                    .map(|c| c.1),
                on_copy: Rc::new(move |block, code, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
                    let _ = this.update(cx, |v, cx| {
                        v.tl.copied = Some((id, block, Instant::now()));
                        cx.notify();
                        cx.spawn(async |this, cx| {
                            cx.background_executor().timer(Duration::from_secs(2)).await;
                            let _ = this.update(cx, |_, cx| cx.notify());
                        })
                        .detach();
                    });
                }),
            };
            text = md::render(&nodes, &ctx);
        }
        if let Some(n) = note {
            if inline {
                // The note ends the text's last line.
                let last = text.pop();
                text.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_start()
                        .children(last)
                        .child(n)
                        .into_any_element(),
                );
            } else {
                text.push(div().flex().child(n).into_any_element());
            }
        }
        let clamp = long && !open;
        let fade = over_app(t, bg);
        let mut content = content.child(
            div()
                .relative()
                .flex()
                .flex_col()
                .when(clamp, |d| d.max_h(px(400.)).overflow_hidden())
                .children(text)
                .when(clamp, |d| {
                    // The web's mask: the last quarter fades out.
                    d.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(100.))
                            .bg(linear_gradient(
                                180.,
                                linear_color_stop(Hsla { a: 0., ..fade }, 0.),
                                linear_color_stop(fade, 1.),
                            )),
                    )
                }),
        );
        if long {
            let (fg2, fg) = (t.fg_2, t.fg);
            content = content.child(
                div().flex().child(
                    div()
                        .id(SharedString::from(format!("more-link-{id}")))
                        .debug_selector(move || format!("show-more-{id}"))
                        .text_step(TEXT_XS)
                        .text_color(fg2)
                        .cursor_pointer()
                        .hover(move |s| s.text_color(fg))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.tl.expanded.remove(&id) {
                                this.tl.expanded.insert(id);
                            }
                            cx.notify();
                        }))
                        .child(if open { "Show less" } else { "Show more" }),
                ),
            );
        }
        for a in &msg.attachments {
            let el = self.attachment(a, cx);
            content = content.child(el);
        }
        content.children(self.reactions(msg, cx))
    }

    /// `.tools`: over the row's top right (4 px down on a head row), 1 px padding, a `--line` border, `--radius`,
    /// `--bg-raised`; React, Reply, Pin and More. It shows with the row's highlight, and stays while its menu is open.
    fn toolbar(
        &mut self,
        msg: &Message,
        head: bool,
        lit: f32,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = tokens(cx).clone();
        let m = &self.model;
        let id = msg.id;
        let menu = self.tl.menu == Some(id);
        let picking = m.picking == Some(id);
        let tool = |name: &str, icon: &'static str, pressed: bool, f: OnClick| Tool {
            id: format!("{name}-{id}").into(),
            icon,
            pressed,
            on_click: f,
        };
        let react = tool(
            "react",
            "smile",
            picking,
            Rc::new(cx.listener(move |this, _, _, cx| {
                this.tl.menu = None;
                this.model.toggle_picker(id);
                cx.notify();
            })),
        );
        let reply = tool(
            "reply",
            "reply",
            false,
            Rc::new(cx.listener(move |this, _, window, cx| {
                this.model.reply(id);
                this.composer.update(cx, |i, cx| i.focus(window, cx));
                cx.notify();
            })),
        );
        let pin = m.can_pin().then(|| {
            tool(
                "pin",
                "pin",
                false,
                Rc::new(cx.listener(move |this, _, _, cx| this.act(|m| m.pin(id), cx))),
            )
        });
        let more = tool(
            "more",
            "more",
            menu,
            Rc::new(cx.listener(move |this, _, _, cx| {
                this.model.picking = None;
                this.tl.menu = if this.tl.menu == Some(id) {
                    None
                } else {
                    Some(id)
                };
                cx.notify();
            })),
        );
        let picker = if picking {
            Some(self.picker(msg, now, cx))
        } else {
            None
        };
        let menu_el = if menu {
            Some(self.more_menu(msg, now, cx))
        } else {
            None
        };
        div()
            .absolute()
            .top(px(if head { 4. } else { 0. }))
            .right(px(16.))
            .flex()
            .p(px(1.))
            .border_1()
            .border_color(t.line)
            .rounded(t.radius)
            .bg(t.bg_raised)
            .opacity(if menu || picking { 1. } else { lit })
            .child(div().relative().child(react).children(picker))
            .child(reply)
            .children(pin)
            .child(div().relative().child(more).children(menu_el))
    }

    /// The More menu under its button (`Menu.svelte`): Edit (mine), Copy text, Copy link, Delete (mine, or anyone's
    /// for a moderator).
    fn more_menu(&mut self, msg: &Message, now: Instant, cx: &mut Context<Self>) -> AnyElement {
        let mo = tokens(cx).motion.clone();
        let at = self.tl.shown_since(format!("menu-{}", msg.id), now);
        let o = self.tl.pop(&mo, at, now);
        let m = &self.model;
        let id = msg.id;
        let body = msg.body.clone();
        let link = format!("{}/r/{}#m{id}", m.server.trim_end_matches('/'), msg.room_id);
        let close = |this: &mut Vesper| this.tl.menu = None;
        let item = |name: &str, label: &'static str, icon_name: &'static str| {
            let sel = format!("{name}-{id}");
            (
                sel.clone(),
                MenuItem::new(ElementId::Name(sel.into()), label).icon(icon_name),
            )
        };
        let tag = |(sel, item): (String, MenuItem)| div().debug_selector(move || sel).child(item);
        let mut list = popover_frame(cx)
            .id(SharedString::from(format!("menu-{id}")))
            .debug_selector(move || format!("menu-{id}"))
            .opacity(o)
            .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                close(this);
                cx.notify();
            }));
        if m.mine(msg) {
            let (sel, it) = item("edit", "Edit", "edit");
            list = list.child(tag((
                sel,
                it.on_select(cx.listener(move |this, _, window, cx| {
                    close(this);
                    this.start_edit(id, window, cx)
                })),
            )));
        }
        let (sel, it) = item("copy-text", "Copy text", "copy");
        list = list.child(tag((
            sel,
            it.on_select(cx.listener(move |this, _, _, cx| {
                close(this);
                cx.write_to_clipboard(ClipboardItem::new_string(body.clone()));
                cx.notify();
            })),
        )));
        let (sel, it) = item("copy-link", "Copy link", "link");
        list = list.child(tag((
            sel,
            it.on_select(cx.listener(move |this, _, _, cx| {
                close(this);
                cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                cx.notify();
            })),
        )));
        if m.can_delete(msg) {
            let (sel, it) = item("delete", "Delete", "trash");
            list = list.child(tag((
                sel,
                it.danger(true)
                    .on_select(cx.listener(move |this, _, _, cx| {
                        close(this);
                        this.act(|m| m.delete(id, false), cx)
                    })),
            )));
        }
        // `place()`: under the button at its left, 4 px off, kept 8 px inside the window.
        div()
            .absolute()
            .top(px(24.))
            .left_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(div().pt(px(4.)).child(list)),
                )
                .with_priority(1),
            )
            .into_any_element()
    }

    /// The reaction picker under React (`Popover.svelte`): the three quick emoji, 32 px each, `--radius`, 16 px; one
    /// I gave already sits on `--bg-hover`.
    fn picker(&mut self, msg: &Message, now: Instant, cx: &mut Context<Self>) -> AnyElement {
        let t = tokens(cx).clone();
        let at = self.tl.shown_since(format!("picker-{}", msg.id), now);
        let o = self.tl.pop(&t.motion, at, now);
        let id = msg.id;
        let me = self.model.state.my_id().to_string();
        let has = |e: &str| {
            msg.reactions
                .iter()
                .any(|r| r.emoji == e && r.user_ids.contains(&me))
        };
        let hover = t.bg_hover;
        let row = div()
            .flex()
            .items_center()
            .gap(px(4.))
            .children(EMOJIS.iter().enumerate().map(|(i, e)| {
                div()
                    .id(SharedString::from(format!("pick-{id}-{i}")))
                    .debug_selector(move || format!("pick-{id}-{i}"))
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(32.))
                    .rounded(t.radius)
                    .text_size(px(16.))
                    .when(has(e), |d| d.bg(hover))
                    .hover(move |s| s.bg(hover))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.act(|m| m.react(id, e), cx)))
                    .child(*e)
            }));
        div()
            .absolute()
            .top(px(24.))
            .left_0()
            .child(
                deferred(
                    anchored()
                        .anchor(Corner::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div().pt(px(4.)).child(
                                popover_frame(cx)
                                    .id(SharedString::from(format!("picker-{id}")))
                                    .debug_selector(move || format!("picker-{id}"))
                                    .opacity(o)
                                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                        this.model.picking = None;
                                        cx.notify();
                                    }))
                                    .child(row),
                            ),
                        ),
                )
                .with_priority(1),
            )
            .into_any_element()
    }

    /// The edit box in place of the text: a 2-line `.control`, then Save, Cancel and the keys.
    fn edit_box(&self, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx);
        div()
            .flex()
            .flex_col()
            .key_context(EDIT_BOX)
            .on_action(cx.listener(Self::save_edit))
            .on_action(cx.listener(Self::cancel_edit))
            .on_action(cx.listener(|this, _: &NewLine, window, cx| {
                this.edit.update(cx, |i, cx| i.insert("\n", window, cx));
            }))
            .debug_selector(|| "edit-box".into())
            .child(
                div().mt(px(4.)).child(
                    Input::new(&self.edit)
                        .bg(t.bg_element)
                        .border_1()
                        .border_color(t.line)
                        .rounded(t.radius)
                        .text_color(t.fg),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .mt(px(4.))
                    .text_step(TEXT_XS)
                    .text_color(t.fg_2)
                    .child(
                        div().debug_selector(|| "edit-save".into()).child(
                            c::Button::new("edit-save", "Save")
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_edit(&SaveEdit, window, cx)
                                })),
                        ),
                    )
                    .child(
                        c::Button::new("edit-cancel", "Cancel")
                            .small()
                            .variant(c::Variant::Ghost)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.cancel_edit(&CancelEdit, window, cx)
                            })),
                    )
                    .child("Enter saves, Esc cancels."),
            )
            .when(!self.model.edit_error.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| "edit-error".into())
                        .text_step(TEXT_XS)
                        .text_color(t.danger_fg)
                        .child(self.model.edit_error.clone()),
                )
            })
    }

    /// The reaction pills: 24 px, 8 px sides, 4 px gap, a `--line-subtle` border, fully round, `--bg-element`,
    /// 12/16 `--fg-2`; mine in the accent. A new one bumps in (scale 0.6 to 1, `--ease-pop`), a new count bumps alone.
    fn reactions(&mut self, msg: &Message, cx: &mut Context<Self>) -> Option<Div> {
        if msg.reactions.is_empty() {
            return None;
        }
        let t = tokens(cx).clone();
        let now = Instant::now();
        let s = &self.model.state;
        let me = s.my_id().to_string();
        let id = msg.id;
        let mut moving = false;
        let pills: Vec<AnyElement> = msg
            .reactions
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let on = r.user_ids.contains(&me);
                let emoji = r.emoji.clone();
                let pick = r.emoji.clone();
                let bump = self
                    .tl
                    .bumps
                    .get(&(id, r.emoji.clone()))
                    .and_then(|(at, whole)| {
                        motion::pose(&t.motion, motion::BUMP, false, *at, now).map(|p| (p, *whole))
                    });
                moving |= bump.is_some();
                let (border, bg, fg) = if on {
                    (t.accent_line, t.accent_subtle, t.accent_fg)
                } else {
                    (t.line_subtle, t.bg_element, t.fg_2)
                };
                let (hover_line, hover_fg) = (t.line_strong, t.fg);
                let count = r.user_ids.len().to_string();
                // A scale is drawn as a smaller pill (or count) in the middle of its full-size box, so nothing moves.
                let pill = |k: f32, count_k: f32, opacity: f32| {
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(4. * k))
                        .h(px(24. * k))
                        .px(px(8. * k))
                        .border_1()
                        .border_color(border)
                        .rounded_full()
                        .bg(bg)
                        .text_color(fg)
                        .text_size(px(12. * k))
                        .line_height(px(16. * k))
                        .opacity(opacity)
                        .child(emoji.clone())
                        .child(
                            div()
                                .relative()
                                .child(div().invisible().child(count.clone()))
                                .child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_size(px(12. * k * count_k))
                                        .child(count.clone()),
                                ),
                        )
                };
                let sel = format!("reaction-{id}-{i}");
                let el = div()
                    .id(SharedString::from(sel.clone()))
                    .debug_selector(move || sel.clone())
                    .relative()
                    .flex_none()
                    .cursor_pointer()
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.act(|m| m.react(id, &pick), cx)),
                    );
                match bump {
                    Some((p, true)) => el
                        .child(pill(1., 1., 0.).invisible())
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(pill(0.6 + 0.4 * p.t, 1., p.opacity)),
                        )
                        .into_any_element(),
                    Some((p, false)) => el.child(pill(1., 0.6 + 0.4 * p.t, 1.)).into_any_element(),
                    None => el
                        .group("rx")
                        .child(pill(1., 1., 1.).group_hover("rx", move |s| {
                            s.border_color(hover_line).text_color(hover_fg)
                        }))
                        .into_any_element(),
                }
            })
            .collect();
        if moving {
            self.tl.moving = true;
        }
        Some(
            div()
                .debug_selector(move || format!("reactions-{id}"))
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .mt(px(4.))
                .children(pills),
        )
    }

    /// An image (`.img`): fetched once with the session cookie, in a 360 x 280 `--bg-element` box until it comes, then
    /// its own shape with no fill; one that fails is a 48 px box with a line of text. Anything else is a card.
    fn attachment(&mut self, a: &Attachment, cx: &mut Context<Self>) -> AnyElement {
        let t = tokens(cx).clone();
        if a.kind == AttachmentKind::Image && a.size as u64 <= IMAGE_MAX {
            if !self.images.contains_key(&a.id) {
                self.images.insert(a.id.clone(), Img::Loading);
                self.run(vec![Cmd::FetchImage(a.id.clone())]);
            }
            let open = {
                let id = a.id.clone();
                cx.listener(move |this, _, _, cx| this.open_viewer(&id, cx))
            };
            let boxed = |w: f32, h: f32| {
                div()
                    .mt(px(4.))
                    .w(px(w))
                    .h(px(h))
                    .max_w_full()
                    .rounded(t.radius)
                    .overflow_hidden()
                    .bg(t.bg_element)
            };
            let el = match &self.images[&a.id] {
                Img::Ready(i) => {
                    let s = i.size(0);
                    let (w, h) = fit(s.width.0 as u32, s.height.0 as u32);
                    // Loaded: no fill behind it, so a transparent picture blends in.
                    boxed(w as f32, h as f32)
                        .bg(gpui::transparent_black())
                        .child(picture(i, format!("pic-{}", a.id), w as f32, h as f32, cx))
                }
                Img::Loading => boxed(BOX.0 as f32, BOX.1 as f32),
                Img::Failed => boxed(BOX.0 as f32, 48.)
                    .flex()
                    .items_center()
                    .px(px(12.))
                    .text_step(TEXT_XS)
                    .text_color(t.fg_2)
                    .child(IMAGE_FAILED),
            };
            let failed = matches!(self.images[&a.id], Img::Failed);
            let sel = format!("image-{}", a.id);
            return div()
                .id(SharedString::from(sel.clone()))
                .debug_selector(move || sel)
                .flex()
                .when(!failed, |d| d.cursor_pointer().on_click(open))
                .child(el)
                .into_any_element();
        }
        self.card(a, &t, cx).into_any_element()
    }

    /// `.card`: 360 px at most, 8 px 8 px 8 px 12 px, 12 px gap, a `--line-subtle` border, `--radius`, `--bg-raised`,
    /// 13/20: the file icon, the name and "PDF file, 1 KB", then Open (audio and video, in the system's player: the
    /// web plays them in the card) and Download.
    fn card(&self, a: &Attachment, t: &Tokens, cx: &mut Context<Self>) -> Div {
        let id = a.id.clone();
        let (open, save) = (a.clone(), a.clone());
        let meta = [
            type_label(&a.mime, &a.name),
            file_size(a.size.max(0) as u64),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
        let media = matches!(a.kind, AttachmentKind::Audio | AttachmentKind::Video);
        let hover = (t.bg_hover, t.fg);
        div()
            .debug_selector(move || format!("file-{id}"))
            .flex()
            .items_center()
            .gap(px(12.))
            .mt(px(4.))
            .w(px(360.))
            .max_w_full()
            .pl(px(12.))
            .pr(px(8.))
            .py(px(8.))
            .border_1()
            .border_color(t.line_subtle)
            .rounded(t.radius)
            .bg(t.bg_raised)
            .text_step(TEXT_SM)
            .child(icon("file").text_color(t.fg_2))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().ellipsis().text_color(t.fg).child(a.name.clone()))
                    .child(div().text_step(TEXT_XS).text_color(t.fg_2).child(meta)),
            )
            .when(media, |d| {
                let sel = format!("open-{}", a.id);
                d.child(
                    div().debug_selector(move || sel).child(
                        c::IconButton::new(
                            ElementId::Name(format!("open-{}", a.id).into()),
                            "expandIcon",
                        )
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.open_attachment(&open, cx)),
                        ),
                    ),
                )
            })
            .child({
                let sel = format!("save-{}", a.id);
                div()
                    .id(ElementId::Name(format!("save-{}", a.id).into()))
                    .debug_selector(move || sel)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .h(px(28.))
                    .px(px(8.))
                    .rounded(t.radius_sm)
                    .text_color(t.fg)
                    .text_step(TEXT_XS)
                    .font_weight(t.weight_ui)
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover.0).text_color(hover.1))
                    .on_click(cx.listener(move |this, _, _, cx| this.save_attachment(&save, cx)))
                    .child(icon("download").text_color(t.fg))
                    .child("Download")
            })
    }

    /// Hands a media file to the system's app. Downloaded first, named by its id and the type the server found.
    pub(super) fn open_attachment(&mut self, a: &Attachment, cx: &mut Context<Self>) {
        let Some(ext) = files::media_ext(&a.mime).filter(|_| files::valid_id(&a.id)) else {
            return;
        };
        let path = files::cache_dir("open").join(format!("{}.{ext}", a.id));
        if path.exists() {
            open_file(&path, cx);
        } else {
            self.run(vec![Cmd::Download {
                id: a.id.clone(),
                to: path,
                open: true,
            }]);
        }
    }

    /// Saves where the user picks, starting in Downloads. With no dialog (no portal on Linux) it goes to Downloads.
    pub(super) fn save_attachment(&mut self, a: &Attachment, cx: &mut Context<Self>) {
        if !files::valid_id(&a.id) {
            return;
        }
        let dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir);
        let name = files::safe_name(&a.name);
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        let id = a.id.clone();
        cx.spawn(async move |this, cx| {
            let to = match rx.await {
                Ok(Ok(Some(p))) => p,
                Ok(Ok(None)) => return,
                _ => files::unique_path(&dir, &name),
            };
            let _ = this.update(cx, |v, _| {
                v.run(vec![Cmd::Download {
                    id,
                    to,
                    open: false,
                }])
            });
        })
        .detach();
    }

    /// A picture came from the network thread: kept for drawing, the oldest let go past `IMAGE_CACHE`.
    pub(super) fn image_loaded(&mut self, id: &str, picture: Option<Picture>, window: &mut Window) {
        let img = picture.and_then(render_image);
        self.images
            .insert(id.to_string(), img.map_or(Img::Failed, Img::Ready));
        self.image_order.push_back(id.to_string());
        while self.image_order.len() > IMAGE_CACHE {
            let Some(old) = self.image_order.pop_front() else {
                break;
            };
            if let Some(Img::Ready(i)) = self.images.remove(&old) {
                let _ = window.drop_image(i);
            }
        }
    }

    pub(super) fn start_edit(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(body) = self.model.start_edit(id) {
            self.edit.update(cx, |i, cx| {
                let end = body.len();
                i.set_value(body, window, cx);
                // The caret at the end, as the web's edit box; this also focuses it.
                let pos = i.text().offset_to_position(end);
                i.set_cursor_position(pos, window, cx);
            });
        }
        cx.notify();
    }

    fn save_edit(&mut self, _: &SaveEdit, window: &mut Window, cx: &mut Context<Self>) {
        if super::end_composition(&self.edit, window, cx) {
            return;
        }
        let body = self.edit.read(cx).value();
        self.act(|m| m.save_edit(&body), cx);
    }

    fn cancel_edit(&mut self, _: &CancelEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.model.cancel_edit();
        self.composer.update(cx, |i, cx| i.focus(window, cx));
        cx.notify();
    }

    /// The people the @ list offers now.
    pub(super) fn people(&self) -> Vec<UserInfo> {
        let s = &self.model.state;
        let (Some((_, q)), Some(room)) = (
            &self.mention,
            self.model.active.and_then(|id| s.find_room(id)),
        ) else {
            return vec![];
        };
        mentionable(q, s.users.values(), room, s.my_id())
            .into_iter()
            .cloned()
            .collect()
    }

    /// The composer's text changed: the @ list follows the caret, and typing goes out.
    pub(super) fn composer_changed(&mut self, cx: &mut Context<Self>) {
        let (text, caret) = {
            let c = self.composer.read(cx);
            (c.value(), c.cursor())
        };
        let q = mention_query(&text, caret);
        if q.as_ref().map(|q| q.0) != self.mention.as_ref().map(|m| m.0) {
            self.mention_ix = 0;
        }
        // An @ list closed with Esc stays shut while that @ is typed.
        if q.is_some() && q.as_ref().map(|q| q.0) == self.mention_dismissed {
            self.mention = None;
        } else {
            (self.mention, self.mention_dismissed) = (q, None);
        }
        // A room's draft put back is not typing.
        if !std::mem::take(&mut self.draft_put) {
            self.act(|m| m.typed(&text, Instant::now()), cx);
        }
    }

    /// Puts `@username ` in place of the @ being typed.
    pub(super) fn pick_mention(
        &mut self,
        username: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((start, _)) = self.mention.take() else {
            return;
        };
        self.composer.update(cx, |i, cx| {
            let (text, caret) = (i.value(), i.cursor());
            let ins = format!("@{username} ");
            let new = format!("{}{ins}{}", &text[..start], &text[caret.max(start)..]);
            i.set_value(new, window, cx);
            let pos = i.text().offset_to_position(start + ins.len());
            i.set_cursor_position(pos, window, cx);
        });
        cx.notify();
    }

    pub(super) fn composer_up(
        &mut self,
        _: &ComposerUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.people().len();
        if n > 0 {
            self.mention_ix = (self.mention_ix + n - 1) % n;
            return cx.notify();
        }
        let empty = self.composer.read(cx).value().is_empty();
        let files = self
            .model
            .active
            .is_some_and(|r| self.model.tray.of(r).next().is_some());
        if empty
            && !files
            && let Some((id, _)) = self.model.edit_last()
        {
            return self.start_edit(id, window, cx);
        }
        cx.propagate();
    }

    pub(super) fn composer_down(
        &mut self,
        _: &ComposerDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let n = self.people().len();
        if n == 0 {
            return cx.propagate();
        }
        self.mention_ix = (self.mention_ix + 1) % n;
        cx.notify();
    }

    pub(super) fn composer_tab(
        &mut self,
        _: &ComposerTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pick_active(window, cx) {
            cx.propagate();
        }
    }

    /// Enter or Tab with the @ list open takes the highlighted person.
    pub(super) fn pick_active(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let people = self.people();
        let Some(u) = people.get(self.mention_ix.min(people.len().saturating_sub(1))) else {
            return false;
        };
        let name = u.username.clone();
        self.pick_mention(&name, window, cx);
        true
    }

    pub(super) fn composer_escape(
        &mut self,
        _: &ComposerEscape,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((start, _)) = self.mention.take() {
            self.mention_dismissed = Some(start);
        } else if self.model.reply_to.is_some() {
            self.model.cancel_reply();
        } else {
            return cx.propagate();
        }
        cx.notify();
    }

    /// A picture on the clipboard goes to the tray as a file; anything else pastes as text.
    pub(super) fn paste_file(&mut self, _: &PasteFile, _: &mut Window, cx: &mut Context<Self>) {
        let image = cx
            .read_from_clipboard()
            .and_then(|c| pasted_image(c.entries()).cloned());
        let Some(image) = image else {
            return cx.propagate();
        };
        let ext = match image.format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Webp => "webp",
            ImageFormat::Gif => "gif",
            ImageFormat::Svg => "svg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Tiff => "tiff",
        };
        let dir = files::cache_dir("paste"); // the network thread deletes it once uploaded
        let name = format!(
            "Pasted image {}.{ext}",
            Local::now().format("%Y-%m-%d %H.%M")
        );
        let path = files::unique_path(&dir, &name);
        let wrote = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &image.bytes));
        match wrote {
            Ok(()) => self.attach_paths(vec![path], cx),
            Err(_) => {
                self.model.error = crate::uploads::UNREADABLE.into();
                cx.notify();
            }
        }
    }

    /// Files from the button, a drop or a paste. Folders are left out.
    pub(super) fn attach_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let files: Vec<(PathBuf, u64)> = paths
            .into_iter()
            .filter_map(|p| {
                let meta = std::fs::metadata(&p).ok().filter(|m| m.is_file())?;
                Some((p, meta.len()))
            })
            .collect();
        self.act(|m| m.attach(files), cx);
    }

    pub(super) fn pick_files(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |v, cx| v.attach_paths(paths, cx));
            }
        })
        .detach();
    }

    /// The composer (`Composer.svelte`): 16 px sides and bottom; the 16 px line above the box (the error, or who is
    /// typing with the dots); the box (a `--line` border, `--line-strong` on hover, `--focus` with focus, `--radius-lg`,
    /// `--bg-element`) with the reply bar, the upload tray and the row: Attach, the text (1 to 10 lines), the count
    /// past 3600 characters, Formatting help and Send. The @ list floats 4 px over the box.
    pub(super) fn render_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx).clone();
        let now = Instant::now();
        let m = &self.model;
        let Some(room) = m.active else {
            return div();
        };
        let text = self.composer.read(cx).value();
        let count = text.trim().chars().count();
        let over = count.saturating_sub(MAX_CHARS);
        let busy = m.tray.busy(room);
        let ready = m.tray.of(room).any(|i| i.attachment.is_some());
        let can_send = m.online && over == 0 && !busy && (!text.trim().is_empty() || ready);
        let focused = self.composer.read(cx).focus_handle(cx).is_focused(window);
        let (h, on_hover) = motion::hover("composer-box", window, cx);
        let border = if focused {
            t.focus.into()
        } else {
            mix(t.line.into(), t.line_strong.into(), h)
        };
        let slot = self.composer_slot(over, &t, now, cx);
        let reply = self.reply_bar(&t, window, cx);
        let away = self.away_line(room, &t);
        let tray = self.upload_tray(room, &t, window, cx);
        let people = self.mention_list(&t, now, cx);
        let help = self.tl.help.then(|| self.help(&t, now, cx));
        let send_fg: Hsla = if can_send {
            t.accent_fg.into()
        } else {
            t.fg_disabled.into()
        };
        let attach = composer_icon("attach", "paperclip", t.fg_2.into(), true, window, cx)
            .on_click(cx.listener(|this, _, _, cx| this.pick_files(cx)));
        let help_button = div()
            .relative()
            .child(
                composer_icon("help-button", "help", t.fg_2.into(), true, window, cx).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.tl.help = !this.tl.help;
                        cx.notify();
                    }),
                ),
            )
            .children(help);
        let send = composer_icon("send", "send", send_fg, can_send, window, cx).on_click(
            cx.listener(|this, _, window, cx| this.send(&super::SendMessage, window, cx)),
        );
        let counter = (count >= COUNT_FROM).then(|| {
            div()
                .px(px(4.))
                .text_size(px(12.))
                .line_height(px(32.))
                .text_color(if over > 0 { t.danger_fg } else { t.fg_2 })
                .child(format!("{count}/{MAX_CHARS}"))
        });
        div().flex_none().px(px(16.)).pb(px(16.)).child(slot).child(
            div()
                .id("composer-box")
                .relative()
                .border_1()
                .border_color(border)
                .rounded(t.radius_lg)
                .bg(t.bg_element)
                .on_hover(on_hover)
                .children(reply)
                .children(away)
                .children(tray)
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(4.))
                        .p(px(4.))
                        .child(attach)
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.composer)
                                    .appearance(false)
                                    .px(px(4.))
                                    .py(px(6.))
                                    .text_size(px(14.))
                                    .text_color(t.fg),
                            ),
                        )
                        .children(counter)
                        .child(help_button)
                        .child(send),
                )
                .children(people),
        )
    }

    /// `.slot`: one 16 px line, 4 px over the box, 12/16: an error (danger), the offline line, the too-long line, or
    /// who is typing in `--fg-3` after the typing mark (one typer's 16 px picture and three pulsing dots, coming in
    /// with `row`: 8 px from the left).
    fn composer_slot(
        &mut self,
        over: usize,
        t: &Tokens,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> Div {
        let m = &self.model;
        let mo = t.motion.clone();
        let slot = div()
            .h(px(16.))
            .mb(px(4.))
            .flex()
            .items_center()
            .text_step(TEXT_XS)
            .overflow_hidden();
        if !m.error.is_empty() {
            return slot.child(
                div()
                    .debug_selector(|| "room-error".into())
                    .ellipsis()
                    .text_color(t.danger_fg)
                    .child(m.error.clone()),
            );
        }
        if !m.online {
            return slot.child(div().ellipsis().text_color(t.fg_2).child(OFFLINE));
        }
        if over > 0 {
            let s = if over == 1 { "character" } else { "characters" };
            return slot.child(
                div()
                    .text_color(t.danger_fg)
                    .child(format!("Too long by {over} {s}.")),
            );
        }
        let Some(room) = m.active else { return slot };
        let ids = m.typers(room, now);
        let names: Vec<String> = ids.iter().map(|u| m.state.name_of(u)).collect();
        let line = typing_line(&names);
        if line.is_empty() {
            return slot;
        }
        let typer = (ids.len() == 1)
            .then(|| m.state.users.get(&ids[0]).cloned())
            .flatten();
        let at = self.tl.shown_since("typing".into(), now);
        let enter = motion::pose(&mo, motion::ROW, false, at, now);
        // The dots never stop: one frame a tick, as the web's CSS animation.
        self.tl.moving = !mo.reduced || enter.is_some() || self.tl.moving;
        let since = now.duration_since(at);
        let mark = (!mo.reduced).then(|| {
            div()
                .relative()
                .when_some(enter, |d, p| {
                    d.left(px(-8. * (1. - p.t))).opacity(p.opacity)
                })
                .flex()
                .items_center()
                .gap(px(4.))
                .mr(px(6.))
                .h(px(16.))
                .when_some(typer, |d, u| {
                    d.child(Avatar::new(u.user_id, u.display_name, u.accent, 16))
                })
                .child(div().flex().gap(px(3.)).children((0..3).map(|i| {
                    div()
                        .size(px(4.))
                        .rounded_full()
                        .bg(t.fg_3)
                        .opacity(motion::dot(&mo, since, i))
                })))
        });
        let _ = cx;
        slot.child(
            div()
                .debug_selector(|| "typing".into())
                .flex()
                .items_center()
                .min_w_0()
                .text_color(t.fg_3)
                .children(mark)
                .child(div().ellipsis().child(line)),
        )
    }

    /// `.bar`: 32 px, 12 px left and 4 px right, an 8 px gap, a `--line-subtle` rule under it, 12/16 `--fg-2`:
    /// "Replying to <name>" and the text, and a 24 px Cancel reply.
    fn reply_bar(&self, t: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let m = &self.model;
        let r = m.reply_to.as_ref()?;
        // `swap` in: 4 px up and a fade, as the web's reply bar.
        let came = motion::since(SharedString::from(format!("reply-{}", r.id)), window, cx);
        let pose = motion::pose(&t.motion, motion::SWAP, false, came, Instant::now());
        if pose.is_some() {
            window.request_animation_frame();
        }
        let (dy, alpha) = pose.map_or((0., 1.), |p| (4. * (1. - p.t), p.opacity));
        let ex = plain(&parse(&r.body));
        let ex = if ex.is_empty() {
            "a message with a file".into()
        } else {
            ex
        };
        let who = m.state.name_of(&r.author_id);
        let text = format!("Replying to {who}  {ex}");
        let at = "Replying to ".len();
        let hover = (t.bg_hover, t.fg);
        Some(
            div()
                .debug_selector(|| "reply-bar".into())
                .relative()
                .top(px(dy))
                .opacity(alpha)
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(32.))
                .pl(px(12.))
                .pr(px(4.))
                .border_b_1()
                .border_color(t.line_subtle)
                .text_step(TEXT_XS)
                .text_color(t.fg_2)
                .child(icon("reply").text_color(t.fg_2))
                .child(div().flex_1().min_w_0().ellipsis().child(
                    StyledText::new(text).with_highlights([(
                        at..at + who.len(),
                        gpui::HighlightStyle {
                            color: Some(t.fg.into()),
                            font_weight: Some(t.weight_strong),
                            ..Default::default()
                        },
                    )]),
                ))
                .child(
                    div()
                        .id("reply-cancel")
                        .debug_selector(|| "reply-cancel".into())
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.))
                        .rounded(t.radius_sm)
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover.0))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.model.cancel_reply();
                            this.composer.update(cx, |i, cx| i.focus(window, cx));
                            cx.notify();
                        }))
                        .child(icon("x").text_color(t.fg_2)),
                ),
        )
    }

    /// `.away`: uploads still running in other rooms, so a room change never hides them.
    fn away_line(&self, room: i64, t: &Tokens) -> Option<Div> {
        let m = &self.model;
        let mut names: Vec<String> = vec![];
        for i in &m.tray.items {
            if i.room != room
                && matches!(i.state, UpState::Queued | UpState::Uploading)
                && let Some(r) = m.state.find_room(i.room)
                && !names.contains(&r.name)
            {
                names.push(r.name.clone());
            }
        }
        (!names.is_empty()).then(|| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(32.))
                .px(px(12.))
                .border_b_1()
                .border_color(t.line_subtle)
                .text_step(TEXT_XS)
                .text_color(t.fg_2)
                .child(icon("upload").text_color(t.fg_2))
                .child(
                    div()
                        .ellipsis()
                        .child(format!("Uploading in {}", names.join(", "))),
                )
        })
    }

    /// `UploadTray.svelte`: 4 px padding and a `--line-subtle` rule; per file a 32 px row (4 px, 8 px at the left,
    /// `--radius`, 13/20): a 24 px thumb (the file's kind, or the alert in danger when it failed), the name, the size
    /// (12 px `--fg-2`), the state (a 4 px bar up to 160 px and the share, Waiting, Ready, or Retry), and Cancel or
    /// Remove. A failed row has its reason under it. The desktop draws no picture preview: the thumb is the kind's icon.
    fn upload_tray(
        &self,
        room: i64,
        t: &Tokens,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let items: Vec<_> = self.model.tray.of(room).cloned().collect();
        if items.is_empty() {
            return None;
        }
        let hover = t.bg_hover;
        // The columns line up across rows, as the web's subgrid: the widest name and size set where the next start.
        let width = |text: &str, size: f32| {
            let run = gpui::TextRun {
                len: text.len(),
                font: gpui::font(t.font_sans.clone()),
                color: t.fg.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .layout_line(text, px(size), &[run], None)
                .width
        };
        let name_w = items
            .iter()
            .map(|i| width(&i.name, 13.))
            .fold(px(0.), |a, b| a.max(b));
        let size_w = items
            .iter()
            .map(|i| width(&file_size(i.size), 12.))
            .fold(px(0.), |a, b| a.max(b));
        Some(
            div()
                .debug_selector(|| "upload-tray".into())
                .flex()
                .flex_col()
                .p(px(4.))
                .border_b_1()
                .border_color(t.line_subtle)
                .children(items.into_iter().map(|i| {
                    let key = i.key;
                    let busy = matches!(i.state, UpState::Queued | UpState::Uploading);
                    let failed = i.state == UpState::Failed;
                    let ext = i
                        .name
                        .rsplit_once('.')
                        .map(|x| x.1.to_ascii_lowercase())
                        .unwrap_or_default();
                    let thumb = if failed {
                        "error"
                    } else if matches!(
                        ext.as_str(),
                        "mp3" | "m4a" | "ogg" | "wav" | "flac" | "opus"
                    ) {
                        "volume"
                    } else if matches!(ext.as_str(), "mp4" | "mov" | "webm") {
                        // The web's first frame of the video; none here, so the box stays empty.
                        ""
                    } else {
                        "file"
                    };
                    let state: Option<AnyElement> = match i.state {
                        UpState::Uploading => {
                            let pct = (i.progress * 100.).round();
                            Some(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .flex_1()
                                    .child(
                                        div()
                                            .relative()
                                            .w(px(160.))
                                            .min_w(px(48.))
                                            .h(px(4.))
                                            .rounded(px(2.))
                                            .overflow_hidden()
                                            .bg(t.line)
                                            .child(
                                                div()
                                                    .absolute()
                                                    .top_0()
                                                    .bottom_0()
                                                    .left_0()
                                                    .w(px(1.6 * pct))
                                                    .bg(t.accent),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .w(px(32.))
                                            .flex()
                                            .justify_end()
                                            .text_color(t.fg_2)
                                            .child(format!("{pct}%")),
                                    )
                                    .into_any_element(),
                            )
                        }
                        UpState::Queued => {
                            Some(div().text_color(t.fg_2).child("Waiting").into_any_element())
                        }
                        UpState::Done => Some(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(4.))
                                .text_color(t.fg_2)
                                .child(icon("check").text_color(t.fg_2))
                                .child("Ready")
                                .into_any_element(),
                        ),
                        UpState::Failed if i.retry => Some(
                            div()
                                .id(SharedString::from(format!("upload-retry-{key}")))
                                .debug_selector(move || format!("upload-retry-{key}"))
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .h(px(28.))
                                .px(px(8.))
                                .rounded(t.radius_sm)
                                .text_color(t.fg)
                                .font_weight(t.weight_ui)
                                .cursor_pointer()
                                .hover(move |s| s.bg(hover))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.act(|m| m.retry_upload(key), cx)
                                }))
                                .child(icon("retry").text_color(t.fg))
                                .child("Retry")
                                .into_any_element(),
                        ),
                        UpState::Failed => None,
                    };
                    div()
                        .debug_selector(move || format!("upload-{key}"))
                        .flex()
                        .flex_col()
                        .min_h(px(32.))
                        .pl(px(8.))
                        .pr(px(4.))
                        .py(px(4.))
                        .rounded(t.radius)
                        .text_step(TEXT_SM)
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .size(px(24.))
                                        .rounded(t.radius_sm)
                                        .bg(t.bg_raised)
                                        .when(!thumb.is_empty(), |d| {
                                            d.child(icon(thumb).text_color(if failed {
                                                t.danger_fg
                                            } else {
                                                t.fg_2
                                            }))
                                        }),
                                )
                                .child(
                                    div()
                                        .w(name_w + px(1.))
                                        .flex_shrink()
                                        .min_w_0()
                                        .ellipsis()
                                        .text_color(t.fg)
                                        .child(i.name.clone()),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .w(size_w + px(1.))
                                        .text_size(px(12.))
                                        .text_color(t.fg_2)
                                        .child(file_size(i.size)),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_1()
                                        .min_w_0()
                                        .items_center()
                                        .text_step(TEXT_XS)
                                        .children(state),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!("upload-remove-{key}")))
                                        .debug_selector(move || format!("upload-remove-{key}"))
                                        .flex()
                                        .flex_none()
                                        .items_center()
                                        .justify_center()
                                        .size(px(24.))
                                        .rounded(t.radius_sm)
                                        .cursor_pointer()
                                        .hover(move |s| s.bg(hover))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.act(|m| m.remove_upload(key), cx)
                                        }))
                                        .child(icon("x").text_color(t.fg_2)),
                                ),
                        )
                        .when(failed, |d| {
                            d.child(
                                div()
                                    .pl(px(32.))
                                    .text_step(TEXT_XS)
                                    .text_color(t.danger_fg)
                                    .child(i.error.clone()),
                            )
                        })
                        .when(busy, |d| d)
                })),
        )
    }

    /// `.people`: 4 px over the box at its left, 320 px wide, 4 px padding, a `--line-subtle` border, `--radius-lg`,
    /// `--bg-raised`, `--shadow-float`, 13/20; 32 px rows of a 20 px picture, the name and @username; the highlighted
    /// one on `--line`. It fades in (`pop`).
    fn mention_list(
        &mut self,
        t: &Tokens,
        now: Instant,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let people = self.people();
        if people.is_empty() {
            return None;
        }
        let at = self.tl.shown_since("mentions".into(), now);
        let o = self.tl.pop(&t.motion, at, now);
        let active = self.mention_ix.min(people.len() - 1);
        let line = t.line;
        Some(
            div()
                .absolute()
                .left_0()
                .top(px(-4.))
                .child(
                    deferred(
                        anchored().anchor(Corner::BottomLeft).child(
                            div()
                                .debug_selector(|| "mentions".into())
                                .occlude()
                                .w(px(320.))
                                .max_h(px(296.))
                                .overflow_hidden()
                                .p(px(4.))
                                .border_1()
                                .border_color(t.line_subtle)
                                .rounded(t.radius_lg)
                                .bg(t.bg_raised)
                                .shadow(t.shadow_float.clone())
                                .text_step(TEXT_SM)
                                .opacity(o)
                                .children(people.into_iter().enumerate().map(|(i, u)| {
                                    let name = u.username.clone();
                                    let sel = format!("mention-{}", u.username);
                                    div()
                                        .id(ElementId::Name(sel.clone().into()))
                                        .debug_selector(move || sel)
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .h(px(32.))
                                        .px(px(8.))
                                        .rounded(t.radius)
                                        .cursor_pointer()
                                        .when(i == active, |d| d.bg(line))
                                        .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                                            if *on && this.mention_ix != i {
                                                this.mention_ix = i;
                                                cx.notify();
                                            }
                                        }))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.pick_mention(&name, window, cx)
                                        }))
                                        .child(Avatar::new(
                                            u.user_id.clone(),
                                            u.display_name.clone(),
                                            u.accent.clone(),
                                            20,
                                        ))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .ellipsis()
                                                .text_color(t.fg)
                                                .child(u.display_name.clone()),
                                        )
                                        .child(
                                            div()
                                                .min_w_0()
                                                .ellipsis()
                                                .text_color(t.fg_2)
                                                .child(format!("@{}", u.username)),
                                        )
                                })),
                        ),
                    )
                    .with_priority(1),
                )
                .into_any_element(),
        )
    }

    /// The formatting help over its button (`Popover` side top): 288 px, 8 px padding, the marks and what they make.
    fn help(&mut self, t: &Tokens, now: Instant, cx: &mut Context<Self>) -> AnyElement {
        let at = self.tl.shown_since("help".into(), now);
        let o = self.tl.pop(&t.motion, at, now);
        let code = |s: &'static str| {
            div()
                .flex_none()
                .px(px(4.))
                .rounded(t.radius_sm)
                .bg(t.bg_element)
                .font_family(t.font_mono.clone())
                .text_size(px(12.))
                .child(s)
        };
        let rows: [(&[&'static str], &'static str); 11] = [
            (&["**bold**"], "bold"),
            (&["*italic*"], "italic"),
            (&["~~strike~~"], "strike"),
            (&["`code`"], "code"),
            (&["```rust", "lines", "```"], "code block"),
            (&["# title"], "heading"),
            (&["> text"], "quote"),
            (&["- item", "1. item"], "list"),
            (&["---"], "rule"),
            (&["https://..."], "link"),
            (&["@name"], "mention"),
        ];
        div().absolute().left_0().top(px(-4.)).child(
            deferred(
                anchored().anchor(Corner::BottomLeft).snap_to_window_with_margin(px(8.)).child(
                    popover_frame(cx)
                        .id("help")
                        .debug_selector(|| "help".into())
                        .opacity(o)
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.tl.help = false;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(288.))
                                .p(px(8.))
                                .flex()
                                .flex_col()
                                .child(div().mb(px(4.)).font_weight(t.weight_strong).text_color(t.fg).child("Formatting"))
                                .children(rows.into_iter().map(|(marks, what)| {
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(16.))
                                        .mb(px(4.))
                                        .child(div().flex().items_center().gap(px(4.)).children(
                                            marks.iter().map(|s| {
                                                if *s == "lines" { div().child("lines") } else { code(s) }
                                            }),
                                        ))
                                        .child(div().text_color(t.fg_2).child(what))
                                }))
                                .child(div().mt(px(8.)).text_color(t.fg_2).child(
                                    "HTML and images show as typed. Enter sends, Shift+Enter adds a line, Up edits your last message.",
                                )),
                        ),
                ),
            )
            .with_priority(1),
        )
        .into_any_element()
    }

    /// The pins list under the header's Pins button (`PinsPopover.svelte`): "Pinned messages", then per pin a
    /// picture, the name and when, the text, and Unpin; "n of 50 pins" under them.
    pub(super) fn render_pins(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let t = tokens(cx).clone();
        let now = Instant::now();
        let at = self.tl.shown_since("pins".into(), now);
        let o = self.tl.pop(&t.motion, at, now);
        let m = &self.model;
        let (_, list) = m.pins.as_ref()?;
        let s = &m.state;
        let loc = Locale::system();
        let hover = t.bg_hover;
        let body: Vec<AnyElement> = match list {
            None => vec![div().px(px(8.)).pb(px(8.)).text_color(t.fg_2).child("Loading...").into_any_element()],
            Some(l) if l.is_empty() => vec![
                div()
                    .px(px(8.))
                    .pb(px(12.))
                    .child(div().text_color(t.fg).child("Nothing is pinned yet."))
                    .child(
                        div()
                            .text_step(TEXT_XS)
                            .text_color(t.fg_2)
                            .child(format!("Pin a message from its toolbar to keep it here. A room holds up to {MAX_PINS}.")),
                    )
                    .into_any_element(),
            ],
            Some(l) => {
                let rows = l.iter().map(|p| {
                    let id = p.id;
                    let u = s.users.get(&p.author_id);
                    let name = s.name_of(&p.author_id);
                    let text = plain(&parse(&p.body));
                    let text = if text.is_empty() { "A file".into() } else { text };
                    let when = pin_time(p.created_at, loc);
                    div()
                        .id(ElementId::Name(format!("pinned-{id}").into()))
                        .debug_selector(move || format!("pinned-{id}"))
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(t.radius)
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.model.pins = None;
                            if this.model.jump_to(id) {
                                this.tl.flash = None;
                            }
                            cx.notify();
                        }))
                        .child(Avatar::new(
                            p.author_id.clone(),
                            name.clone(),
                            u.map_or(String::new(), |u| u.accent.clone()),
                            20,
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex()
                                        .items_start()
                                        .gap(px(8.))
                                        .child(div().min_w_0().ellipsis().font_weight(t.weight_strong).text_color(t.fg).child(name))
                                        .child(
                                            div()
                                                .flex_none()
                                                .mt(px(2.))
                                                .text_step(TEXT_XS)
                                                .text_color(t.fg_2)
                                                .child(when),
                                        ),
                                )
                                .child(div().ellipsis().text_color(t.fg_2).child(text)),
                        )
                        .child(
                            c::Button::new(ElementId::Name(format!("unpin-{id}").into()), "Unpin")
                                .small()
                                .variant(c::Variant::Ghost)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let cmds = this.model.pin(id);
                                    if let Some((_, Some(l))) = this.model.pins.as_mut() {
                                        l.retain(|x| x.id != id);
                                    }
                                    this.run(cmds);
                                    cx.notify();
                                })),
                        )
                        .into_any_element()
                });
                let mut v: Vec<AnyElement> = rows.collect();
                v.push(
                    div()
                        .px(px(8.))
                        .py(px(4.))
                        .text_step(TEXT_XS)
                        .text_color(t.fg_2)
                        .flex()
                        // The count rolls its digits (`Roll.svelte`) when a pin goes.
                        .child(motion::roll("pins-count", &l.len().to_string(), px(16.), false, window, cx))
                        .child(format!(" of {MAX_PINS} pins"))
                        .into_any_element(),
                );
                v
            }
        };
        Some(
            div()
                .absolute()
                .left_0()
                .top(px(32.))
                .child(
                    deferred(
                        anchored()
                            .anchor(Corner::TopLeft)
                            .snap_to_window_with_margin(px(8.))
                            .child(
                                div().pt(px(4.)).child(
                                    popover_frame(cx)
                                        .id("pins-panel")
                                        .debug_selector(|| "pins-panel".into())
                                        .max_w(px(400.))
                                        .opacity(o)
                                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                            this.model.pins = None;
                                            cx.notify();
                                        }))
                                        .child(
                                            div()
                                                .w(px(312.))
                                                .flex()
                                                .flex_col()
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .h(px(32.))
                                                        .px(px(8.))
                                                        .font_weight(t.weight_strong)
                                                        .text_color(t.fg)
                                                        .child("Pinned messages"),
                                                )
                                                .children(body),
                                        ),
                                ),
                            ),
                    )
                    .with_priority(1),
                )
                .into_any_element(),
        )
    }

    /// The delete dialog (`Dialog.svelte`), over the whole window.
    pub(super) fn delete_dialog(&self, cx: &mut Context<Self>) -> Option<Div> {
        let id = self.model.confirm_delete?;
        let cancel = cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| {
            this.model.cancel_delete();
            cx.notify();
        });
        Some(
            div()
                .absolute()
                .inset_0()
                .track_focus(&self.dialog_focus)
                .key_context(super::DIALOG)
                .debug_selector(move || format!("confirm-{id}"))
                .child(
                    c::Dialog::new("Delete this message?")
                        .child("It is removed for everyone in the room. This cannot be undone.")
                        .footer(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(
                                    div().debug_selector(|| "confirm-cancel".into()).child(
                                        c::Button::new("confirm-cancel", "Cancel")
                                            .variant(c::Variant::Ghost)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.model.cancel_delete();
                                                cx.notify();
                                            })),
                                    ),
                                )
                                .child(
                                    div().debug_selector(|| "confirm-delete".into()).child(
                                        c::Button::new("confirm-delete", "Delete")
                                            .variant(c::Variant::Danger)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.act(|m| m.delete(id, true), cx)
                                            })),
                                    ),
                                ),
                        )
                        .on_close(cancel),
                ),
        )
    }
}

/// A decoded picture as GPUI draws it.
/// Opens a file in the system's app. On Linux through `open_url`, which first asks the compositor for an activation
/// token, so the app comes up in front: `open_with_system` runs xdg-open without one, and the desktop may keep the
/// new window behind Vesper.
pub(super) fn open_file(path: &std::path::Path, cx: &mut App) {
    #[cfg(target_os = "linux")]
    if let Ok(u) = url::Url::from_file_path(path) {
        return cx.open_url(u.as_str());
    }
    cx.open_with_system(path);
}

pub(super) fn render_image(p: Picture) -> Option<Arc<RenderImage>> {
    if p.delays.is_empty() {
        let buf = image::RgbaImage::from_raw(p.width, p.height, p.bgra)?;
        return Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])));
    }
    let len = (p.width * p.height * 4) as usize;
    let frames = p
        .bgra
        .chunks_exact(len)
        .zip(&p.delays)
        .map(|(px, &ms)| {
            let buf = image::RgbaImage::from_raw(p.width, p.height, px.to_vec())?;
            let delay = image::Delay::from_numer_denom_ms(u32::from(ms), 1);
            Some(image::Frame::from_parts(buf, 0, 0, delay))
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Arc::new(RenderImage::new(frames)))
}

/// A picture `w` x `h`. An animated one plays (GPUI keeps the frame in the element's state, by `id`); under reduced
/// motion it has no id, so it stays on its first frame.
pub(super) fn picture(i: &Arc<RenderImage>, id: String, w: f32, h: f32, cx: &App) -> AnyElement {
    let el = img(ImageSource::Render(i.clone())).w(px(w)).h(px(h));
    if i.frame_count() > 1 && !tokens(cx).motion.reduced {
        el.id(SharedString::from(id)).into_any_element()
    } else {
        el.into_any_element()
    }
}

/// `.attach`, `.send` and the help IconButton: 32 px, `--radius`, a `--bg-hover` fill on hover in `--dur-1`; Attach
/// and help go from `--fg-2` to `--fg`, Send stays in its colour (`--accent-fg`, or `--fg-disabled` with no fill).
fn composer_icon(
    id: &'static str,
    name: &'static str,
    fg: Hsla,
    live: bool,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<Div> {
    let (h, on_hover) = motion::hover(id, window, cx);
    let t = tokens(cx);
    let h = if live { h } else { 0. };
    let fg = if name == "send" {
        fg
    } else {
        mix(fg, t.fg.into(), h)
    };
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(32.))
        .rounded(t.radius)
        .bg(mix(transparent_black(), t.bg_hover.into(), h))
        .when(live, |d| d.cursor_pointer())
        .on_hover(on_hover)
        .child(icon(name).text_color(fg))
}

/// The picture a paste attaches: only a copy with no text, as the web takes files only when the copy is one (a page's
/// text with pictures in it pastes as text).
fn pasted_image(entries: &[ClipboardEntry]) -> Option<&gpui::Image> {
    let text = entries
        .iter()
        .any(|e| matches!(e, ClipboardEntry::String(s) if !s.text().trim().is_empty()));
    entries.iter().find_map(|e| match e {
        ClipboardEntry::Image(i) if !text => Some(i),
        _ => None,
    })
}

/// The composer's limit and where its count shows (the server's limit, in characters).
const MAX_CHARS: usize = 4000;
const COUNT_FROM: usize = 3600;
const OFFLINE: &str =
    "You are offline. Your text is kept here, and Send works again once the connection is back.";

/// A pin's time (`toLocaleString` with day, month, hour and minute): "Nov 13, 7:41 PM" in en-US, "13 Nov, 19:41".
fn pin_time(ms: i64, loc: Locale) -> String {
    use chrono::{TimeZone, Timelike};
    let Some(d) = Local.timestamp_millis_opt(ms).single() else {
        return String::new();
    };
    let day = if loc.month_first {
        d.format("%b %-d")
    } else {
        d.format("%-d %b")
    };
    // Not the pinned clock: the web harness fixes only formats without a day.
    format!(
        "{day}, {}",
        crate::timeline::hm(d.hour(), d.minute(), loc, false)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_with_text_is_text() {
        let image = gpui::Image::from_bytes(ImageFormat::Png, vec![1, 2, 3]);
        let both = [
            ClipboardEntry::from("some words".to_string()),
            ClipboardEntry::Image(image.clone()),
        ];
        assert!(pasted_image(&both).is_none());
        let picture = [ClipboardEntry::Image(image.clone())];
        assert_eq!(pasted_image(&picture), Some(&image));
        let blank = [
            ClipboardEntry::from(" ".to_string()),
            ClipboardEntry::Image(image.clone()),
        ];
        assert_eq!(pasted_image(&blank), Some(&image), "no real text");
    }
}
