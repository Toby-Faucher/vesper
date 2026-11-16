//! Selectable text, as a browser selects a page's text: a drag selects, also across texts and messages, a double
//! click selects a word, a triple click a whole text, Shift extends, Ctrl+A (Cmd+A) selects every loaded message
//! (`mod.rs`) and Ctrl+C (Cmd+C) copies. It is drawn in `--selection`, the accent at 30 %, over the text. A click (a
//! press and release with no drag) on a link opens it.
//!
//! Each `SelText` has a `Key` that orders it in the document: (scope, row, part); in the chat the row is the message
//! id. The selection is two places, a key and a byte offset in that text, and each text paints the part of the range
//! it holds. Each text drawn records its text and bounds (`laid`, cleared when the root view renders). A drag puts the
//! head on the text under the pointer, or the nearest one, so it runs over gaps and messages, and the chat list
//! scrolls when it passes the list's edge (`chat.rs`). A copy takes the drawn texts, and in the chat the model's
//! (`chat_texts`), so rows scrolled away are copied too: one line per text.

use std::collections::BTreeMap;
use std::ops::{Range, RangeInclusive};
use std::rc::Rc;

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, GlobalElementId, HighlightStyle,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, SharedString, StyledText, TextLayout, TextRun,
    Window, fill, point,
};

use super::theme::tokens;

/// Where a text sits in the document: its scope (a selection stays in its scope), its row and its part in the row.
pub type Key = (u8, i64, u32);

/// The chat's messages: the row is the message id.
pub const CHAT: u8 = 0;
/// The pinned messages panel: the row is the message id.
pub const PINS: u8 = 1;
/// The search results: the row is the hit's number.
pub const FIND: u8 = 2;

/// A place in the selectable text: which text, and a byte offset in it.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Pos {
    key: Key,
    ix: usize,
}

/// A text drawn in the last frame: its text and where it showed.
struct Laid {
    text: SharedString,
    bounds: Bounds<Pixels>,
    layout: TextLayout,
}

/// The selection: where the press was (`anchor`) and where it is now (`head`), and the texts drawn last frame.
#[derive(Default)]
pub struct Selection {
    anchor: Pos,
    head: Pos,
    dragging: bool,
    laid: BTreeMap<Key, Laid>,
}

impl gpui::Global for Selection {}

/// What a press on selectable text focuses (the root view's), so Ctrl+C reaches the copy there and a field's own
/// Ctrl+C does not take it.
pub struct TextFocus(pub gpui::FocusHandle);

impl gpui::Global for TextFocus {}

/// Every key of `scope`.
fn scope_keys(scope: u8) -> RangeInclusive<Key> {
    (scope, i64::MIN, 0)..=(scope, i64::MAX, u32::MAX)
}

impl Selection {
    fn ordered(&self) -> (Pos, Pos) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    /// The bytes of the text at `key` (`len` long) that are selected.
    fn part(&self, key: Key, len: usize) -> Option<Range<usize>> {
        let (a, b) = self.ordered();
        if a == b || key < a.key || key > b.key {
            return None;
        }
        let start = if key == a.key { a.ix.min(len) } else { 0 };
        let end = if key == b.key { b.ix.min(len) } else { len };
        (start < end).then_some(start..end)
    }

    /// The scope and the rows the selection runs over, `None` when nothing is selected.
    pub fn rows(cx: &App) -> Option<(u8, RangeInclusive<i64>)> {
        let s = cx.try_global::<Selection>()?;
        let (a, b) = s.ordered();
        (a != b).then_some((a.key.0, a.key.1..=b.key.1))
    }

    /// The selected text, out of the texts drawn last frame and `more` (the model's, in the chat), which wins on the
    /// same key. One line per text; the thin spaces that pad inline code and mentions are left out (`md.rs`).
    pub fn selected(cx: &App, more: Vec<(Key, SharedString)>) -> Option<String> {
        let s = cx.try_global::<Selection>()?;
        let mut texts: BTreeMap<Key, SharedString> =
            s.laid.iter().map(|(k, l)| (*k, l.text.clone())).collect();
        texts.extend(more);
        let parts: Vec<&str> = texts
            .iter()
            .filter_map(|(k, t)| t.get(s.part(*k, t.len())?))
            .collect();
        (!parts.is_empty()).then(|| parts.join("\n").replace(['\u{2005}', '\u{200a}'], ""))
    }

