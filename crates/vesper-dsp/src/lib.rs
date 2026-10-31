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
