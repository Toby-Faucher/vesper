//! Playback maths: per-person volume, the limiter after the mix, the speaking detector, and speech while muted.

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

/// "You are muted": speech on the mic while muted by hand, the web's `mutedSpeech` (`web/src/lib/audio/muted-speech.ts`),
/// with the same numbers. The idea is Mumble's; this is our own code. It reads one level per frame on this device and
/// never sends anything.
#[derive(Debug, Default)]
pub struct MutedSpeech {
    /// Ms since the call started, counted in frames.
    now: u64,
    /// Ms of speech heard, net of quiet, at most `SUSTAIN_MS`.
    heard: u64,
    /// When listening began: the grace counts from it.
    from: Option<u64>,
    /// When the last notice showed: the cool down counts from it, for the whole call.
    shown_at: Option<u64>,
}

impl MutedSpeech {
    /// Speech must add up to this before the notice shows; each moment of quiet takes back as much.
    pub const SUSTAIN_MS: u64 = 500;
    /// Nothing counts this long after muting, so the end of a sentence does not show it.
    pub const GRACE_MS: u64 = 1000;
    /// The notice stays up this long, or until unmuted. Muting again does not bring it back.
    pub const SHOW_MS: u64 = 5000;
    /// After a notice, the next one waits this long.
    pub const COOLDOWN_MS: u64 = 60_000;

    /// One frame of `ms`. `listening`: muted by hand, not deafened, the setting on. `level`: the frame's RMS, speech
    /// above the speaking ring's `Speaking::ON`. Returns whether the notice shows.
    pub fn update(&mut self, ms: u64, listening: bool, level: f32) -> bool {
        self.now += ms;
        if !listening {
            (self.heard, self.from) = (0, None);
            return false;
        }
        let from = *self.from.get_or_insert(self.now);
        let start = from + Self::GRACE_MS;
        let step = if self.now > start {
            ms.min(self.now - start)
        } else {
            0
        };
        self.heard = if level > Speaking::ON {
            (self.heard + step).min(Self::SUSTAIN_MS)
        } else {
            self.heard.saturating_sub(step)
        };
        let cooled = self
            .shown_at
            .is_none_or(|t| self.now - t >= Self::COOLDOWN_MS);
        if self.heard >= Self::SUSTAIN_MS && cooled {
            (self.shown_at, self.heard) = (Some(self.now), 0);
        }
        self.shown_at
            .is_some_and(|t| t >= from && self.now - t < Self::SHOW_MS)
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

    /// Frames of 20 ms for `ms`, with `level(i)`; when the notice first showed, in ms from the start of the run.
    fn run(
        m: &mut MutedSpeech,
        ms: u64,
        listening: bool,
        level: impl Fn(u64) -> f32,
    ) -> Option<u64> {
        let mut shown = None;
        for i in 0..ms / 20 {
            if m.update(20, listening, level(i)) && shown.is_none() {
                shown = Some((i + 1) * 20);
            }
        }
        shown
    }
    const TALK: f32 = 0.1;

    #[test]
    fn muted_speech_shows_after_the_grace_and_half_a_second_then_hides() {
        let mut m = MutedSpeech::default();
        let at = run(&mut m, 3000, true, |_| TALK);
        assert_eq!(
            at,
            Some(MutedSpeech::GRACE_MS + MutedSpeech::SUSTAIN_MS + 20),
            "the first frame after the grace starts counting"
        );
        assert!(m.update(20, true, TALK), "still up");
        run(&mut m, MutedSpeech::SHOW_MS, true, |_| TALK);
        assert!(!m.update(20, true, TALK), "gone after SHOW_MS");
    }

    #[test]
    fn muted_speech_ignores_clicks_and_short_sounds() {
        let mut m = MutedSpeech::default();
        // Sound in `on` of every `of` frames.
        let beat = |on: u64, of: u64| move |i: u64| if i % of < on { TALK } else { 0.0 };
        assert_eq!(run(&mut m, 60_000, true, beat(1, 3)), None, "clicks");
        assert_eq!(
            run(&mut m, 60_000, true, beat(20, 50)),
            None,
            "400 ms on, 600 off"
        );
        assert_eq!(
            run(&mut m, 60_000, true, |_| Speaking::ON),
            None,
            "the threshold itself is not speech"
        );
        // Words with short gaps do add up.
        assert!(run(&mut m, 3000, true, beat(3, 4)).is_some());
    }

    #[test]
    fn muted_speech_cools_down_and_unmuting_hides_it() {
        let mut m = MutedSpeech::default();
        assert!(run(&mut m, 1600, true, |_| TALK).is_some());
        assert!(!m.update(20, false, TALK), "unmuted: hidden at once");
        assert_eq!(run(&mut m, 2000, false, |_| TALK), None, "not listening");
        assert_eq!(
            run(&mut m, 30_000, true, |_| TALK),
            None,
            "muted again inside the cool down"
        );
        assert!(
            run(&mut m, MutedSpeech::COOLDOWN_MS, true, |_| TALK).is_some(),
            "after it"
        );
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