    /// The texts drawn last frame.
    #[cfg(test)]
    pub fn drawn(cx: &App) -> Vec<(Key, SharedString)> {
        cx.try_global::<Selection>().map_or(vec![], |s| {
            s.laid.iter().map(|(k, l)| (*k, l.text.clone())).collect()
        })
    }

    /// Something is selected (so a click that made it is not also a click on the button under it).
    pub fn active(cx: &App) -> bool {
        cx.try_global::<Selection>()
            .is_some_and(|s| s.anchor != s.head)
    }

    /// Selects every text of `scope`, those not drawn too.
    pub fn select_all(scope: u8, cx: &mut App) {
        let s = cx.default_global::<Selection>();
        let keys = scope_keys(scope);
        s.anchor = Pos {
            key: *keys.start(),
            ix: 0,
        };
        s.head = Pos {
            key: *keys.end(),
            ix: usize::MAX,
        };
        s.dragging = false;
    }

    /// A drag that selects goes on in `scope`.
    pub fn dragging(scope: u8, cx: &App) -> bool {
        cx.try_global::<Selection>()
            .is_some_and(|s| s.dragging && s.anchor.key.0 == scope)
    }

    /// The root view renders: the texts are drawn again.
    pub fn new_frame(cx: &mut App) {
        cx.default_global::<Selection>().laid.clear();
    }

    /// While a drag goes on, moves the head to `p`.
    pub fn drag(p: Point<Pixels>, cx: &mut App) {
        let s = cx.default_global::<Selection>();
        if s.dragging
            && let Some(h) = s.hit(p)
        {
            s.head = h;
        }
    }

    /// The place at `p` among the texts of the anchor's scope drawn last frame: in the text level with `p` (the
    /// nearest across), else at the end of the last text above `p`, else at the start of the first.
    fn hit(&self, p: Point<Pixels>) -> Option<Pos> {
        let (mut level, mut above, mut first) = (None::<(Pixels, Key)>, None, None);
        for (k, l) in self.laid.range(scope_keys(self.anchor.key.0)) {
            let b = l.bounds;
            if b.size.width <= Pixels::ZERO || b.size.height <= Pixels::ZERO {
                continue;
            }
            first.get_or_insert(*k);
            if p.y >= b.top() && p.y < b.bottom() {
                let dx = (b.left() - p.x).max(p.x - b.right()).max(Pixels::ZERO);
                if level.is_none_or(|(d, _)| dx < d) {
                    level = Some((dx, *k));
                }
            } else if b.top() <= p.y {
                above = Some(*k);
            }
        }
        if let Some((_, key)) = level {
            let ix = self.laid[&key]
                .layout
                .index_for_position(p)
                .unwrap_or_else(|i| i);
            return Some(Pos { key, ix });
        }
        if let Some(key) = above {
            let ix = self.laid[&key].text.len();
            return Some(Pos { key, ix });
        }
        first.map(|key| Pos { key, ix: 0 })
    }
}

/// The word around byte `ix` of `text`: letters, digits and `_`, or else the one character there.
pub fn word_at(text: &str, ix: usize) -> Range<usize> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let ix = ix.min(text.len());
    let start = text[..ix]
        .char_indices()
        .rev()
        .take_while(|(_, c)| word(*c))
        .last()
        .map_or(ix, |(i, _)| i);
    let end = text[ix..]
        .char_indices()
        .find(|(_, c)| !word(*c))
        .map_or(text.len(), |(i, _)| ix + i);
    if start == end {
        let next = text[ix..].chars().next().map_or(0, char::len_utf8);
        return ix..ix + next;
    }
    start..end
}

