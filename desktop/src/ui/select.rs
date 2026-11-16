//! Selectable text, as a browser selects a page's text: a drag selects, a double click selects a word, a triple click
//! the whole text, Shift extends, and Ctrl+C (Cmd+C) copies. One selection at a time, inside one text element (the
//! web can also select across messages; this cannot). It is drawn in `--selection`, the accent at 30 %, over the text.
//! A click (a press and release with no drag) on a link opens it.

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, CursorStyle, DispatchPhase, Element, ElementId, GlobalElementId,
    Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, SharedString, StyledText, TextLayout, TextRun, Window,
    fill, point,
};

use super::theme::tokens;

/// The selection: which text (`key`), its text then, where the press was (`anchor`) and where it is now (`head`).
#[derive(Default)]
pub struct Selection {
    key: SharedString,
    text: SharedString,
    anchor: usize,
    head: usize,
    dragging: bool,
}

impl gpui::Global for Selection {}

/// What a press on selectable text focuses (the root view's), so Ctrl+C reaches the copy there and a field's own
/// Ctrl+C does not take it.
pub struct TextFocus(pub gpui::FocusHandle);

impl gpui::Global for TextFocus {}

impl Selection {
    fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    /// The selected text, `None` when nothing is selected. The thin spaces that pad inline code and mentions are left
    /// out (`md.rs`).
    pub fn selected(cx: &App) -> Option<String> {
        let s = cx.try_global::<Selection>()?;
        let r = s.range();
        let t = s.text.get(r).filter(|t| !t.is_empty())?;
        Some(t.replace(['\u{2005}', '\u{200a}'], ""))
    }

    /// Copies the selection, if there is one.
    pub fn copy(cx: &mut App) {
        if let Some(t) = Self::selected(cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(t));
        }
    }

    /// A drag selected something in `key` (a click on it should not also act).
    pub fn dragged_in(key: &str, cx: &App) -> bool {
        cx.try_global::<Selection>()
            .is_some_and(|s| s.key == key && s.anchor != s.head)
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

/// Text that can be selected. `key` names it for the selection; keep it stable across frames and unique.
pub struct SelText {
    key: SharedString,
    text: SharedString,
    styled: StyledText,
    links: Vec<Range<usize>>,
    on_link: Option<OnLink>,
    cursor: CursorStyle,
}

impl SelText {
    pub fn new(key: impl Into<SharedString>, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        SelText {
            key: key.into(),
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
        if let Some(s) = cx.try_global::<Selection>().filter(|s| s.key == self.key) {
            let color = tokens(cx).selection();
            for r in rects(&layout, &s.range(), bounds) {
                window.paint_quad(fill(r, color));
            }
        }
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

        let (key, text, hb) = (self.key.clone(), self.text.clone(), hitbox.clone());
        let lay = layout.clone();
        window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                return;
            }
            let s = cx.default_global::<Selection>();
            if !hb.is_hovered(window) {
                // A press anywhere else lets this selection go.
                if s.key == key {
                    *s = Selection::default();
                    cx.notify(view);
                }
                return;
            }
            let ix = lay.index_for_position(e.position).unwrap_or_else(|i| i);
            let mine = s.key == key;
            let r = match e.click_count {
                1 if e.modifiers.shift && mine => s.anchor..ix,
                1 => ix..ix,
                2 => word_at(&text, ix),
                _ => 0..text.len(),
            };
            *s = Selection {
                key: key.clone(),
                text: text.clone(),
                anchor: r.start,
                head: r.end,
                dragging: e.click_count == 1,
            };
            // The composer lets go of the keys, so Ctrl+C copies this (as a click on a page's text blurs a field).
            if let Some(f) = cx.try_global::<TextFocus>() {
                window.focus(&f.0.clone());
            }
            cx.notify(view);
        });

        let (key, lay) = (self.key.clone(), layout.clone());
        window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if let Some(s) = cx.try_global::<Selection>()
                && s.key == key
                && s.dragging
            {
                let head = lay.index_for_position(e.position).unwrap_or_else(|i| i);
                if head != s.head {
                    cx.global_mut::<Selection>().head = head;
                    cx.notify(view);
                }
            }
        });

        let (key, links, on_link) = (self.key.clone(), self.links.clone(), self.on_link.clone());
        window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || e.button != MouseButton::Left {
                return;
            }
            let Some(s) = cx
                .try_global::<Selection>()
                .filter(|s| s.key == key && s.dragging)
            else {
                return;
            };
            let click = (s.anchor == s.head).then_some(s.head);
            cx.global_mut::<Selection>().dragging = false;
            // A click, not a drag: a link under it opens.
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
}
