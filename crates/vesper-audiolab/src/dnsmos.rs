//! DNSMOS P.835 (SIG, BAK, OVRL), a Rust port of the non-personalised path of Microsoft's
//! `DNSMOS/dnsmos_local.py` (DNS-Challenge master; fetched by research/scripts/fetch-dnsmos.sh).
//! The `sig_bak_ovr.onnx` model takes raw 16 kHz audio, so there is no mel step (that one is P.808 only).

use std::path::Path;

use anyhow::{Context, Result, bail};
use tract_onnx::prelude::*;

pub const FS: usize = 16_000;
/// `INPUT_LENGTH = 9.01` seconds in the reference.
pub const INPUT_LENGTH: f64 = 9.01;
/// `int(INPUT_LENGTH * fs)` = 144160 samples.
pub const LEN_SAMPLES: usize = 144_160;

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Scores {
    pub sig: f64,
    pub bak: f64,
    pub ovrl: f64,
}

/// The reference's `np.poly1d` fits for the non-personalised model, highest power first.
const P_SIG: [f64; 3] = [-0.08397278, 1.22083953, 0.0052439];
const P_BAK: [f64; 3] = [-0.13166888, 1.60915514, -0.39604546];
const P_OVR: [f64; 3] = [-0.06766283, 1.11546468, 0.04602535];

fn poly(p: [f64; 3], x: f64) -> f64 {
    (p[0] * x + p[1]) * x + p[2]
}

/// Raw model outputs (sig, bak, ovr) to MOS, as `get_polyfit_val(..., is_personalized_MOS=False)`.
pub fn polyfit(raw: [f64; 3]) -> Scores {
    Scores {
        sig: poly(P_SIG, raw[0]),
        bak: poly(P_BAK, raw[1]),
        ovrl: poly(P_OVR, raw[2]),
    }
}

/// The windows the reference scores, as sample ranges into the (possibly doubled) audio.
/// Short audio is doubled (`np.append(audio, audio)`) until it is at least 9.01 s.
/// `num_hops = int(floor(len / fs) - 9.01) + 1`, hop 1 s, and segments shorter than 144160
/// samples are skipped. Float rounding makes hops 7 and up 144159 samples long, so the reference
/// skips them; this port does the same on purpose.
pub fn windows(len: usize) -> Vec<std::ops::Range<usize>> {
    let num_hops = ((len as f64 / FS as f64).floor() - INPUT_LENGTH) as i64 + 1;
    (0..num_hops.max(0) as usize)
        .map(|idx| idx * FS..((idx as f64 + INPUT_LENGTH) * FS as f64) as usize)
        .filter(|r| r.end.min(len) - r.start >= LEN_SAMPLES)
        .collect()
}

type Plan = TypedSimplePlan<TypedModel>;

pub struct Dnsmos {
    plan: Plan,
}

impl Dnsmos {
    pub fn load(path: &Path) -> Result<Self> {
        let plan = tract_onnx::onnx()
            .model_for_path(path)
            .with_context(|| {
                format!(
                    "load DNSMOS model {} (run research/scripts/fetch-dnsmos.sh)",
                    path.display()
                )
            })?
            .with_input_fact(0, f32::fact([1, LEN_SAMPLES]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self { plan })
    }

    /// Scores 16 kHz audio: the mean of the per-window MOS values, as the reference's `OVRL`, `SIG`, `BAK`.
    pub fn score_16k(&self, audio: &[f32]) -> Result<Scores> {
        if audio.is_empty() {
            bail!("empty audio");
        }
        let mut a = audio.to_vec();
        while a.len() < LEN_SAMPLES {
            a.extend_from_within(..);
        }
        let wins = windows(a.len());
        let mut sum = Scores {
            sig: 0.0,
            bak: 0.0,
            ovrl: 0.0,
        };
        for w in &wins {
            let input =
                tract_ndarray::Array2::from_shape_vec((1, LEN_SAMPLES), a[w.clone()].to_vec())?;
            let out = self.plan.run(tvec!(Tensor::from(input).into()))?;
            let v = out[0].to_array_view::<f32>()?;
            let v: Vec<f32> = v.iter().copied().collect();
            if v.len() != 3 {
                bail!("DNSMOS output has {} values, expected 3", v.len());
            }
            let s = polyfit([v[0] as f64, v[1] as f64, v[2] as f64]);
            sum.sig += s.sig;
            sum.bak += s.bak;
            sum.ovrl += s.ovrl;
        }
        let n = wins.len() as f64;
        Ok(Scores {
            sig: sum.sig / n,
            bak: sum.bak / n,
            ovrl: sum.ovrl / n,
        })
    }

    /// Scores 48 kHz audio (resampled to 16 kHz with the 3x low-pass first).
    pub fn score_48k(&self, audio: &[f32]) -> Result<Scores> {
        self.score_16k(&crate::audio::down3(audio))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_count_follows_the_reference() {
        // Python: int(9.01 * 16000) == 144160.
        assert_eq!((INPUT_LENGTH * FS as f64) as usize, LEN_SAMPLES);
        // A 10 s item (minus the pipeline delay) gives one window at the start.
        assert_eq!(windows(159_520), vec![0..144_160]);
        assert_eq!(windows(160_000), vec![0..144_160]);
        // 20 s: 11 hops, but hops 7..=10 end at 256159.. (144159 long) and are skipped.
        let w = windows(320_000);
        assert_eq!(w.len(), 7);
        assert_eq!(w[6], 96_000..240_160);
        assert!(windows(100).is_empty());
    }

    fn model() -> Dnsmos {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../research/models/sig_bak_ovr.onnx"
        );
        Dnsmos::load(Path::new(p)).unwrap()
    }

    #[test]
    fn same_input_gives_identical_scores() {
        // 10 s at 48 kHz: an amplitude-modulated harmonic tone over quiet noise.
        let mut s: u32 = 9;
        let x: Vec<f32> = (0..480_000)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let t = i as f64 / 48_000.0;
                let v = (1.0 + (2.0 * std::f64::consts::PI * 4.0 * t).sin())
                    * (1..6)
                        .map(|h| {
                            (2.0 * std::f64::consts::PI * 150.0 * h as f64 * t).sin() / h as f64
                        })
                        .sum::<f64>();
                (0.05 * v + 0.002 * (s as f64 / u32::MAX as f64 - 0.5)) as f32
            })
            .collect();
        let (a, b) = (model(), model());
        let sa = a.score_48k(&x).unwrap();
        assert_eq!(sa, a.score_48k(&x).unwrap());
        assert_eq!(sa, b.score_48k(&x).unwrap());
        for v in [sa.sig, sa.bak, sa.ovrl] {
            assert!((1.0..=5.0).contains(&v), "{sa:?}");
        }
    }

    #[test]
    fn polyfit_matches_numpy_poly1d() {
        // np.poly1d([-0.06766283, 1.11546468, 0.04602535])(3.0)
        let s = polyfit([3.0, 3.0, 3.0]);
        assert!((s.ovrl - 2.78345392).abs() < 1e-8, "{}", s.ovrl);
        assert!((s.sig - 2.91200747).abs() < 1e-8, "{}", s.sig);
        assert!((s.bak - 3.24640004).abs() < 1e-8, "{}", s.bak);
    }
}