type OnLink = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Text that can be selected. `key` places it in the document; keep it stable across frames and unique.
pub struct SelText {
    key: Key,
    text: SharedString,
    styled: StyledText,
    links: Vec<Range<usize>>,
    on_link: Option<OnLink>,
    cursor: CursorStyle,
}

impl SelText {
    pub fn new(key: Key, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        SelText {
            key,
            styled: StyledText::new(text.clone()),
            text,
            links: vec![],
            on_link: None,
            cursor: CursorStyle::IBeam,
        }
    }

    pub fn with_runs(mut self, runs: Vec<TextRun>) -> Self {
        self.styled = StyledText::new(self.text.clone()).with_runs(runs);
        self
    }

    pub fn with_highlights(
        mut self,
        h: impl IntoIterator<Item = (Range<usize>, HighlightStyle)>,
    ) -> Self {
        self.styled = StyledText::new(self.text.clone()).with_highlights(h);
        self
    }

    /// The pointing hand over it, not the text cursor: text that is also a button (a name).
    pub fn pointer(mut self) -> Self {
        self.cursor = CursorStyle::PointingHand;
        self
    }

    /// A click on one of `ranges` calls `f` with its number.
    pub fn on_link(
        mut self,
        ranges: Vec<Range<usize>>,
        f: impl Fn(usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.links = ranges;
        self.on_link = Some(Rc::new(f));
        self
    }
}

/// The rectangles of `r` in `layout`: one per line it touches, the inner lines full width.
fn rects(layout: &TextLayout, r: &Range<usize>, bounds: Bounds<Pixels>) -> Vec<Bounds<Pixels>> {
    let (Some(a), Some(b)) = (
        layout.position_for_index(r.start),
        layout.position_for_index(r.end),
    ) else {
        return vec![];
    };
    let lh = layout.line_height();
    if a.y == b.y {
        return vec![Bounds::from_corners(a, point(b.x, b.y + lh))];
    }
    let mut out = vec![Bounds::from_corners(a, point(bounds.right(), a.y + lh))];
    if b.y > a.y + lh {
        out.push(Bounds::from_corners(
            point(bounds.left(), a.y + lh),
            point(bounds.right(), b.y),
        ));
    }
    out.push(Bounds::from_corners(
        point(bounds.left(), b.y),
        point(b.x, b.y + lh),
    ));
    out
}

impl Element for SelText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.styled.request_layout(None, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.styled
            .prepaint(None, inspector_id, bounds, state, window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.styled
            .paint(None, inspector_id, bounds, state, &mut (), window, cx);
        let layout = self.styled.layout().clone();
        let key = self.key;
        if let Some(r) = cx
            .try_global::<Selection>()
            .and_then(|s| s.part(key, self.text.len()))
        {
            let color = tokens(cx).selection();
            for r in rects(&layout, &r, bounds) {
                window.paint_quad(fill(r, color));
            }
        }
        cx.default_global::<Selection>().laid.insert(
            key,
            Laid {
                text: self.text.clone(),
                bounds: bounds.intersect(&window.content_mask().bounds),
                layout: layout.clone(),
            },
        );
        let on_link = layout
            .index_for_position(window.mouse_position())
            .is_ok_and(|i| self.links.iter().any(|r| r.contains(&i)));
        let cursor = if on_link {
            CursorStyle::PointingHand
        } else {
            self.cursor
        };
        window.set_cursor_style(cursor, hitbox);
        let view = window.current_view();

        let (text, hb, lay) = (self.text.clone(), hitbox.clone(), layout.clone());
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if e.button != MouseButton::Left {
                return;
            }
            let s = cx.default_global::<Selection>();
            if phase == DispatchPhase::Capture {
                // A press anywhere lets the selection go (a Shift press may extend it); the text under it then starts
                // a new one.
                if !e.modifiers.shift && (s.anchor != s.head || s.dragging) {
                    s.head = s.anchor;
                    s.dragging = false;
                    cx.notify(view);
                }
                return;
            }
            if !hb.is_hovered(window) {
                return;
            }
            let ix = lay.index_for_position(e.position).unwrap_or_else(|i| i);
            let at = |ix| Pos { key, ix };
            let (anchor, head) = match e.click_count {
                1 if e.modifiers.shift && s.anchor.key.0 == key.0 => (s.anchor, at(ix)),
                1 => (at(ix), at(ix)),
                2 => {
                    let w = word_at(&text, ix);
                    (at(w.start), at(w.end))
                }
                _ => (at(0), at(text.len())),
            };
            *s = Selection {
                anchor,
                head,
                dragging: e.click_count == 1,
                laid: std::mem::take(&mut s.laid),
            };
            // The composer lets go of the keys, so Ctrl+C copies this (as a click on a page's text blurs a field).
            if let Some(f) = cx.try_global::<TextFocus>() {
                window.focus(&f.0.clone());
            }
            cx.notify(view);
        });

        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble
                || e.pressed_button != Some(MouseButton::Left)
                || !Selection::dragging(key.0, cx)
            {
                return;
            }
            // One text of the scope moves the head: the first drawn.
            let first = cx
                .global::<Selection>()
                .laid
                .range(scope_keys(key.0))
                .next()
                .map(|(k, _)| *k);
            if first == Some(key) {
                Selection::drag(e.position, cx);
                // Drawn again even if the head stays: the chat list scrolls while the pointer is past its edge.
                cx.notify(view);
            }
        });

        let (links, on_link) = (self.links.clone(), self.on_link.clone());
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if e.button != MouseButton::Left {
                return;
            }
            let s = cx.default_global::<Selection>();
            if phase == DispatchPhase::Bubble {
                s.dragging = false;
                return;
            }
            // A click, not a drag, on this text: a link under it opens.
            let click =
                (s.dragging && s.anchor == s.head && s.anchor.key == key).then_some(s.head.ix);
            if let (Some(ix), Some(f)) = (click, &on_link)
                && let Some(n) = links.iter().position(|r| r.contains(&ix))
            {
                f(n, window, cx);
            }
        });
    }
}

