use nnnoiseless::DenoiseState;

use crate::{DspError, HOP};

/// RNNoise (the `nnnoiseless` port, model built in) on 10 ms frames. Much lighter than DeepFilterNet and needs no
/// download, but takes out less noise. Delays the output by one frame (480 samples, 10 ms).
pub struct Rnnoise {
    st: Box<DenoiseState<'static>>,
    buf: [f32; HOP],
    out: [f32; HOP],
}

impl Default for Rnnoise {
    fn default() -> Self {
        Self {
            st: DenoiseState::new(),
            buf: [0.0; HOP],
            out: [0.0; HOP],
        }
    }
}

impl Rnnoise {
    /// Takes `HOP` samples in -1..1 and returns `HOP` samples.
    pub fn process(&mut self, frame: &[f32]) -> Result<Vec<f32>, DspError> {
        if frame.len() != HOP {
            return Err(DspError::BadFrameLength { got: frame.len() });
        }
        // RNNoise works on i16-scaled floats.
        for (b, x) in self.buf.iter_mut().zip(frame) {
            *b = x * 32768.0;
        }
        self.st.process_frame(&mut self.out, &self.buf);
        Ok(self.out.iter().map(|x| x / 32768.0).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64, uniform, scaled to this RMS.
    fn noise(s: &mut u64, rms: f32) -> f32 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        ((*s >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * rms * 3f32.sqrt()
    }

    #[test]
    fn rejects_wrong_frame_length() {
        assert_eq!(
            Rnnoise::default().process(&[0.0; 100]),
            Err(DspError::BadFrameLength { got: 100 })
        );
    }

    #[test]
    fn silence_stays_finite() {
        let mut r = Rnnoise::default();
        for _ in 0..100 {
            let out = r.process(&[0.0; HOP]).unwrap();
            assert_eq!(out.len(), HOP);
            assert!(out.iter().all(|x| x.is_finite()));
        }
    }

    /// A speech-like voice (120 Hz harmonics with vibrato, 300 ms syllables, 200 ms pauses) plus white noise at
    /// -34 dBFS: the noise left in the pauses goes down, the syllables come through. The same signal as the desktop's
    /// DeepFilterNet test. The output is one frame late.
    #[test]
    fn reduces_noise_and_keeps_the_voice() {
        let rate = 48_000.0;
        let voice = |i: usize| -> f32 {
            let t = i as f32 / rate;
            let in_syllable = t % 0.5;
            if in_syllable >= 0.3 {
                return 0.0;
            }
            let env = (std::f32::consts::PI * in_syllable / 0.3).sin();
            let f0 = 120.0 * (1.0 + 0.03 * (std::f32::consts::TAU * 5.0 * t).sin());
            let phase = std::f32::consts::TAU * f0 * t;
            let x: f32 = (1..30).map(|k| (k as f32 * phase).sin() / k as f32).sum();
            0.08 * env * x
        };
        let mut r = Rnnoise::default();
        let mut s = 0x2545F4914F6CDD1D;
        let seconds = 4;
        let (mut input, mut hiss, mut output) = (vec![], vec![], vec![]);
        for f in 0..seconds * 100 {
            let n: Vec<f32> = (0..HOP).map(|_| noise(&mut s, 0.02)).collect();
            let x: Vec<f32> = (0..HOP).map(|i| voice(f * HOP + i) + n[i]).collect();
            output.extend(r.process(&x).unwrap());
            input.extend(x);
            hiss.extend(n);
        }
        // Skip the first second; the middle of each pause and syllable, so the delay and fades do not matter.
        let range = |from: f32, to: f32| {
            (2..seconds * 2).flat_map(move |k| {
                let base = k as f32 * 0.5;
                ((base + from) * rate) as usize..((base + to) * rate) as usize
            })
        };
        let e = |x: &[f32], from: f32, to: f32, lag: usize| -> f64 {
            range(from, to).map(|i| f64::from(x[i + lag]).powi(2)).sum()
        };
        let db = |a: f64, b: f64| 10.0 * (a / b).log10();
        let noise_down = db(e(&hiss, 0.35, 0.45, 0), e(&output, 0.35, 0.45, HOP));
        let voice_kept = db(e(&output, 0.1, 0.2, HOP), e(&input, 0.1, 0.2, 0));
        println!(
            "RNNoise: noise in the pauses down {noise_down:.1} dB, syllables at {voice_kept:.1} dB"
        );
        assert!(
            noise_down > 10.0,
            "noise down by over 10 dB, got {noise_down:.1}"
        );
        assert!(
            voice_kept > -6.0,
            "the voice comes through, got {voice_kept:.1}"
        );
    }
}
