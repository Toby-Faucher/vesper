//! Playback maths: per-person volume, the limiter after the mix, and the speaking detector.

/// The per-person volume slider's top, in percent.
pub const MAX_VOLUME: f32 = 400.0;

/// Slider percent (0 to 400) to linear gain, as `web/src/lib/volume.ts`: quadratic to 1.0 at 100, then even dB steps to
/// +20 dB (10x) at 400.
pub fn volume_to_gain(v: f32) -> f32 {
    if v.is_nan() {
        return 1.0;
    }
    let c = v.clamp(0.0, MAX_VOLUME);
    if c <= 100.0 {
        (c / 100.0).powi(2)
    } else {
        10f32.powf((c - 100.0) / 300.0)
    }
}

/// Root mean square of `x`.
pub fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

/// The input level of `MicCheck.svelte`: the peak of 100 ms of 20 ms frames, in dBFS, -100 for silence.
#[derive(Debug, Default)]
pub struct Meter {
    peak: f32,
    frames: u32,
}

impl Meter {
    /// Adds a frame; every fifth returns the level.
    pub fn update(&mut self, frame: &[f32]) -> Option<f32> {
        self.peak = frame.iter().fold(self.peak, |p, x| p.max(x.abs()));
        self.frames += 1;
        if self.frames < 5 {
            return None;
        }
        self.frames = 0;
        let peak = std::mem::take(&mut self.peak);
        Some(if peak > 0.0 {
            (20.0 * peak.log10()).max(-100.0)
        } else {
            -100.0
        })
    }
}

/// Adds `src` times `gain` into `out`.
pub fn mix_into(out: &mut [f32], src: &[f32], gain: f32) {
    for (o, s) in out.iter_mut().zip(src) {
        *o += s * gain;
    }
}

/// A peak limiter at -3 dBFS after the volumes, like the web's compressor: it cuts at once, so no sample goes over the
/// ceiling, and lets go over about 50 ms.
#[derive(Debug)]
pub struct Limiter {
    gain: f32,
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter { gain: 1.0 }
    }
}

impl Limiter {
    pub const CEILING: f32 = 0.708;
    /// Per sample at 48 kHz: 1 - e^(-1 / (0.05 s * 48000)).
    const RELEASE: f32 = 0.000_416;

    pub fn process(&mut self, buf: &mut [f32]) {
        for x in buf {
            let peak = x.abs();
            let want = if peak > Self::CEILING {
                Self::CEILING / peak
            } else {
                1.0
            };
            self.gain = (self.gain + (1.0 - self.gain) * Self::RELEASE).min(want);
            *x *= self.gain;
        }
    }
}

/// Speaking, from the level of each 20 ms frame, with hysteresis: on above `ON` at once, off only after `HOLD` frames
/// in a row under `OFF`, so the ring does not flicker between words.
#[derive(Debug, Default)]
pub struct Speaking {
    on: bool,
    quiet: u32,
}

impl Speaking {
    /// The web app's speaking threshold (`isSpeaking` in `volume.ts`).
    pub const ON: f32 = 0.02;
    pub const OFF: f32 = 0.01;
    /// 300 ms.
    pub const HOLD: u32 = 15;

    pub fn update(&mut self, rms: f32) -> bool {
        if rms > Self::ON {
            (self.on, self.quiet) = (true, 0);
        } else if self.on && rms < Self::OFF {
            self.quiet += 1;
            if self.quiet >= Self::HOLD {
                (self.on, self.quiet) = (false, 0);
            }
        } else {
            self.quiet = 0;
        }
        self.on
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_matches_the_web_curve() {
        assert_eq!(volume_to_gain(0.0), 0.0);
        assert_eq!(volume_to_gain(50.0), 0.25);
        assert_eq!(volume_to_gain(100.0), 1.0);
        assert!((volume_to_gain(400.0) - 10.0).abs() < 1e-4);
        assert!((volume_to_gain(900.0) - 10.0).abs() < 1e-4, "clamped");
        assert_eq!(volume_to_gain(f32::NAN), 1.0);
    }

    #[test]
    fn mixer_adds_with_gain() {
        let mut out = [0.25, 0.5];
        mix_into(&mut out, &[0.5, -0.25], 2.0);
        assert_eq!(out, [1.25, 0.0]);
    }

    #[test]
    fn limiter_keeps_peaks_under_the_ceiling_and_leaves_quiet_audio_alone() {
        let mut l = Limiter::default();
        let mut quiet: Vec<f32> = (0..480).map(|i| 0.5 * (i as f32 * 0.1).sin()).collect();
        let before = quiet.clone();
        l.process(&mut quiet);
        assert_eq!(quiet, before);

        // 400 % on a loud voice: a 10x sine.
        let mut loud: Vec<f32> = (0..4800).map(|i| 9.0 * (i as f32 * 0.05).sin()).collect();
        l.process(&mut loud);
        assert!(loud.iter().all(|x| x.abs() <= Limiter::CEILING + 1e-6));
        assert!(rms(&loud) > 0.3, "still audible: {}", rms(&loud));

        // It lets go again: a quiet signal after a second comes back to unity.
        let mut after = vec![0.1; 48_000];
        l.process(&mut after);
        assert!((after[47_999] - 0.1).abs() < 1e-3);
    }

    #[test]
    fn speaking_has_hysteresis() {
        let mut s = Speaking::default();
        assert!(
            !s.update(0.015),
            "between the thresholds does not turn it on"
        );
        assert!(s.update(0.05));
        // Between OFF and ON keeps it on for as long as it lasts.
        for _ in 0..50 {
            assert!(s.update(0.015));
        }
        for _ in 0..Speaking::HOLD - 1 {
            assert!(s.update(0.0));
        }
        assert!(s.update(0.05), "a word before the hold ends keeps it on");
        for _ in 0..Speaking::HOLD - 1 {
            assert!(s.update(0.0));
        }
        assert!(!s.update(0.0), "off after the hold");
    }

    #[test]
    fn rms_of_a_sine() {
        let x: Vec<f32> = (0..9600)
            .map(|i| (i as f32 * 2.0 * std::f32::consts::PI * 440.0 / 48_000.0).sin())
            .collect();
        assert!((rms(&x) - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-3);
        assert_eq!(rms(&[]), 0.0);
    }
}
