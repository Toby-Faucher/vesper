//! The tray icon: Show, Mute, Deafen, Quit. `tray-icon` (Tauri). On Windows it needs a message loop on the thread
//! that made it: GPUI's main thread runs one (`GetMessageW` for every window of the thread), so the icon is made
//! there. On macOS it must be made on the main thread once the app runs, which is where `Application::run` calls
//! us. On Linux it uses KSNI (StatusNotifierItem over D-Bus, on its own thread), so it needs no GTK; a desktop
//! without a tray host shows no icon.
//!
//! Menu clicks come on `tray-icon`'s own thread and are passed to the UI thread through a channel.

use gpui::{AnyWindowHandle, App, AppContext, AsyncApp, WeakEntity};
use tokio::sync::mpsc::unbounded_channel;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use super::Vesper;
use crate::model::Model;

/// The app icon (package/icons, made from the web's icon-512.png), a 64 pixel PNG.
pub const PNG: &[u8] = include_bytes!("../../package/icons/64.png");

/// The app icon's pixels: RGBA and its width.
fn pixels() -> Option<(Vec<u8>, u32)> {
    let img = image::load_from_memory(PNG).ok()?.into_rgba8();
    let w = img.width();
    Some((img.into_raw(), w))
}

/// The app icon.
fn icon() -> Option<Icon> {
    let (rgba, n) = pixels()?;
    Icon::from_rgba(rgba, n, n).ok()
}

/// The icon with a red dot at its top right, for unread mentions and DMs. The dot is drawn over `rgba`, a square
/// RGBA picture `n` pixels wide, with a dark ring so it reads on light and dark bars.
fn with_dot(rgba: Vec<u8>, n: u32) -> Vec<u8> {
    let r = n as f32 / 4.0;
    dot(rgba, n, r, (n as f32 - r - 0.5, r - 0.5))
}

/// The dot alone, filling a clear square `n` pixels wide: the taskbar's overlay on Windows.
pub fn badge(n: u32) -> Vec<u8> {
    let r = n as f32 / 2.0;
    dot(vec![0; (n * n * 4) as usize], n, r, (r - 0.5, r - 0.5))
}

/// Draws a dot of radius `r` centred on `(cx, cy)` over `rgba`, a square RGBA picture `n` pixels wide.
fn dot(mut rgba: Vec<u8>, n: u32, r: f32, (cx, cy): (f32, f32)) -> Vec<u8> {
    for (i, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i as u32 % n) as f32 - cx, (i as u32 / n) as f32 - cy);
        let d = (x * x + y * y).sqrt();
        let cover = (r - d).clamp(0.0, 1.0);
        if cover == 0.0 {
            continue;
        }
        let red = (r - 2.0 - d).clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| (b + (a - b) * red) as u8;
        px[..3].copy_from_slice(&[mix(239.0, 24.0), mix(68.0, 24.0), mix(68.0, 27.0)]);
        px[3] = px[3].max((cover * 255.0) as u8);
    }
    rgba
}

/// The tray's picture, with the unread dot or without.
pub fn set_unread(tray: &TrayIcon, unread: bool) {
    let icon = if unread {
        pixels().and_then(|(rgba, n)| Icon::from_rgba(with_dot(rgba, n), n, n).ok())
    } else {
        icon()
    };
    let _ = tray.set_icon(icon);
}

/// Makes the tray icon and handles its menu. `None` when the system has no tray; the app works without it.
pub fn start(cx: &mut App, view: WeakEntity<Vesper>, window: AnyWindowHandle) -> Option<TrayIcon> {
    let menu = Menu::new();
    menu.append_items(&[
        &MenuItem::with_id("show", "Show Vesper", true, None),
        &MenuItem::with_id("mute", "Mute or unmute", true, None),
        &MenuItem::with_id("deafen", "Deafen or undeafen", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", "Quit", true, None),
    ])
    .ok()?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Vesper")
        .with_icon(icon()?)
        .build()
        .ok()?;

    let (tx, mut rx) = unbounded_channel();
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let _ = tx.send(e.id.0);
    }));
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(id) = rx.recv().await {
            let _ = match id.as_str() {
                "show" => cx.update_window(window, |_, window, cx| {
                    window.activate_window();
                    cx.activate(true);
                }),
                "mute" => view.update(cx, |v, cx| v.act(Model::toggle_mute, cx)),
                "deafen" => view.update(cx, |v, cx| v.act(Model::toggle_deafen, cx)),
                "quit" => cx.update(|cx| cx.quit()),
                _ => Ok(()),
            };
        }
    })
    .detach();
    Some(tray)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_app_icon_decodes() {
        assert!(super::icon().is_some());
    }

    #[test]
    fn the_dot_is_red_at_the_top_right_and_leaves_the_rest() {
        let n = 32;
        let rgba = super::with_dot(vec![0u8; (n * n * 4) as usize], n);
        let at = |x: u32, y: u32| &rgba[((y * n + x) * 4) as usize..][..4];
        assert_eq!(at(24, 7), [239, 68, 68, 255], "red in the middle");
        assert_eq!(at(31, 7)[..3], [24, 24, 27], "a dark rim");
        assert_eq!(at(4, 28), [0, 0, 0, 0], "the rest is left alone");
    }

    #[test]
    fn the_badge_is_a_red_dot_on_a_clear_square() {
        let rgba = super::badge(32);
        let at = |x: u32, y: u32| &rgba[((y * 32 + x) * 4) as usize..][..4];
        assert_eq!(at(16, 16), [239, 68, 68, 255], "red in the middle");
        assert_eq!(at(31, 16)[..3], [24, 24, 27], "a dark rim");
        assert_eq!(at(0, 0), [0, 0, 0, 0], "clear corners");
    }
}
