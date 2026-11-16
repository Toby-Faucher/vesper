//! Draws a parsed message (`crate::markdown`) like `Markdown.svelte` and `CodeBlock.svelte`: each run of inline nodes
//! is one `StyledText` built from text runs (so inline code can be monospace), each block its own element. Text is only
//! ever drawn as text. Links open in the system browser; the parser lets only http and https through.

use std::ops::Range;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, ElementId, FontStyle, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, RenderOnce, StatefulInteractiveElement, StrikethroughStyle, Styled, TextRun,
    UnderlineStyle, Window, div, font, px, transparent_black,
};

use super::c::mix;
use super::icons::icon;
use super::motion;
use super::select::SelText;
use super::theme::{TEXT_BASE, TEXT_LG, TEXT_SM, TEXT_XS, TextStepExt, Tokens};
use crate::highlight::{Kind, highlight, lang_of};
use crate::markdown::Md;

pub type OnCopy = Rc<dyn Fn(usize, &str, &mut Window, &mut App)>;
type OnPress = Rc<dyn Fn(&mut Window, &mut App)>;

/// What drawing needs besides the tree.
pub struct Ctx {
    /// The reader's username: a mention of them is drawn in amber.
    pub me: String,
    /// Every known username, lower case: a mention of one is the web's PersonButton.
    pub known: std::collections::HashSet<String>,
    pub t: Tokens,
    /// The message id, for element ids.
    pub id: i64,
    /// The code block (its number in the message) whose button reads Copied.
    pub copied: Option<usize>,
    /// A code block's Copy: its number and its text.
    pub on_copy: OnCopy,
}

/// How a run of text looks.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    color: Hsla,
    bg: Option<Hsla>,
    weight: FontWeight,
    italic: bool,
    strike: bool,
    underline: bool,
    mono: bool,
}

#[derive(Default)]
struct Flat {
    text: String,
    runs: Vec<(usize, Look)>,
    links: Vec<(Range<usize>, String)>,
}

impl Flat {
    fn push(&mut self, v: &str, look: Look) -> Range<usize> {
        let r = self.text.len()..self.text.len() + v.len();
        self.text.push_str(v);
        match self.runs.last_mut() {
            Some(last) if last.1 == look => last.0 += v.len(),
            _ if v.is_empty() => {}
            _ => self.runs.push((v.len(), look)),
        }
        r
    }
}

// shortcut: a text run has no padding, so the 4 px of inline code and the 2 px of a mention are a four-per-em and a
// hair space inside the run's background. Upgrade if GPUI text runs get padding.
const CODE_PAD: &str = "\u{2005}";
const MENTION_PAD: &str = "\u{200a}";

fn flat(nodes: &[Md], look: Look, f: &mut Flat, c: &Ctx) {
    let t = &c.t;
    for n in nodes {
        match n {
            Md::Text(v) => drop(f.push(v, look)),
            Md::Br => drop(f.push("\n", look)),
            // `font-medium text-fg`
            Md::B(k) => flat(
                k,
                Look {
                    weight: t.weight_strong,
                    color: t.fg.into(),
                    ..look
                },
                f,
                c,
            ),
            Md::I(k) => flat(
                k,
                Look {
                    italic: true,
                    ..look
                },
                f,
                c,
            ),
            Md::S(k) => flat(
                k,
                Look {
                    strike: true,
                    ..look
                },
                f,
                c,
            ),
            // `rounded-sm bg-element px-1 font-mono`
            Md::Code(v) => {
                let code = Look {
                    bg: Some(t.bg_element.into()),
                    mono: true,
                    ..look
                };
                f.push(&format!("{CODE_PAD}{v}{CODE_PAD}"), code);
            }
            // `text-accent-fg underline`
            Md::Link(href) => {
                let r = f.push(
                    href,
                    Look {
                        color: t.accent_fg.into(),
                        underline: true,
                        ..look
                    },
                );
                f.links.push((r, href.clone()));
            }
            // `rounded-sm px-0.5 font-medium`, amber for me, `--bg-selected` for others. A known name is a
            // PersonButton, whose `font: inherit; color: inherit; padding: 0` wins over those classes: only the fill
            // shows.
            Md::Mention(u) => {
                // @everyone and @here reach the reader too: drawn as a mention of them, as on the web.
                let mine = u.eq_ignore_ascii_case(&c.me)
                    || u.eq_ignore_ascii_case("everyone")
                    || u.eq_ignore_ascii_case("here");
                let (bg, fg) = if mine {
                    (t.accent_subtle, t.accent_fg)
                } else {
                    (t.bg_selected, t.fg)
                };
                let known = c.known.contains(&u.to_lowercase());
                let pad = if known { "" } else { MENTION_PAD };
                f.push(
                    &format!("{pad}@{u}{pad}"),
                    Look {
                        bg: Some(bg.into()),
                        color: if known { look.color } else { fg.into() },
                        weight: if known { look.weight } else { t.weight_strong },
                        ..look
                    },
                );
            }
            // Blocks never sit in an inline run.
            _ => {}
        }
    }
}

