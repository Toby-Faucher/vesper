//! A share popped out into its own window, as the web's Pop out (`pip.ts`) puts it in the browser's picture in
//! picture window: the whole picture letterboxed on `--bg-app`, with the sharer's name. One window at a time: another
//! share popped out takes its place. It closes when popped back in, when the share or the call ends, and its own
//! close button pops the share back in.
//!
//! The window draws what the main window keeps (`ShareView::screens`) and takes the popped share's newest picture
//! itself, so it goes on while the main window is minimised. Dragging the picture moves the window, for a system that
//! gives it no title bar (GNOME under Wayland).
//!
//! Not here: always on top (GPUI 0.2.2 has no such window option).

use std::time::Instant;

use gpui::{
    App, AppContext, Context, Entity, ImageSource, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, Styled, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowOptions, div, img, px, size,
};

use super::Vesper;
use super::c::Avatar;
use super::share::{picture, picture_box};
use super::theme::{TEXT_SM, TEXT_XS, TextStepExt, tokens};

pub struct PopOut {
    main: WeakEntity<Vesper>,
    _watch: gpui::Subscription,
}

impl PopOut {
    /// Opens the window for the share `main` has popped out.
    pub fn open(main: &Entity<Vesper>, cx: &mut App) -> Option<Entity<PopOut>> {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(640.), px(360.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("Vesper".into()),
                ..Default::default()
            }),
            window_min_size: Some(size(px(240.), px(135.))),
            app_id: Some("vesper".into()),
            ..Default::default()
        };
        let weak = main.downgrade();
        let handle = cx
            .open_window(options, |window, cx| {
                let back = weak.clone();
                // Its own close button pops the share back in.
                window.on_window_should_close(cx, move |_, cx| {
                    let _ = back.update(cx, |v, cx| {
                        v.pop = None;
                        v.act(|m| m.pop_out(None, Instant::now()), cx);
                    });
                    true
                });
                let main = weak.upgrade().expect("the main view opens the pop-out");
                cx.new(|cx| PopOut {
                    main: weak.clone(),
                    // Each change of the main view may bring a picture, or end the share.
                    _watch: cx.observe(&main, |_, _, cx| cx.notify()),
                })
            })
            .map_err(|e| eprintln!("vesper: pop out: {e}"))
            .ok()?;
        handle.entity(cx).ok()
    }
}

impl Render for PopOut {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = tokens(cx).clone();
        let root = div()
            .size_full()
            .relative()
            .bg(t.bg_app)
            .text_color(t.fg)
            .font_family(t.font_sans.clone())
            .text_step(TEXT_SM);
        let Some(main) = self.main.upgrade() else {
            window.remove_window();
            return root;
        };
        let Some(id) = main.read(cx).model.call.popped.clone() else {
            main.update(cx, |v, _| v.pop = None);
            window.remove_window();
            return root;
        };
        main.update(cx, |v, cx| {
            if let Some(f) = v.model.call.frames.remove(&id)
                && let Some(new) = picture(f)
                && let Some(old) = v.share.screens.insert(id.clone(), new)
            {
                cx.drop_image(old, Some(window));
            }
        });
        let v = main.read(cx);
        let name = v.model.state.name_of(&id);
        let accent = v
            .model
            .state
            .users
            .get(&id)
            .map_or(String::new(), |u| u.accent.clone());
        let vp = window.viewport_size();
        let pic = v.share.screens.get(&id).cloned().map(|i| {
            let s = i.size(0);
            let b = picture_box(
                f32::from(vp.width),
                f32::from(vp.height),
                s.width.0 as f32,
                s.height.0 as f32,
            );
            div()
                .absolute()
                .left(px(b.origin.x))
                .top(px(b.origin.y))
                .w(px(b.size.width))
                .h(px(b.size.height))
                .child(img(ImageSource::Render(i)).size_full())
        });
        root.debug_selector(|| "share-popout".into())
            .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
            .children(pic)
            .child(
                div()
                    .absolute()
                    .left(px(8.))
                    .bottom(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .h(px(24.))
                    .pl(px(2.))
                    .pr(px(8.))
                    .rounded(t.radius)
                    .bg(t.bg_app)
                    .text_step(TEXT_XS)
                    .whitespace_nowrap()
                    .child(Avatar::new(id.clone(), name.clone(), accent, 20))
                    .child(name),
            )
    }
}
