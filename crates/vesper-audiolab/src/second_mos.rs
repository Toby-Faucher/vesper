//! A second non-intrusive judge: UTMOS22 strong (Saeki et al. 2022, MIT licence), the ONNX export
//! from huggingface.co/TigreGotico/utmos-onnx (fetched by research/scripts/fetch-utmos.sh).
//! Input: 16 kHz mono float in [-1, 1], output one MOS. A wav2vec2-style model trained on
//! listening tests of synthetic and natural speech (VoiceMOS 2022), not on enhancement output.

use std::path::Path;

use anyhow::{Context, Result, bail};
use tract_onnx::prelude::*;

/// The first 9 s at 16 kHz: every item and every Opus round trip is at least that long.
pub const LEN_16K: usize = 9 * 16_000;

pub struct Utmos {
    plan: TypedSimplePlan<TypedModel>,
}

impl Utmos {
    pub fn load(path: &Path) -> Result<Self> {
        let plan = tract_onnx::onnx()
            .model_for_path(path)
            .with_context(|| {
                format!(
                    "load UTMOS model {} (run research/scripts/fetch-utmos.sh)",
                    path.display()
                )
            })?
            .with_input_fact(0, f32::fact([1, LEN_16K]).into())?
            .into_optimized()?
            .into_runnable()?;
        Ok(Self { plan })
    }

    /// MOS of 48 kHz audio: resampled to 16 kHz, the first [`LEN_16K`] samples (short audio is
    /// repeated to that length, as DNSMOS does).
    pub fn score_48k(&self, audio: &[f32]) -> Result<f64> {
        let mut a = crate::audio::down3(audio);
        if a.is_empty() {
            bail!("empty audio");
        }
        while a.len() < LEN_16K {
            a.extend_from_within(..);
        }
        a.truncate(LEN_16K);
        let input = tract_ndarray::Array2::from_shape_vec((1, LEN_16K), a)?;
        let out = self.plan.run(tvec!(Tensor::from(input).into()))?;
        let v = out[0].to_array_view::<f32>()?;
        match v.iter().next() {
            Some(&m) if v.len() == 1 => Ok(m as f64),
            _ => bail!("UTMOS output has {} values, expected 1", v.len()),
        }
    }
}