impl IntoElement for SelText {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_double_click_takes_the_word() {
        let t = "hello, wörld_2 ok";
        assert_eq!(&t[word_at(t, 2)], "hello");
        assert_eq!(&t[word_at(t, 0)], "hello");
        assert_eq!(&t[word_at(t, 5)], "hello", "just after a word: that word");
        assert_eq!(&t[word_at(t, 6)], " ", "between: the one character");
        assert_eq!(&t[word_at(t, 8)], "wörld_2");
        assert_eq!(&t[word_at(t, t.len())], "ok");
        assert_eq!(word_at("", 0), 0..0);
    }

    #[test]
    fn a_range_takes_the_ends_in_part_and_the_middle_whole() {
        let at = |row, part, ix| Pos {
            key: (CHAT, row, part),
            ix,
        };
        let s = Selection {
            anchor: at(2, 1, 3),
            head: at(1, 4, 2),
            ..Default::default()
        };
        assert_eq!(s.part((CHAT, 1, 3), 9), None, "before the start");
        assert_eq!(
            s.part((CHAT, 1, 4), 9),
            Some(2..9),
            "the start, from the head"
        );
        assert_eq!(s.part((CHAT, 1, 9), 5), Some(0..5), "between");
        assert_eq!(
            s.part((CHAT, 2, 1), 9),
            Some(0..3),
            "the end, to the anchor"
        );
        assert_eq!(s.part((CHAT, 2, 2), 9), None, "after the end");
        assert_eq!(s.part((PINS, 1, 9), 9), None, "another scope");
        let mut all = Selection::default();
        let k = scope_keys(CHAT);
        (all.anchor, all.head) = (
            Pos {
                key: *k.start(),
                ix: 0,
            },
            Pos {
                key: *k.end(),
                ix: usize::MAX,
            },
        );
        assert_eq!(all.part((CHAT, 7, 0), 4), Some(0..4), "select all");
        assert_eq!(all.part((FIND, 7, 0), 4), None);
    }
}