fn runs(f: &Flat, t: &Tokens) -> Vec<TextRun> {
    f.runs
        .iter()
        .map(|(len, l)| {
            let mut fo = font(if l.mono {
                t.font_mono.clone()
            } else {
                t.font_sans.clone()
            });
            fo.weight = l.weight;
            if l.italic {
                fo.style = FontStyle::Italic;
            }
            TextRun {
                len: *len,
                font: fo,
                color: l.color,
                background_color: l.bg,
                underline: l.underline.then_some(UnderlineStyle {
                    thickness: px(1.),
                    color: None,
                    wavy: false,
                }),
                strikethrough: l.strike.then_some(StrikethroughStyle {
                    thickness: px(1.),
                    color: None,
                }),
            }
        })
        .collect()
}

fn base(color: impl Into<Hsla>) -> Look {
    Look {
        color: color.into(),
        bg: None,
        weight: FontWeight::NORMAL,
        italic: false,
        strike: false,
        underline: false,
        mono: false,
    }
}

/// One run of inline nodes as text, its links clickable.
fn paragraph(nodes: &[Md], color: Hsla, c: &Ctx, n: &mut usize) -> AnyElement {
    let mut f = Flat::default();
    flat(nodes, base(color), &mut f, c);
    *n += 1;
    let name = format!("md-{}-{n}", c.id);
    let el = div().debug_selector({
        let name = name.clone();
        move || name
    });
    let r = runs(&f, &c.t);
    let (ranges, hrefs): (Vec<_>, Vec<_>) = f.links.into_iter().unzip();
    el.child(
        SelText::new(name, f.text)
            .with_runs(r)
            .on_link(ranges, move |ix, _, cx| cx.open_url(&hrefs[ix])),
    )
    .into_any_element()
}

/// `CodeBlock.svelte`'s Copy button: 24 px, 6 px sides, 4 px gap, `--radius-sm`, 12/16 `--fg-2`, the hover fill and
/// colour in `--dur-1`. It reads "Copied" with a check for 2 s after a copy.
#[derive(IntoElement)]
struct CopyButton {
    id: ElementId,
    copied: bool,
    on_click: OnPress,
}

impl RenderOnce for CopyButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (h, on_hover) = motion::hover(self.id.clone(), window, cx);
        let t = super::theme::tokens(cx);
        let fg = mix(t.fg_2.into(), t.fg.into(), h);
        let f = self.on_click;
        div()
            .id(self.id)
            .flex()
            .items_center()
            .gap(px(4.))
            .h(px(24.))
            .px(px(6.))
            .rounded(t.radius_sm)
            .bg(mix(transparent_black(), t.bg_hover.into(), h))
            .text_color(fg)
            .text_step(TEXT_XS)
            .cursor_pointer()
            .on_hover(on_hover)
            .on_click(move |_, w, cx| f(w, cx))
            .child(icon(if self.copied { "check" } else { "copy" }).text_color(fg))
            .child(if self.copied { "Copied" } else { "Copy" })
    }
}

/// `CodeBlock.svelte`: a 28 px bar with the language (mono 12/16 `--fg-3`) and Copy, then the code in mono 13/20 on
/// `--bg-element` with the `--syn-*` colours, one line per line (a long line is cut, not scrolled).
fn code_block(v: &str, lang: Option<&String>, c: &Ctx, n: &mut usize) -> AnyElement {
    let t = &c.t;
    *n += 1;
    let block = *n;
    let toks = match lang_of(lang.map(String::as_str)) {
        Some(l) => highlight(v, l),
        None => vec![(Kind::Plain, v.to_string())],
    };
    let mut text = String::new();
    let mut r = vec![];
    for (k, s) in toks {
        let color: Hsla = match k {
            Kind::Kw => t.syn_kw,
            Kind::Str => t.syn_str,
            Kind::Num => t.syn_num,
            Kind::Ty => t.syn_ty,
            Kind::Com => t.syn_com,
            _ => t.fg,
        }
        .into();
        let mut fo = font(t.font_mono.clone());
        if k == Kind::Com {
            fo.style = FontStyle::Italic;
        }
        text.push_str(&s);
        r.push(TextRun {
            len: s.len(),
            font: fo,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        });
    }
    let on_copy = c.on_copy.clone();
    let code = v.to_string();
    let id = c.id;
    div()
        .debug_selector(move || format!("code-{id}-{block}"))
        .my(px(4.))
        .border_1()
        .border_color(t.line_subtle)
        .rounded(t.radius)
        .bg(t.bg_element)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .h(px(28.))
                .pl(px(12.))
                .pr(px(4.))
                .border_b_1()
                .border_color(t.line_subtle)
                .child(
                    div()
                        .font_family(t.font_mono.clone())
                        .text_step(TEXT_XS)
                        .text_color(t.fg_3)
                        .child(lang.cloned().unwrap_or_default()),
                )
                .child(CopyButton {
                    id: ElementId::Name(format!("copy-{id}-{block}").into()),
                    copied: c.copied == Some(block),
                    on_click: Rc::new(move |w, cx| on_copy(block, &code, w, cx)),
                }),
        )
        .child(
            div()
                .overflow_hidden()
                .px(px(12.))
                .py(px(8.))
                .text_size(px(13.))
                .line_height(px(20.))
                .whitespace_nowrap()
                .child(SelText::new(format!("code-{id}-{block}"), text).with_runs(r)),
        )
        .into_any_element()
}

