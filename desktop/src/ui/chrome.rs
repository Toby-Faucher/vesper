//! The window's own minimize, maximize and close at the top right of every screen, as Discord draws them: the app
//! asks for client-side decorations, so tiling window managers, which draw no buttons, still have them. The empty
//! parts of the top bars move the window, and a double click there maximizes it. macOS keeps its own title bar.

use gpui::{
    Context, Decorations, Div, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled,
    Window, WindowControlArea, div, px,
};
use gpui_component::h_flex;

use super::{Vesper, c};

/// The controls are drawn (everywhere but macOS).
pub const SHOWN: bool = cfg!(not(target_os = "macos"));
/// The width a 48 px top bar leaves free at its right for the controls, past its own 16 px: three 32 px buttons
/// 4 px apart, 8 px from the window's edge, and the bar's 12 px gap.
pub const RESERVE: f32 = if SHOWN {
    3. * 32. + 2. * 4. + 8. + 12. - 16.
} else {
    0.
};

impl Vesper {
    /// The three buttons, floating at the window's top right over the 48 px top bar.
    pub(super) fn window_controls(&self, window: &Window, cx: &mut Context<Self>) -> Option<Div> {
        if !SHOWN {
            return None;
        }
        let restore = window.is_maximized() || window.is_fullscreen();
        let max = if restore { "winRestore" } else { "winMax" };
        Some(
            h_flex()
                .absolute()
                .top_0()
                .right_0()
                .h(px(48.))
                .pr(px(8.))
                .gap(px(4.))
                .child(control(
                    "win-min",
                    "winMin",
                    WindowControlArea::Min,
                    cx.listener(|_, _, w, _| w.minimize_window()),
                ))
                .child(control(
                    "win-max",
                    max,
                    WindowControlArea::Max,
                    cx.listener(|_, _, w, _| toggle_max(w)),
                ))
                .child(control(
                    "win-close",
                    "x",
                    WindowControlArea::Close,
                    cx.listener(|this, _, w, cx| this.close_window(w, cx)),
                )),
        )
    }

    /// Close: as the system's close does (`on_window_should_close` in `run`), a call keeps going with the window
    /// minimised; else the window closes and the app quits.
    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.call.phase != crate::model::Phase::Idle {
            self.minimised_by_close(cx);
            window.minimize_window();
        } else {
            window.remove_window();
        }
    }

    /// `d` moves the window when dragged, and maximizes it on a double click. Give it only empty space: on Windows
    /// the system takes every click there.
    pub(super) fn draggable(&self, d: Div, cx: &mut Context<Self>) -> Div {
        if !SHOWN {
            return d;
        }
        if cfg!(windows) {
            return d.window_control_area(WindowControlArea::Drag);
        }
        // Moved on the first move after the press, not on the press itself, so a double click still comes.
        d.on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, e: &gpui::MouseDownEvent, w, _| {
                this.drag_armed = e.click_count == 1;
                if e.click_count == 2 {
                    toggle_max(w);
                }
            }),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, _, _| this.drag_armed = false),
        )
        .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, w, _| {
            if std::mem::take(&mut this.drag_armed) && e.pressed_button == Some(MouseButton::Left) {
                w.start_window_move();
            }
        }))
    }

    /// An empty stretch of a top bar that moves the window.
    pub(super) fn drag_space(&self, cx: &mut Context<Self>) -> Div {
        self.draggable(div().flex_1().h_full(), cx)
    }
}

/// Maximize or restore. A tiled window on Linux cannot be maximized (the tiling window manager places it), so there
/// it goes full screen instead.
fn toggle_max(w: &mut Window) {
    let tiled =
        matches!(w.window_decorations(), Decorations::Client { tiling } if tiling.is_tiled());
    if cfg!(target_os = "linux") && (tiled || w.is_fullscreen()) {
        w.toggle_fullscreen();
    } else {
        w.zoom_window();
    }
}

/// One button. On Windows the system handles the click (and shows its snap layouts on Maximize), so the button only
/// marks its area; elsewhere it acts on click.
fn control(
    id: &'static str,
    icon: &'static str,
    area: WindowControlArea,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> impl IntoElement {
    let b = c::IconButton::new(id, icon);
    let d = div().debug_selector(move || id.into());
    if cfg!(windows) {
        d.window_control_area(area).child(b)
    } else {
        d.child(b.on_click(on_click))
    }
}
