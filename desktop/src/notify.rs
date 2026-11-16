//! System notifications and the unread count on the app's icons. Which messages notify is the web's `shouldNotify`
//! (`web/src/lib/ui/notify.ts`); the count is its title count plus unread direct messages.
//!
//! Linux: freedesktop notifications and the launcher's count (`com.canonical.Unity.LauncherEntry`, read by KDE, Dash
//! to Dock and Plank) over D-Bus with zbus, on one thread. Windows: toasts (`tauri-winrt-notification`) under our own
//! app id (`register`). A click opens the room and brings the window up. Message text goes to the notification only,
//! never to the log.

use vesper_protocol::{Message, RoomKind};

use crate::settings::Settings;
use crate::state::AppState;

/// One notification. `room` opens on a click.
#[derive(Debug, Clone, PartialEq)]
pub struct Toast {
    pub title: String,
    pub body: String,
    pub room: i64,
}

/// A notification for a new message: a mention of me (mentions on), or any message in a DM (direct messages on), by
/// someone else, not deleted, in a room I am not looking at. `looking_at` is the shown room while the window has
/// focus and the room is on screen.
pub fn for_message(
    st: &AppState,
    m: &Message,
    looking_at: Option<i64>,
    s: &Settings,
) -> Option<Toast> {
    let room = st.find_room(m.room_id)?;
    let dm = room.kind == RoomKind::Dm;
    let me = st.my_id();
    let wanted = if dm {
        s.notify_dms
    } else {
        s.notify_mentions && m.mentions.iter().any(|id| id == me)
    };
    if !wanted || m.author_id == me || m.deleted || looking_at == Some(m.room_id) {
        return None;
    }
    let name = st.name_of(&m.author_id);
    let title = if dm {
        format!("{name} sent you a message")
    } else {
        format!("{name} mentioned you in {}", room.name)
    };
    let text = crate::markdown::plain(&crate::markdown::parse(&m.body));
    Some(Toast {
        title,
        body: text.trim().chars().take(120).collect(),
        room: m.room_id,
    })
}

/// A notification for a DM call while the window does not have focus (with focus, the ring prompt shows).
pub fn for_ring(
    st: &AppState,
    room: i64,
    caller: &str,
    focused: bool,
    s: &Settings,
) -> Option<Toast> {
    (s.notify_dms && !focused).then(|| Toast {
        title: format!("{} is calling you", st.name_of(caller)),
        body: "Open Vesper to answer.".into(),
        room,
    })
}

/// The count on the icons: mentions in rooms, every unread message in DMs.
pub fn badge(st: &AppState) -> u32 {
    st.unread
        .values()
        .map(|u| {
            let dm = st.dms.iter().any(|r| r.id == u.room_id);
            if dm { u.unread } else { u.mentions }
        })
        .sum()
}

/// The window title, the web's `titleFor` (the taskbar shows it).
pub fn title(count: u32) -> String {
    if count > 0 {
        format!("({count}) Vesper")
    } else {
        "Vesper".into()
    }
}

enum Msg {
    Show(Toast),
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Count(u32),
}

/// The system side. Sends never block; with no notification service they do nothing.
pub struct Os {
    tx: tokio::sync::mpsc::UnboundedSender<Msg>,
}

impl Os {
    /// Starts the notification thread. `clicked` gets the room of a clicked notification, on that thread.
    pub fn start(clicked: impl Fn(i64) + Send + Sync + 'static) -> Os {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let _ = std::thread::Builder::new()
            .name("vesper-notify".into())
            .spawn(move || run(rx, clicked));
        Os { tx }
    }

    pub fn show(&self, t: Toast) {
        let _ = self.tx.send(Msg::Show(t));
    }

    /// The launcher's count (Linux only; Windows has the dot on the tray and taskbar icons, and the title).
    pub fn count(&self, n: u32) {
        let _ = self.tx.send(Msg::Count(n));
    }
}