fn block(node: &Md, color: Hsla, c: &Ctx, n: &mut usize, depth: usize) -> AnyElement {
    let t = &c.t;
    match node {
        // `border-l-2 border-line pl-3 text-fg-2`
        Md::Quote(k) => div()
            .pl(px(12.))
            .border_l_2()
            .border_color(t.line)
            .child(paragraph(k, t.fg_2.into(), c, n))
            .into_any_element(),
        // 20/28, 16/24, 14/20, `--weight-strong`, 4 px over and 2 under.
        Md::H(level, k) => div()
            .mt(px(4.))
            .mb(px(2.))
            .text_step(match level {
                1 => TEXT_LG,
                2 => TEXT_BASE,
                _ => TEXT_SM,
            })
            .when(*level > 2, |d| d.text_size(px(14.)).line_height(px(20.)))
            .font_weight(t.weight_strong)
            .child({
                let mut f = Flat::default();
                let look = Look {
                    weight: t.weight_strong,
                    ..base(t.fg)
                };
                flat(k, look, &mut f, c);
                *n += 1;
                let name = format!("md-{}-{n}", c.id);
                let r = runs(&f, t);
                div()
                    .debug_selector({
                        let name = name.clone();
                        move || name
                    })
                    .child(SelText::new(name, f.text).with_runs(r))
            })
            .into_any_element(),
        // `my-2 border-line`
        Md::Hr => div().my(px(8.)).h(px(1.)).bg(t.line).into_any_element(),
        Md::Pre(v, lang) => code_block(v, lang.as_ref(), c, n),
        // `padding-left: 20px; margin: 2px 0`, the marker in `--fg-3`: a disc, a circle inside a list, or the number.
        Md::Ul(items) | Md::Ol(_, items) => {
            let start = if let Md::Ol(s, _) = node {
                Some(*s)
            } else {
                None
            };
            div()
                .flex()
                .flex_col()
                .my(px(2.))
                .children(items.iter().enumerate().map(|(i, li)| {
                    let marker = match start {
                        Some(s) => format!("{}.", u64::from(s) + i as u64),
                        None if depth == 0 => "\u{2022}".into(),
                        None => "\u{25e6}".into(),
                    };
                    let kids = if let Md::Li(k) = li { &k[..] } else { &[] };
                    div()
                        .flex()
                        .items_start()
                        .child(
                            div()
                                .w(px(20.))
                                .flex_none()
                                .flex()
                                .justify_end()
                                .pr(px(6.))
                                .text_color(t.fg_3)
                                .child(marker),
                        )
                        .child(div().flex().flex_col().flex_1().min_w_0().children(blocks(
                            kids,
                            color,
                            c,
                            n,
                            depth + 1,
                        )))
                }))
                .into_any_element()
        }
        // Inline nodes go through `paragraph`.
        _ => paragraph(std::slice::from_ref(node), color, c, n),
    }
}

fn blocks(nodes: &[Md], color: Hsla, c: &Ctx, n: &mut usize, depth: usize) -> Vec<AnyElement> {
    let mut out = vec![];
    let mut start = 0;
    for (i, node) in nodes.iter().enumerate() {
        let inline = matches!(
            node,
            Md::Text(_)
                | Md::Br
                | Md::B(_)
                | Md::I(_)
                | Md::S(_)
                | Md::Code(_)
                | Md::Link(_)
                | Md::Mention(_)
        );
        if inline {
            continue;
        }
        if start < i {
            out.push(paragraph(&nodes[start..i], color, c, n));
        }
        out.push(block(node, color, c, n, depth));
        start = i + 1;
    }
    if start < nodes.len() {
        out.push(paragraph(&nodes[start..], color, c, n));
    }
    out
}

/// A parsed message as elements, top to bottom, in `--fg`. The last element is a paragraph when the message ends in
/// text (the (edited) note follows it on its line).
pub fn render(nodes: &[Md], c: &Ctx) -> Vec<AnyElement> {
    blocks(nodes, c.t.fg.into(), c, &mut 0, 0)
}

/// Whether the message ends in a run of text rather than a block.
pub fn ends_inline(nodes: &[Md]) -> bool {
    nodes.last().is_some_and(|n| {
        matches!(
            n,
            Md::Text(_)
                | Md::Br
                | Md::B(_)
                | Md::I(_)
                | Md::S(_)
                | Md::Code(_)
                | Md::Link(_)
                | Md::Mention(_)
        )
    })
}
