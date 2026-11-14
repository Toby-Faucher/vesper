//! Push to talk. A thread watches one key: globally through `device_query` (it asks the OS whether the key is down,
//! every 10 ms, so it sees the key while a game has focus and never takes the key away from it), and through the
//! window's own key events, which is all there is on Wayland. The key opens the mic and a short delay after the
//! release closes it, so the end of a word is not cut.

use std::str::FromStr;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use device_query::{DeviceQuery, DeviceState, Keycode};

/// The web app has no release delay; 200 ms keeps the last syllable without leaving the mic open long.
pub const RELEASE_DELAY: Duration = Duration::from_millis(200);
const POLL: Duration = Duration::from_millis(10);

pub const WAYLAND: &str =
    "Global keys need X11. On Wayland, push to talk works only while the Vesper window has focus.";
pub const NO_DISPLAY: &str = "Global keys are not available here, so push to talk works only while the Vesper window has focus.";
#[cfg(target_os = "macos")]
pub const NO_ACCESS: &str = "Give Vesper Accessibility access in System Settings to use push to talk while another app has focus.";

/// A key name as stored in the settings (`device_query`'s names), if it is one.
pub fn parse_key(name: &str) -> Option<Keycode> {
    Keycode::from_str(name).ok()
}

/// How a stored key reads: `Key5` is 5, the rest keep their names.
pub fn key_label(name: &str) -> String {
    name.strip_prefix("Key").unwrap_or(name).to_string()
}

/// A key pressed in the window (GPUI's `Keystroke::key`) as a stored key name, or why it cannot be used.
pub fn key_from_window(key: &str) -> Result<String, &'static str> {
    let named = match key {
        "space" => "Space",
        "`" => "Grave",
        "-" => "Minus",
        "=" => "Equal",
        "[" => "LeftBracket",
        "]" => "RightBracket",
        "\\" => "BackSlash",
        ";" => "Semicolon",
        "'" => "Apostrophe",
        "," => "Comma",
        "." => "Dot",
        "/" => "Slash",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "insert" => "Insert",
        "delete" => "Delete",
        "tab" | "enter" | "backspace" => {
            return Err("Tab, Enter and Backspace move around the app and type. Pick another one.");
        }
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => return Ok(format!("Key{k}")),
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_lowercase()) => {
            return Ok(k.to_ascii_uppercase());
        }
        k if k.starts_with('f') && parse_key(&k.to_ascii_uppercase()).is_some() => {
            return Ok(k.to_ascii_uppercase());
        }
        _ => {
            return Err(
                "That key cannot be used. Pick a letter, a number, an F key or a symbol key.",
            );
        }
    };
    Ok(named.into())
}

/// Why push to talk only works while the window has focus, or `None` when global keys work.
pub fn global_note() -> Option<&'static str> {
    #[cfg(target_os = "linux")]
    {
        let set = |v: &str| std::env::var_os(v).is_some_and(|s| !s.is_empty());
        if set("WAYLAND_DISPLAY") {
            return Some(WAYLAND);
        }
        if !set("DISPLAY") {
            return Some(NO_DISPLAY);
        }
    }
    #[cfg(target_os = "macos")]
    if DeviceState::checked_new().is_none() {
        return Some(NO_ACCESS);
    }
    None
}

/// Whether a key is down right now, anywhere on the computer.
pub trait KeySource {
    fn down(&mut self, key: Keycode) -> bool;
}

impl KeySource for DeviceState {
    fn down(&mut self, key: Keycode) -> bool {
        self.get_keys().contains(&key)
    }
}

/// The global key source, or `None` (Wayland, no display, no permission): then only window keys count.
pub fn global_source() -> Option<Box<dyn KeySource>> {
    if global_note().is_some() {
        return None;
    }
    DeviceState::checked_new().map(|d| Box::new(d) as Box<dyn KeySource>)
}

/// Held or released, with the release delay. Pure, so tests give it the time.
#[derive(Debug, Default)]
pub struct Ptt {
    down: bool,
    open: bool,
    close_at: Option<Instant>,
}

impl Ptt {
    /// The key's state at `now`. Returns whether the mic is open, when that changed.
    pub fn update(&mut self, down: bool, now: Instant) -> Option<bool> {
        if down {
            self.close_at = None;
        } else if self.down {
            self.close_at = Some(now + RELEASE_DELAY);
        }
        self.down = down;
        let open = down || self.close_at.is_some_and(|t| now < t);
        if !open {
            self.close_at = None;
        }
        (open != self.open).then(|| {
            self.open = open;
            open
        })
    }
}

/// To the key thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMsg {
    /// The key to watch; `None` when push to talk is off.
    Watch(Option<Keycode>),
    /// The key went down or up in the Vesper window.
    Window(bool),
}

