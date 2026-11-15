//! Echo cancellation on the microphone: `sonora`, a pure Rust port of WebRTC's audio processing, with only its echo
//! canceller (AEC3) on. It hears the far end (the exact mix sent to the speakers) and takes it out of the microphone.
//! AEC3 finds the delay between the two itself, and follows it when it changes, so the engine sets no delay.
//!
//! Like DeepFilterNet, it falls back to off for the rest of the call when it cannot keep up (`Late`).

use std::time::{Duration, Instant};

use sonora::config::EchoCanceller;
use sonora::{AudioProcessing, Config, StreamConfig};

use super::denoise::Late;

/// AEC3's block: 10 ms at 48 kHz.
const HOP: usize = 480;

pub const SLOW: &str = "Echo cancellation could not keep up on this computer, so it is off for the rest of the \
                        call. Use headphones, or the others may hear themselves.";

/// The echo canceller of one call, or nothing when it is off.
pub struct Echo {
    apm: Option<AudioProcessing>,
    late: Late,
    /// Render time not yet counted towards a capture block.
    render: Duration,
    /// The far end given so far, for the tests.
    #[cfg(test)]
    pub far: Vec<f32>,
}

impl Echo {
    pub fn new(on: bool, late: Late) -> Echo {
        let apm = on.then(|| {
            let mono = StreamConfig::new(super::RATE, 1);
            AudioProcessing::builder()
                .config(Config {
                    echo_canceller: Some(EchoCanceller::default()),
                    ..Default::default()
                })
                .capture_config(mono)
                .render_config(mono)
                .build()
        });
        Echo {
            apm,
            late,
            render: Duration::ZERO,
            #[cfg(test)]
            far: vec![],
        }
    }

    /// Off from the start, or after a fall back.
    pub fn is_on(&self) -> bool {
        self.apm.is_some()
    }

    /// What went to the speakers, a multiple of 10 ms.
    pub fn render(&mut self, far: &[f32]) {
        #[cfg(test)]
        self.far.extend_from_slice(far);
        let Some(apm) = &mut self.apm else { return };
        let t = Instant::now();
        let mut out = [0f32; HOP];
        for block in far.as_chunks::<HOP>().0 {
            let _ = apm.process_render_f32(&[block], &mut [&mut out]);
        }
        self.render += t.elapsed();
    }

    /// Takes the echo out of `near` (a multiple of 10 ms) in place. Returns the notice when it falls back.
    pub fn capture(&mut self, near: &mut [f32]) -> Option<&'static str> {
        let apm = self.apm.as_mut()?;
        let mut out = [0f32; HOP];
        for block in near.as_chunks_mut::<HOP>().0 {
            let t = Instant::now();
            if apm.process_capture_f32(&[block], &mut [&mut out]).is_ok() {
                block.copy_from_slice(&out);
            }
            let took = t.elapsed() + std::mem::take(&mut self.render);
            if self.late.report(took) {
                self.apm = None;
                return Some(SLOW);
            }
        }
        None
    }
}

