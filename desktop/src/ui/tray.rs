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

/// A 32 px amber dot, drawn here so no image file or decoder is needed.
fn icon() -> Option<Icon> {
    const N: u32 = 32;
    let rgba = (0..N * N)
        .flat_map(|i| {
            let (x, y) = ((i % N) as f32 - 15.5, (i / N) as f32 - 15.5);
            let alpha = (15.5 - (x * x + y * y).sqrt()).clamp(0.0, 1.0);
            [0xf5, 0xa5, 0x24, (alpha * 255.0) as u8]
        })
        .collect();
    Icon::from_rgba(rgba, N, N).ok()
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
