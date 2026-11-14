//! Draws a parsed message (`crate::markdown`) with GPUI text styles: each run of inline nodes is one `StyledText`, each
//! block its own element. Text is only ever drawn as text. Links open in the system browser; the parser lets only
//! http and https through.

use std::ops::Range;

use gpui::{
    AnyElement, ElementId, FontStyle, FontWeight, HighlightStyle, InteractiveElement,
    InteractiveText, IntoElement, ParentElement, SharedString, StrikethroughStyle, Styled,
    StyledText, UnderlineStyle, div, px, rgb,
};
use gpui_component::{h_flex, v_flex};

use super::theme;
use crate::markdown::Md;

/// What drawing needs besides the tree.
pub struct Ctx {
    /// The reader's username: a mention of them is drawn in amber.
    pub me: String,
    pub mono: SharedString,
    /// The message id, for element ids.
    pub id: i64,
}

#[derive(Default)]
struct Flat {
    text: String,
    hl: Vec<(Range<usize>, HighlightStyle)>,
    links: Vec<(Range<usize>, String)>,
}

impl Flat {
    fn push(&mut self, v: &str, style: HighlightStyle) -> Range<usize> {
        let r = self.text.len()..self.text.len() + v.len();
        self.text.push_str(v);
        if style != HighlightStyle::default() && !v.is_empty() {
            self.hl.push((r.clone(), style));
        }
        r
    }
}

fn flat(nodes: &[Md], style: HighlightStyle, f: &mut Flat, c: &Ctx) {
    for n in nodes {
        match n {
            Md::Text(v) => drop(f.push(v, style)),
            Md::Br => drop(f.push("\n", style)),
            Md::B(k) => {
                let s = HighlightStyle {
                    font_weight: Some(FontWeight::SEMIBOLD),
                    color: Some(rgb(theme::FG).into()),
                    ..style
                };
                flat(k, s, f, c);
            }
            Md::I(k) => {
                let s = HighlightStyle {
                    font_style: Some(FontStyle::Italic),
                    ..style
                };
                flat(k, s, f, c);
            }
            Md::S(k) => {
                let s = HighlightStyle {
                    strikethrough: Some(StrikethroughStyle {
                        thickness: px(1.),
                        color: None,
                    }),
                    ..style
                };
                flat(k, s, f, c);
            }
            Md::Code(v) => drop(f.push(
                v,
                HighlightStyle {
                    background_color: Some(rgb(theme::BG_SELECTED).into()),
                    color: Some(rgb(theme::FG).into()),
                    ..style
                },
            )),
            Md::Link(href) => {
                let s = HighlightStyle {
                    color: Some(rgb(theme::ACCENT_FG).into()),
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    ..style
                };
                let r = f.push(href, s);
                f.links.push((r, href.clone()));
            }
            Md::Mention(u) => {
                let mine = u.eq_ignore_ascii_case(&c.me);
                let (bg, fg) = if mine {
                    (theme::ACCENT_SUBTLE, theme::ACCENT_FG)
                } else {
                    (theme::BG_SELECTED, theme::FG)
                };
                let s = HighlightStyle {
                    background_color: Some(rgb(bg).into()),
                    color: Some(rgb(fg).into()),
                    font_weight: Some(FontWeight::SEMIBOLD),
                    ..style
                };
                f.push(&format!("@{u}"), s);
            }
            // Blocks never sit in an inline run.
            _ => {}
        }
    }
}

/// One run of inline nodes as text, its links clickable.
fn paragraph(nodes: &[Md], c: &Ctx, n: &mut usize) -> AnyElement {
    let mut f = Flat::default();
    flat(nodes, HighlightStyle::default(), &mut f, c);
    *n += 1;
    let name = format!("md-{}-{n}", c.id);
    let el = div().debug_selector({
        let name = name.clone();
        move || name
    });
    let styled = StyledText::new(f.text).with_highlights(f.hl);
    if f.links.is_empty() {
        return el.child(styled).into_any_element();
    }
    let (ranges, hrefs): (Vec<_>, Vec<_>) = f.links.into_iter().unzip();
    el.child(
        InteractiveText::new(ElementId::Name(name.into()), styled)
            .on_click(ranges, move |ix, _, cx| cx.open_url(&hrefs[ix])),
    )
    .into_any_element()
}

fn block(node: &Md, c: &Ctx, n: &mut usize, depth: usize) -> AnyElement {
    match node {
        Md::Quote(k) => div()
            .pl_3()
            .border_l_2()
            .border_color(rgb(theme::LINE))
            .text_color(rgb(theme::FG_2))
            .child(paragraph(k, c, n))
            .into_any_element(),
        Md::H(level, k) => div()
            .mt_1()
            .text_size(px(match level {
                1 => 20.,
                2 => 16.,
                _ => 14.,
            }))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(theme::FG))
            .child(paragraph(k, c, n))
            .into_any_element(),
        Md::Hr => div()
            .my_2()
            .h(px(1.))
            .bg(rgb(theme::LINE))
            .into_any_element(),
        Md::Pre(v, lang) => v_flex()
            .my_1()
            .p_2()
            .rounded(px(6.))
            .bg(rgb(theme::BG_RAISED))
            .border_1()
            .border_color(rgb(theme::LINE_SUBTLE))
            .children(lang.as_ref().map(|l| {
                div()
                    .text_size(px(12.))
                    .text_color(rgb(theme::FG_3))
                    .child(l.clone())
            }))
            .child(
                div()
                    .font_family(c.mono.clone())
                    .text_size(px(13.))
                    .child(v.clone()),
            )
            .into_any_element(),
        Md::Ul(items) | Md::Ol(_, items) => {
            let start = if let Md::Ol(s, _) = node {
                Some(*s)
            } else {
                None
            };
            v_flex()
                .children(items.iter().enumerate().map(|(i, li)| {
                    let marker = match start {
                        Some(s) => format!("{}.", u64::from(s) + i as u64),
                        None if depth == 0 => "\u{2022}".into(),
                        None => "\u{25e6}".into(),
                    };
                    let kids = if let Md::Li(k) = li { &k[..] } else { &[] };
                    h_flex()
                        .items_start()
                        .gap_1p5()
                        .child(div().text_color(rgb(theme::FG_3)).child(marker))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .children(blocks(kids, c, n, depth + 1)),
                        )
                }))
                .into_any_element()
        }
        // Inline nodes go through `paragraph`.
        _ => paragraph(std::slice::from_ref(node), c, n),
    }
}

fn blocks(nodes: &[Md], c: &Ctx, n: &mut usize, depth: usize) -> Vec<AnyElement> {
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
            out.push(paragraph(&nodes[start..i], c, n));
        }
        out.push(block(node, c, n, depth));
        start = i + 1;
    }
    if start < nodes.len() {
        out.push(paragraph(&nodes[start..], c, n));
    }
    out
}

/// A parsed message as elements, top to bottom.
pub fn render(nodes: &[Md], c: &Ctx) -> Vec<AnyElement> {
    blocks(nodes, c, &mut 0, 0)
}