/// The call's budget: 5 ms per 10 ms block, as it shares the 20 ms turn with DeepFilterNet and Opus. Measured at
/// about 0.5 ms (see the test below), so only a computer far too slow, or very busy, trips it.
pub fn late() -> Late {
    Late::new(Duration::from_millis(5), 100, 20, 50)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// xorshift, uniform in -1 to 1.
    struct Noise(u64);
    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0
        }
    }

    /// A speech-like voice: harmonics of `f0` with vibrato, in syllables of `on` seconds every `every` seconds, plus a
    /// little breath noise so it is not a pure tone.
    fn voice(f0: f32, on: f32, every: f32, level: f32, seed: u64) -> impl FnMut(usize) -> f32 {
        let mut n = Noise(seed);
        move |i| {
            let t = i as f32 / RATE;
            let s = t % every;
            let breath = 0.05 * n.next();
            if s >= on {
                return 0.0;
            }
            let env = (std::f32::consts::PI * s / on).sin();
            let f = f0 * (1.0 + 0.04 * (std::f32::consts::TAU * 4.0 * t).sin());
            let phase = std::f32::consts::TAU * f * t;
            let x: f32 = (1..25).map(|k| (k as f32 * phase).sin() / k as f32).sum();
            level * env * (x * 0.5 + breath)
        }
    }

    /// A small room as taps (delay in samples, gain): the direct sound 30 ms after the speaker (sound path plus
    /// device buffers) at -6 dB, then 60 reflections that die away over the next 60 ms.
    fn room() -> Vec<(usize, f32)> {
        let mut n = Noise(99);
        let mut h = vec![(48 * 30, 0.5)];
        for k in 0..60 {
            let at = 48 * 31 + k * 48 + (n.next().abs() * 40.0) as usize;
            let after = (at - 48 * 31) as f32 / RATE;
            h.push((at, 0.2 * n.next() * (-after / 0.015).exp()));
        }
        h
    }

    fn convolve(x: &[f32], h: &[(usize, f32)]) -> Vec<f32> {
        let mut y = vec![0f32; x.len()];
        for &(d, g) in h {
            for (o, s) in y[d..].iter_mut().zip(x) {
                *o += g * s;
            }
        }
        y
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|s| f64::from(*s).powi(2)).sum()
    }

    fn db(a: f64, b: f64) -> f64 {
        10.0 * (a / b).log10()
    }

    /// The far end plays and comes back through the room into the microphone, with the near end talking over it
    /// from second 6. After AEC3: the echo alone (seconds 3 to 6, once converged) is down by over 15 dB, and in the
    /// double talk the near voice keeps most of its level. Measured on the dev machine: echo down 34 dB in the first
    /// second and 50 dB once converged; in double talk the near voice comes out at -3.9 dB with the echo down 54 dB.
    #[test]
    fn the_echo_of_a_synthetic_room_drops_and_the_near_voice_survives() {
        let seconds = 9;
        let len = seconds * RATE as usize;
        let mut far_voice = voice(140.0, 0.35, 0.5, 0.3, 1);
        let far: Vec<f32> = (0..len).map(&mut far_voice).collect();
        let echo = convolve(&far, &room());
        let mut near_voice = voice(210.0, 0.25, 0.4, 0.4, 2);
        let near: Vec<f32> = (0..len)
            .map(|i| if i >= 6 * 48_000 { near_voice(i) } else { 0.0 })
            .collect();

        // The same room twice, with and without the near voice: their difference is what the near voice became.
        let run = |near: &[f32]| -> (Vec<f32>, f64) {
            // A budget no debug build misses: this test is about the sound, the fall back has its own.
            let mut e = Echo::new(true, Late::new(Duration::from_secs(1), 100, 20, 0));
            let mut out = Vec::with_capacity(len);
            let t = Instant::now();
            for i in (0..len).step_by(super::super::FRAME) {
                let r = i..i + super::super::FRAME;
                e.render(&far[r.clone()]);
                let mut mic: Vec<f32> = echo[r.clone()]
                    .iter()
                    .zip(&near[r])
                    .map(|(a, b)| a + b)
                    .collect();
                assert_eq!(e.capture(&mut mic), None);
                out.extend(mic);
            }
            (out, t.elapsed().as_secs_f64() * 1000.0 / (len / HOP) as f64)
        };
        let (out, ms) = run(&near);
        let (alone, _) = run(&vec![0.0; len]);

        let s = |a: usize, b: usize| a * 48_000..b * 48_000;
        let echo_only = db(energy(&echo[s(3, 6)]), energy(&out[s(3, 6)]));
        let first = db(energy(&echo[s(0, 1)]), energy(&out[s(0, 1)]));
        let r = s(6, 9);
        let came: Vec<f32> = out[r.clone()]
            .iter()
            .zip(&alone[r.clone()])
            .map(|(a, b)| a - b)
            .collect();
        let kept = db(energy(&came), energy(&near[r.clone()]));
        let over = db(energy(&near[r.clone()]), energy(&echo[r.clone()]));
        let left = db(energy(&echo[r.clone()]), energy(&alone[r]));
        println!(
            "AEC3 on a synthetic room: echo down {first:.1} dB in the first second, {echo_only:.1} dB once converged \
             (3 to 6 s); in double talk (near voice {over:.1} dB over the echo) the near voice comes out at {kept:.1} \
             dB and the echo is down {left:.1} dB; {ms:.3} ms per 10 ms block (this build)",
        );
        assert!(echo_only > 15.0, "echo down by over 15 dB: {echo_only:.1}");
        assert!(kept > -6.0, "the near voice survives: {kept:.1} dB");
    }

    #[test]
    fn off_passes_the_audio_and_a_slow_canceller_turns_off_with_a_notice() {
        let mut e = Echo::new(false, Late::default());
        let mut x: Vec<f32> = (0..960).map(|i| (i as f32 * 0.01).sin()).collect();
        let before = x.clone();
        e.render(&before);
        assert_eq!(e.capture(&mut x), None);
        assert_eq!(x, before);
        assert!(!e.is_on());

        // A zero budget: every block is late, so it trips on the third.
        let mut e = Echo::new(true, Late::new(Duration::ZERO, 10, 3, 0));
        e.render(&before);
        assert_eq!(e.capture(&mut x), None, "two blocks");
        assert_eq!(e.capture(&mut x), Some(SLOW));
        assert!(!e.is_on());
        let mut y = before.clone();
        assert_eq!(e.capture(&mut y), None);
        assert_eq!(y, before, "off now");
    }
}
