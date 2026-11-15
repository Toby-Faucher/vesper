//! Where the signed-in session is kept between runs: the OS keyring, never a plain file.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use vesper_protocol::Secret;

/// A signed-in session. `cookie` is `name=value` pairs; `Debug` shows `***` for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Saved {
    /// The server's origin, e.g. `https://vesper.example.com`.
    pub server: String,
    pub cookie: Secret,
}

/// Keeps one saved session. An `Err` means the store itself is not there (no keyring); the text is for a log line
/// and never holds the secret.
pub trait SessionStore: Send + Sync + 'static {
    fn load(&self) -> Result<Option<Saved>, String>;
    fn save(&self, s: &Saved) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}

/// The OS keyring: Secret Service on Linux, Keychain on macOS, Credential Manager on Windows.
pub struct Keyring;

const SERVICE: &str = "vesper-desktop";
const ACCOUNT: &str = "session";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())
}

impl SessionStore for Keyring {
    fn load(&self) -> Result<Option<Saved>, String> {
        match entry()?.get_password() {
            // A value we cannot read is as good as none; the next sign in replaces it.
            Ok(s) => Ok(serde_json::from_str(&s).ok()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn save(&self, s: &Saved) -> Result<(), String> {
        let json = serde_json::to_string(s).map_err(|e| e.to_string())?;
        entry()?.set_password(&json).map_err(|e| e.to_string())
    }

    fn clear(&self) -> Result<(), String> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// An in-memory store for tests. `broken` acts like a machine with no keyring.
#[derive(Default)]
pub struct MemStore {
    pub saved: Mutex<Option<Saved>>,
    pub broken: bool,
}

impl MemStore {
    pub fn broken() -> Self {
        MemStore {
            saved: Mutex::default(),
            broken: true,
        }
    }

    pub fn get(&self) -> Option<Saved> {
        self.saved.lock().unwrap().clone()
    }
}

impl SessionStore for MemStore {
    fn load(&self) -> Result<Option<Saved>, String> {
        if self.broken {
            return Err("no keyring".into());
        }
        Ok(self.get())
    }

    fn save(&self, s: &Saved) -> Result<(), String> {
        if self.broken {
            return Err("no keyring".into());
        }
        *self.saved.lock().unwrap() = Some(s.clone());
        Ok(())
    }

    fn clear(&self) -> Result<(), String> {
        *self.saved.lock().unwrap() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_cookie() {
        let s = Saved {
            server: "https://x.example".into(),
            cookie: "vesper_session=abc123".into(),
        };
        let d = format!("{s:?}");
        assert!(!d.contains("abc123") && d.contains("***"), "{d}");
    }
}
