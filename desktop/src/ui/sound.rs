//! The ring of an incoming DM call, the web's `chime('ring')` (`web/src/lib/ui/chime.ts`): E5 C5 E5 C5, 160 ms
//! triangle notes 40 ms apart, each a 10 ms rise then an exponential fall, at the web's default volume (50: a peak of
//! 0.08). It plays on the chosen speakers, on a thread of its own, apart from a call's audio.
//!
//! Not here: the web's other call sounds (join, leave, mute, connect) and the Settings switch and volume for them.

use std::sync::Arc;
use std::time::Duration;

use crate::voice::{AudioDevices, FRAME, RATE};

/// The ring repeats this often while the prompt shows (`RING_EVERY_MS`).
pub const RING_EVERY: Duration = Duration::from_millis(2500);

const NOTES: [f32; 4] = [659.25, 523.25, 659.25, 523.25];
const STEP: f32 = 0.16;
const GAP: f32 = 0.04;
const PEAK: f32 = 0.08;

/// The ring as 48 kHz mono samples.
pub fn ring() -> Vec<f32> {
    let rate = RATE as f32;
    let start = 0.01;
    let len = start + NOTES.len() as f32 * (STEP + GAP) - GAP;
    (0..(len * rate).round() as usize)
        .map(|n| {
            let s = n as f32 / rate;
            NOTES
                .iter()
                .enumerate()
                .map(|(i, hz)| {
                    let t = s - (start + i as f32 * (STEP + GAP));
                    if !(0.0..STEP).contains(&t) {
                        return 0.0;
                    }
                    let level = if t < 0.01 {
                        PEAK * t / 0.01
                    } else {
                        PEAK * (0.0001 / PEAK).powf((t - 0.01) / (STEP - 0.01))
                    };
                    let phase = (hz * t).fract();
                    level * (4.0 * (phase - 0.5).abs() - 1.0)
                })
                .sum()
        })
        .collect()
}

/// Plays the ring once on `speakers` (none: the default). A device that does not open plays nothing.
pub fn play_ring(audio: Arc<dyn AudioDevices>, speakers: Option<String>) {
    let _ = std::thread::Builder::new()
        .name("vesper-ring".into())
        .spawn(move || {
            let Ok(mut out) = audio.output(speakers.as_deref()) else {
                return;
            };
            // In 20 ms steps: the output keeps 200 ms at most.
            for chunk in ring().chunks(FRAME) {
                out.write(chunk);
                std::thread::sleep(Duration::from_millis(20));
            }
            std::thread::sleep(Duration::from_millis(250));
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_ring_is_four_quiet_notes() {
        let r = super::ring();
        assert_eq!(r.len(), 36_960, "770 ms at 48 kHz");
        let peak = r.iter().fold(0f32, |a, s| a.max(s.abs()));
        assert!(peak > 0.07 && peak <= 0.08, "{peak}");
        assert_eq!(r[0], 0.0, "it starts silent");
        // The gap between the first two notes is silent.
        assert!(
            r[(0.175 * 48_000.) as usize..(0.205 * 48_000.) as usize]
                .iter()
                .all(|s| s.abs() < 1e-3)
        );
    }
}