#[cfg(target_os = "linux")]
fn run(mut rx: tokio::sync::mpsc::UnboundedReceiver<Msg>, clicked: impl Fn(i64)) {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use futures_util::StreamExt;
    use zbus::zvariant::Value;

    let r = zbus::block_on(async move {
        let conn = zbus::Connection::session().await?;
        let proxy = zbus::Proxy::new(
            &conn,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .await?;
        let mut clicks = proxy.receive_signal("ActionInvoked").await?;
        // Notification id to room, the newest 50.
        let rooms = RefCell::new(HashMap::<u32, i64>::new());
        let send = async {
            while let Some(m) = rx.recv().await {
                match m {
                    Msg::Show(t) => {
                        // `desktop-entry` names the app's .desktop file, for its icon and settings.
                        let hints = HashMap::from([("desktop-entry", Value::from("vesper"))]);
                        let body = (
                            "Vesper",
                            0u32,
                            "vesper",
                            t.title.as_str(),
                            t.body.as_str(),
                            vec!["default", "Open"],
                            hints,
                            -1i32,
                        );
                        match proxy.call::<_, _, u32>("Notify", &body).await {
                            Ok(id) => {
                                let mut r = rooms.borrow_mut();
                                r.retain(|k, _| k.saturating_add(50) > id);
                                r.insert(id, t.room);
                            }
                            Err(e) => log::warn!("notification not shown: {e}"),
                        }
                    }
                    Msg::Count(n) => {
                        let props = HashMap::from([
                            ("count", Value::from(i64::from(n))),
                            ("count-visible", Value::from(n > 0)),
                        ]);
                        let _ = conn
                            .emit_signal(
                                None::<zbus::names::BusName<'_>>,
                                "/com/canonical/unity/launcherentry/vesper",
                                "com.canonical.Unity.LauncherEntry",
                                "Update",
                                &("application://vesper.desktop", props),
                            )
                            .await;
                    }
                }
            }
        };
        let hear = async {
            while let Some(sig) = clicks.next().await {
                if let Ok((id, action)) = sig.body().deserialize::<(u32, String)>()
                    && action == "default"
                    && let Some(&room) = rooms.borrow().get(&id)
                {
                    clicked(room);
                }
            }
        };
        futures_util::future::join(send, hear).await;
        Ok::<(), zbus::Error>(())
    });
    if let Err(e) = r {
        log::warn!("no notifications: {e}");
    }
}

#[cfg(windows)]
fn run(
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Msg>,
    clicked: impl Fn(i64) + Send + Sync + 'static,
) {
    use std::sync::Arc;
    use tauri_winrt_notification::Toast as WinToast;

    let id = match register() {
        Ok(()) => APP_ID,
        Err(e) => {
            log::warn!("toasts show as Windows PowerShell: {e}");
            WinToast::POWERSHELL_APP_ID
        }
    };
    let clicked = Arc::new(clicked);
    while let Some(m) = rx.blocking_recv() {
        let Msg::Show(t) = m else { continue };
        let clicked = clicked.clone();
        let room = t.room;
        let r = WinToast::new(id)
            .title(&t.title)
            .text1(&t.body)
            .on_activated(move |_| {
                clicked(room);
                Ok(())
            })
            .show();
        if let Err(e) = r {
            log::warn!("notification not shown: {e}");
        }
    }
}

/// Our app id for toasts. Registered under it, a toast says "Vesper" with our icon; no Start menu shortcut needed.
#[cfg(windows)]
const APP_ID: &str = "Vesper.Desktop";

/// Registers `APP_ID` for the current user (no admin), at every start so a portable copy gets it too. The installer
/// writes the same key and its uninstaller removes it. The icon is `vesper.png` beside Vesper.exe, written if missing;
/// in a folder we cannot write to, the toast shows without it.
#[cfg(windows)]
fn register() -> Result<(), Box<dyn std::error::Error>> {
    let key = windows_registry::CURRENT_USER
        .create(format!(r"Software\Classes\AppUserModelId\{APP_ID}"))?;
    key.set_string("DisplayName", "Vesper")?;
    let icon = std::env::current_exe()?.with_file_name("vesper.png");
    if icon.exists() || std::fs::write(&icon, crate::ui::tray::PNG).is_ok() {
        key.set_string("IconUri", icon.to_string_lossy())?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn run(_: tokio::sync::mpsc::UnboundedReceiver<Msg>, _: impl Fn(i64)) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tests::{hello, msg};
    use vesper_protocol::Unread;

    fn state() -> AppState {
        let mut st = AppState::default();
        st.apply(&hello()); // I am Ann (a); room 1 is general, 3 a DM with Bob (b)
        st
    }

    fn m(room: i64, author: &str, mentions: &[&str], body: &str) -> Message {
        Message {
            mentions: mentions.iter().map(|s| s.to_string()).collect(),
            ..msg(10, room, author, body)
        }
    }

    #[test]
    fn notifies_like_the_web() {
        let st = state();
        let on = Settings::default();
        let t = for_message(&st, &m(1, "b", &["a"], "hi **@ann**"), None, &on).unwrap();
        assert_eq!(t.title, "Bob mentioned you in general");
        assert_eq!((t.body.as_str(), t.room), ("hi @ann", 1));
        let t = for_message(&st, &m(3, "b", &[], "yo"), None, &on).unwrap();
        assert_eq!(t.title, "Bob sent you a message");

        assert!(for_message(&st, &m(1, "b", &[], "no mention"), None, &on).is_none());
        assert!(for_message(&st, &m(1, "a", &["a"], "mine"), None, &on).is_none());
        let gone = Message {
            deleted: true,
            ..m(3, "b", &[], "x")
        };
        assert!(for_message(&st, &gone, None, &on).is_none());
        assert!(
            for_message(&st, &m(3, "b", &[], "x"), Some(3), &on).is_none(),
            "looking at it"
        );
        assert!(for_message(&st, &m(3, "b", &[], "x"), Some(1), &on).is_some());
        assert!(
            for_message(&st, &m(9, "b", &["a"], "x"), None, &on).is_none(),
            "unknown room"
        );
        let off = Settings {
            notify_mentions: false,
            notify_dms: false,
            ..Settings::default()
        };
        assert!(for_message(&st, &m(1, "b", &["a"], "x"), None, &off).is_none());
        assert!(for_message(&st, &m(3, "b", &[], "x"), None, &off).is_none());
        let long = "a".repeat(300);
        let t = for_message(&st, &m(3, "b", &[], &long), None, &on).unwrap();
        assert_eq!(t.body.chars().count(), 120);

        assert_eq!(
            for_ring(&st, 3, "b", false, &on).unwrap().title,
            "Bob is calling you"
        );
        assert!(
            for_ring(&st, 3, "b", true, &on).is_none(),
            "the prompt shows"
        );
        assert!(for_ring(&st, 3, "b", false, &off).is_none());
    }

    #[test]
    fn the_badge_counts_mentions_and_dm_messages() {
        let mut st = state();
        let u = |room_id, unread, mentions| Unread {
            room_id,
            last_read_id: 0,
            unread,
            mentions,
        };
        st.unread.insert(1, u(1, 7, 2));
        st.unread.insert(3, u(3, 3, 0));
        assert_eq!(badge(&st), 5);
        assert_eq!(
            (title(5).as_str(), title(0).as_str()),
            ("(5) Vesper", "Vesper")
        );
    }
}
