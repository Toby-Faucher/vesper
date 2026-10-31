#![forbid(unsafe_code)]
//! Thin wrapper over DeepFilterNet3 (tract runtime) for 10 ms mono frames at 48 kHz.

#[cfg(feature = "tract")]
mod denoiser;
#[cfg(feature = "tract")]
pub use denoiser::Denoiser;
#[cfg(feature = "wasm")]
mod wasm;

/// Samples per frame (10 ms at 48 kHz).
pub const HOP: usize = 480;

#[derive(Debug, PartialEq)]
pub enum DspError {
    BadFrameLength { got: usize },
    Model(String),
}

impl std::fmt::Display for DspError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadFrameLength { got } => write!(f, "frame must be {HOP} samples, got {got}"),
            Self::Model(e) => write!(f, "model error: {e}"),
        }
    }
}

impl std::error::Error for DspError {}

#[cfg(all(test, feature = "tract"))]
mod tests {
    use super::*;

    fn denoiser() -> Denoiser {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/model/DeepFilterNet3_onnx.tar.gz"
        );
        let bytes = std::fs::read(path).unwrap_or_else(|e| {
            panic!("model missing at {path} ({e}); run scripts/fetch-model.sh")
        });
        Denoiser::new(&bytes, 100.0, 0.0).unwrap()
    }

    /// Output of the shipped `Denoiser` (100 dB, beta 0.02) on `golden_input`, taken before `Pipeline` existed.
    const GOLDEN_HASH: u64 = 0x4ae8befc195842e3;

    /// 300 frames: xorshift noise plus a 100 Hz to 8 kHz sine sweep, rounded to i16 steps.
    fn golden_input() -> Vec<f32> {
        let n = 300 * HOP;
        let mut s: u64 = 0x9E3779B97F4A7C15;
        let mut phase = 0f64;
        (0..n)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                let noise = (s >> 40) as f64 / (1u64 << 23) as f64 - 1.0;
                let f = 100.0 * 80f64.powf(i as f64 / n as f64);
                phase += 2.0 * std::f64::consts::PI * f / 48_000.0;
                let x = 0.02 * noise + 0.1 * phase.sin();
                ((x * 32768.0).round() / 32768.0) as f32
            })
            .collect()
    }

    /// FNV-1a over the bits of every output sample.
    fn hash_output(mut process: impl FnMut(&[f32]) -> Vec<f32>, input: &[f32]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for frame in input.chunks(HOP) {
            for x in process(frame) {
                for b in x.to_bits().to_le_bytes() {
                    h ^= b as u64;
                    h = h.wrapping_mul(0x100000001b3);
                }
            }
        }
        h
    }

    #[test]
    fn denoiser_matches_golden_hash() {
        let mut d = denoiser();
        d.set_post_filter(0.02);
        let h = hash_output(|f| d.process(f).unwrap(), &golden_input());
        assert_eq!(h, GOLDEN_HASH, "got {h:#018x}");
    }

    #[test]
    fn rejects_wrong_frame_length() {
        assert_eq!(
            denoiser().process(&[0.0; 100]),
            Err(DspError::BadFrameLength { got: 100 })
        );
    }

    #[test]
    fn setters_keep_processing_and_attenuation_limit_mixes_noise_back() {
        let run = |limit: Option<f32>| {
            let mut d = denoiser();
            d.set_post_filter(0.02);
            if let Some(db) = limit {
                d.set_atten_limit(db);
            }
            let mut s: u32 = 12345;
            let mut e = 0f64;
            for i in 0..300 {
                let f: Vec<f32> = (0..HOP)
                    .map(|_| {
                        s = s.wrapping_mul(1664525).wrapping_add(1013904223);
                        ((s >> 8) as f32 / (1u32 << 23) as f32 - 1.0) * 0.05
                    })
                    .collect();
                let out = d.process(&f).unwrap();
                if i >= 50 {
                    e += out.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
                }
            }
            e
        };
        // a 6 dB limit keeps far more of the noise than no limit
        assert!(run(Some(6.0)) > 2.0 * run(None));
    }

    #[test]
    fn silence_stays_finite() {
        let mut d = denoiser();
        for _ in 0..100 {
            let out = d.process(&[0.0; HOP]).unwrap();
            assert_eq!(out.len(), HOP);
            assert!(out.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn reduces_white_noise() {
        let mut d = denoiser();
        let mut s: u64 = 0x2545F4914F6CDD1D;
        // xorshift64, uniform in [-1, 1) scaled to ~-30 dBFS RMS (0.0316)
        let amp = 0.0316 * 3f32.sqrt();
        let (mut ein, mut eout) = (0f64, 0f64);
        for i in 0..500 {
            let frame: Vec<f32> = (0..HOP)
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    ((s >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * amp
                })
                .collect();
            let out = d.process(&frame).unwrap();
            if i >= 50 {
                ein += frame.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
                eout += out.iter().map(|x| (*x as f64).powi(2)).sum::<f64>();
            }
        }
        let (rin, rout) = (
            (ein / 450.0 / HOP as f64).sqrt(),
            (eout / 450.0 / HOP as f64).sqrt(),
        );
        assert!(rout < 0.5 * rin, "in {rin} out {rout}");
    }
}
