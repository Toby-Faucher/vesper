//! The app's settings: a small JSON file in the OS config dir. It holds no secrets (the session is in the keyring).
//! Values are clamped on load like the web's `clampSettings`, so a hand-edited or old file cannot break a call.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What runs on the microphone before Opus. The web's modes are `deepfilter`, `browser` and `off`; the desktop has no
/// browser filter, so its light mode is the web's noise gate on the raw microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Denoise {
    #[default]
    DeepFilter,
    Gate,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub denoise: Denoise,
    /// Echo cancellation on the microphone.
    pub echo_cancel: bool,
    /// Push to talk; otherwise the mic sends whenever it is not muted (voice activity).
    pub push_to_talk: bool,
    /// A `device_query` key name, like `V`, `F13` or `Grave`.
    pub ptt_key: String,
    /// 32 to 510, 64 like the web's default.
    pub bitrate_kbps: u32,
    /// Device names; `None` or a name that is gone means the system default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub start_with_computer: bool,
    /// Each person's volume, 0 to 400 percent, by user id. Applied whenever they are in a call.
    pub volumes: HashMap<String, f32>,
    /// The sidebar is hidden (the web keeps this in localStorage).
    pub sidebar_collapsed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            denoise: Denoise::DeepFilter,
            echo_cancel: true,
            push_to_talk: false,
            // Not Space as on the web: a global Space would open the mic on every jump in a game.
            ptt_key: "Grave".into(),
            bitrate_kbps: 64,
            input_device: None,
            output_device: None,
            start_with_computer: false,
            volumes: HashMap::new(),
            sidebar_collapsed: false,
        }
    }
}

impl Settings {
    pub fn clamped(mut self) -> Settings {
        self.bitrate_kbps = self.bitrate_kbps.clamp(32, 510);
        if crate::voice::ptt::parse_key(&self.ptt_key).is_none() {
            self.ptt_key = Settings::default().ptt_key;
        }
        let max = crate::voice::mix::MAX_VOLUME;
        self.volumes.retain(|_, v| v.is_finite());
        self.volumes
            .values_mut()
            .for_each(|v| *v = v.clamp(0.0, max));
        self
    }

    /// The settings file, or defaults when it is missing or unreadable.
    pub fn load(path: &Path) -> Settings {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Settings>(&b).ok())
            .unwrap_or_default()
            .clamped()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).expect("settings are JSON");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, json).map_err(|e| e.to_string())
    }
}

/// `<config dir>/Vesper/settings.json`: `%APPDATA%` on Windows, `~/Library/Application Support` on macOS,
/// `$XDG_CONFIG_HOME` on Linux.
pub fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Vesper").join("settings.json"))
}

/// Turns starting with the computer on or off: the Run key on Windows, a login item on macOS, an autostart entry on
/// Linux. The error is a sentence for the user.
pub fn set_start_with_computer(on: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let launch = auto_launch::AutoLaunchBuilder::new()
        .set_app_name("Vesper")
        .set_app_path(&exe.to_string_lossy())
        .build()
        .map_err(|e| format!("Could not set start with the computer: {e}"))?;
    let r = if on {
        launch.enable()
    } else if launch.is_enabled().unwrap_or(true) {
        launch.disable()
    } else {
        Ok(())
    };
    r.map_err(|e| format!("Could not set start with the computer: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_defaults_and_clamps() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a").join("settings.json");
        assert_eq!(Settings::load(&p), Settings::default(), "missing file");
        let s = Settings {
            denoise: Denoise::Gate,
            echo_cancel: false,
            push_to_talk: true,
            ptt_key: "F13".into(),
            bitrate_kbps: 96,
            input_device: Some("USB mic".into()),
            output_device: None,
            start_with_computer: true,
            volumes: HashMap::from([("0123456789abcdef0123456789abcdef".into(), 250.0)]),
            sidebar_collapsed: true,
        };
        s.save(&p).unwrap();
        assert_eq!(Settings::load(&p), s);

        std::fs::write(
            &p,
            r#"{"bitrate_kbps": 9000, "ptt_key": "Nope", "denoise": "off", "volumes": {"a": 900, "b": -5}}"#,
        )
        .unwrap();
        let s = Settings::load(&p);
        assert_eq!(
            (s.bitrate_kbps, s.ptt_key.as_str(), s.denoise),
            (510, "Grave", Denoise::Off)
        );
        assert_eq!(
            (s.volumes["a"], s.volumes["b"]),
            (400.0, 0.0),
            "volumes clamped"
        );
        assert!(s.echo_cancel, "on when the file does not say");
        std::fs::write(&p, "not json").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
    }
}