/// Starts the key thread. `source` runs on it (an X11 connection must stay on one thread). `emit` gets whether the
/// mic is open each time it changes. The thread ends when the sender is dropped.
pub fn spawn(
    source: impl FnOnce() -> Option<Box<dyn KeySource>> + Send + 'static,
    emit: impl Fn(bool) + Send + 'static,
) -> Sender<KeyMsg> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("vesper-ptt".into())
        .spawn(move || watch(source(), &rx, &emit))
        .expect("the key thread starts");
    tx
}

fn watch(mut source: Option<Box<dyn KeySource>>, rx: &Receiver<KeyMsg>, emit: &dyn Fn(bool)) {
    let (mut key, mut window, mut ptt) = (None, false, Ptt::default());
    loop {
        let msg = if key.is_some() {
            rx.recv_timeout(POLL)
        } else {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        match msg {
            Ok(KeyMsg::Watch(k)) => {
                (key, window) = (k, false);
                if k.is_none() {
                    ptt = Ptt::default(); // off closes at once, without the delay
                    emit(false);
                }
            }
            Ok(KeyMsg::Window(d)) => window = d,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let Some(k) = key else { continue };
        let down = window || source.as_mut().is_some_and(|s| s.down(k));
        if let Some(open) = ptt.update(down, Instant::now()) {
            emit(open);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};

    use super::*;

    #[test]
    fn press_release_and_the_delay() {
        let t = Instant::now();
        let ms = |n| t + Duration::from_millis(n);
        let mut p = Ptt::default();
        assert_eq!(p.update(false, t), None, "starts closed");
        assert_eq!(p.update(true, ms(10)), Some(true), "press opens");
        assert_eq!(p.update(true, ms(20)), None);
        assert_eq!(p.update(false, ms(30)), None, "release keeps it open");
        assert_eq!(p.update(false, ms(229)), None);
        assert_eq!(
            p.update(false, ms(230)),
            Some(false),
            "closed after the delay"
        );
        // A press inside the delay keeps it open, and the delay starts again at the next release.
        p.update(true, ms(300));
        p.update(false, ms(310));
        assert_eq!(p.update(true, ms(400)), None);
        assert_eq!(p.update(false, ms(420)), None);
        assert_eq!(p.update(false, ms(600)), None);
        assert_eq!(p.update(false, ms(620)), Some(false));
    }

    #[test]
    fn window_keys_map_to_stored_names() {
        assert_eq!(key_from_window("v"), Ok("V".into()));
        assert_eq!(key_from_window("5"), Ok("Key5".into()));
        assert_eq!(key_from_window("f13"), Ok("F13".into()));
        assert_eq!(key_from_window("`"), Ok("Grave".into()));
        assert_eq!(key_from_window("space"), Ok("Space".into()));
        assert!(key_from_window("tab").is_err());
        assert!(key_from_window("f99").is_err());
        assert!(key_from_window("shift").is_err());
        for k in ["V", "Key5", "F13", "Grave", "Space", "BackSlash", "PageUp"] {
            assert!(parse_key(k).is_some(), "{k} is a device_query key");
        }
        assert_eq!(key_label("Key5"), "5");
    }

    /// A fake keyboard: a flag the test sets.
    struct Fake(Arc<AtomicBool>);
    impl KeySource for Fake {
        fn down(&mut self, key: Keycode) -> bool {
            key == Keycode::V && self.0.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn the_thread_follows_a_fake_key_and_the_window() {
        let held = Arc::new(AtomicBool::new(false));
        let (etx, erx) = mpsc::channel();
        let fake = held.clone();
        let tx = spawn(
            move || Some(Box::new(Fake(fake)) as Box<dyn KeySource>),
            move |open| {
                let _ = etx.send((open, Instant::now()));
            },
        );
        let next = || erx.recv_timeout(Duration::from_secs(2)).expect("an event");

        held.store(true, Ordering::SeqCst);
        tx.send(KeyMsg::Watch(Some(Keycode::B))).unwrap();
        assert!(
            erx.recv_timeout(Duration::from_millis(100)).is_err(),
            "another key is watched"
        );
        tx.send(KeyMsg::Watch(Some(Keycode::V))).unwrap();
        assert!(next().0, "global press opens");
        held.store(false, Ordering::SeqCst);
        let released = Instant::now();
        let (open, at) = next();
        assert!(!open);
        assert!(
            at - released >= RELEASE_DELAY - POLL,
            "closed after the delay"
        );

        tx.send(KeyMsg::Window(true)).unwrap();
        assert!(next().0, "a window press opens");
        tx.send(KeyMsg::Window(false)).unwrap();
        assert!(!next().0);

        held.store(true, Ordering::SeqCst);
        assert!(next().0);
        tx.send(KeyMsg::Watch(None)).unwrap();
        assert!(!next().0, "turning push to talk off closes at once");
    }
}
