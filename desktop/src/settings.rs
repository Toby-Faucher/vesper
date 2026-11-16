//! The app's settings: a small JSON file in the OS config dir. It holds no secrets (the session is in the keyring).
//! Values are clamped on load like the web's `clampSettings`, so a hand-edited or old file cannot break a call.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What runs on the microphone before Opus. The web's modes are `deepfilter`, `rnnoise`, `browser` and `off`; the
/// desktop has no browser filter, so its light mode is the web's noise gate on the raw microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Denoise {
    #[default]
    DeepFilter,
    Rnnoise,
    Gate,
    Off,
}

/// How the share panel lays out the screens (the web's `vesper.shareLayout`). An unknown value reads as Grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareLayout {
    Focus,
    Dock,
    #[default]
    #[serde(other)]
    Grid,
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
    /// The level marker, -100 to 0 dBFS (the web's `vad.thresholdDb`): the noise gate opens above it.
    pub gate_db: i32,
    /// Device names; `None` or a name that is gone means the system default.
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub start_with_computer: bool,
    /// Each person's volume, 0 to 400 percent, by user id. Applied whenever they are in a call.
    pub volumes: HashMap<String, f32>,
    /// The sidebar is hidden (the web keeps this in localStorage).
    pub sidebar_collapsed: bool,
    /// The call bar is the small pill (the web's `vesper.callbar`).
    pub call_small: bool,
    /// Settings > Appearance, on this device only like the web's `vesper.scheme`: `dark`, `light` or `system`. An old
    /// file's `style` (Persona, Cozy, BBS, Aurora were removed) is an unknown key: ignored on load, dropped on save.
    pub scheme: String,
    /// Settings > Notifications: the ring of a DM call, and its volume, 0 to 100 (the web's `vesper.notify`).
    pub ring: bool,
    pub volume: u32,
    /// The share panel's layout, on this device only like the web's.
    pub share_layout: ShareLayout,
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
            gate_db: -50,
            input_device: None,
            output_device: None,
            start_with_computer: false,
            volumes: HashMap::new(),
            sidebar_collapsed: false,
            call_small: false,
            scheme: "dark".into(),
            ring: true,
            volume: 50,
            share_layout: ShareLayout::Grid,
        }
    }
}

impl Settings {
    pub fn clamped(mut self) -> Settings {
        self.bitrate_kbps = self.bitrate_kbps.clamp(32, 510);
        self.gate_db = self.gate_db.clamp(-100, 0);
        self.volume = self.volume.min(100);
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

    /// The settings file, or defaults when it is missing. A value this build cannot read (a newer build's mode, a
    /// typo) falls back on its own, keeping the rest. A file that is not JSON at all is kept as `settings.json.bad`.
    pub fn load(path: &Path) -> Settings {
        let Ok(bytes) = std::fs::read(path) else {
            return Settings::default();
        };
        let Ok(Value::Object(mut file)) = serde_json::from_slice::<Value>(&bytes) else {
            let _ = std::fs::write(path.with_extension("json.bad"), &bytes);
            return Settings::default();
        };
        if let Some(Value::Object(v)) = file.get_mut("volumes") {
            v.retain(|_, n| n.is_number());
        }
        let Ok(Value::Object(mut s)) = serde_json::to_value(Settings::default()) else {
            unreachable!("settings are a JSON object");
        };
        for (k, v) in file {
            let old = s.insert(k.clone(), v);
            if serde_json::from_value::<Settings>(Value::Object(s.clone())).is_err() {
                match old {
                    Some(o) => s.insert(k, o),
                    None => s.remove(&k),
                };
            }
        }
        serde_json::from_value::<Settings>(Value::Object(s))
            .unwrap_or_default()
            .clamped()
    }

    /// Written to a temporary file and renamed over the old one, so a crash mid-write never leaves half a file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::to_vec_pretty(self).expect("settings are JSON");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|e| e.to_string())
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
            gate_db: -42,
            input_device: Some("USB mic".into()),
            output_device: None,
            start_with_computer: true,
            volumes: HashMap::from([("0123456789abcdef0123456789abcdef".into(), 250.0)]),
            sidebar_collapsed: true,
            call_small: true,
            scheme: "system".into(),
            ring: false,
            volume: 30,
            share_layout: ShareLayout::Dock,
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

        // Every mode is saved by its name; files from before RNNoise keep their mode.
        for (name, d) in [
            ("deepfilter", Denoise::DeepFilter),
            ("rnnoise", Denoise::Rnnoise),
            ("gate", Denoise::Gate),
            ("off", Denoise::Off),
        ] {
            let s = Settings {
                denoise: d,
                ..Settings::default()
            };
            s.save(&p).unwrap();
            assert!(
                std::fs::read_to_string(&p)
                    .unwrap()
                    .contains(&format!(r#""denoise": "{name}""#))
            );
            assert_eq!(Settings::load(&p), s);
        }

        // An old file's style is ignored: the look is Vesper with the scheme it had.
        std::fs::write(&p, r#"{"style": "cozy", "scheme": "light"}"#).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.scheme, "light");
        s.save(&p).unwrap();
        assert!(!std::fs::read_to_string(&p).unwrap().contains("style"));

        // One value this build cannot read falls back alone; the rest, volumes too, stay.
        std::fs::write(
            &p,
            r#"{"denoise": "future", "bitrate_kbps": -1, "push_to_talk": true, "volumes": {"a": 50, "b": "x"}}"#,
        )
        .unwrap();
        let s = Settings::load(&p);
        assert_eq!(
            (s.denoise, s.bitrate_kbps, s.push_to_talk),
            (Denoise::DeepFilter, 64, true)
        );
        assert_eq!(s.volumes, HashMap::from([("a".into(), 50.0)]));

        std::fs::write(&p, "not json").unwrap();
        assert_eq!(Settings::load(&p), Settings::default());
        assert_eq!(
            std::fs::read_to_string(p.with_extension("json.bad")).unwrap(),
            "not json",
            "kept for the user"
        );
        Settings::default().save(&p).unwrap();
        assert!(!p.with_extension("json.tmp").exists());
    }
}
