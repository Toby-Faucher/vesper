use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;

use crate::{DspError, HOP};

/// Settings for a [`Pipeline`]. `Default` reproduces the shipped denoiser bit for bit.
#[derive(Clone, Debug)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default, deny_unknown_fields)
)]
pub struct PipelineConfig {
    /// DeepFilterNet attenuation limit in dB. 100 or more means no limit.
    pub atten_db: f32,
    /// DeepFilterNet post filter strength. 0 turns it off.
    pub post_filter_beta: f32,
    /// Local SNR below which the model outputs silence. None keeps the library default (-10 dB).
    pub min_db_thresh: Option<f32>,
    /// Local SNR above which the ERB stage is skipped. None keeps the library default (30 dB).
    pub max_db_erb_thresh: Option<f32>,
    /// Local SNR above which the deep filter stage is skipped. None keeps the library default (20 dB).
    pub max_db_df_thresh: Option<f32>,
    /// Gain applied to the input before anything else, so before DeepFilterNet.
    /// The web client's input boost is a gain node after DeepFilterNet (`capture.ts`), so it is not the same.
    pub input_gain_db: f32,
    pub pre: PreConfig,
    pub post: PostConfig,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            atten_db: 100.0,
            post_filter_beta: 0.02,
            min_db_thresh: None,
            max_db_erb_thresh: None,
            max_db_df_thresh: None,
            input_gain_db: 0.0,
            pre: PreConfig::default(),
            post: PostConfig::default(),
        }
    }
}

/// Stages that run before DeepFilterNet. Empty for now: experiments add fields here
/// (a high-pass, for example). Each stage must be off by default.
#[derive(Clone, Debug, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default, deny_unknown_fields)
)]
#[non_exhaustive]
pub struct PreConfig {}

/// Stages that run after DeepFilterNet. Empty for now: experiments add fields here
/// (a limiter, for example). Each stage must be off by default.
#[derive(Clone, Debug, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default, deny_unknown_fields)
)]
#[non_exhaustive]
pub struct PostConfig {}

/// The capture path for one mono 48 kHz stream: input gain, pre stages, DeepFilterNet, post stages.
/// This is the one code path through DeepFilterNet; `Denoiser` and the WASM export use it.
pub struct Pipeline {
    df: DfTract,
    gain: f32,
    noisy: Array2<f32>,
    enh: Array2<f32>,
    lsnr: f32,
}

impl Pipeline {
    pub fn new(model_tar_gz: &[u8], cfg: &PipelineConfig) -> Result<Self, DspError> {
        let params =
            DfParams::from_bytes(model_tar_gz).map_err(|e| DspError::Model(format!("{e:#}")))?;
        let rt = RuntimeParams::default_with_ch(1).with_atten_lim(cfg.atten_db);
        let rt = RuntimeParams {
            min_db_thresh: cfg.min_db_thresh.unwrap_or(rt.min_db_thresh),
            max_db_erb_thresh: cfg.max_db_erb_thresh.unwrap_or(rt.max_db_erb_thresh),
            max_db_df_thresh: cfg.max_db_df_thresh.unwrap_or(rt.max_db_df_thresh),
            ..rt
        };
        let mut df = DfTract::new(params, &rt).map_err(|e| DspError::Model(format!("{e:#}")))?;
        df.set_pf_beta(cfg.post_filter_beta);
        Ok(Self {
            df,
            gain: 10f32.powf(cfg.input_gain_db / 20.0),
            noisy: Array2::zeros((1, HOP)),
            enh: Array2::zeros((1, HOP)),
            lsnr: 0.0,
        })
    }

    pub fn set_atten_limit(&mut self, db: f32) {
        self.df.set_atten_lim(db);
    }

    pub fn set_post_filter(&mut self, beta: f32) {
        self.df.set_pf_beta(beta);
    }

    /// Takes `HOP` samples and returns `HOP` samples.
    pub fn process(&mut self, frame: &[f32]) -> Result<Vec<f32>, DspError> {
        if frame.len() != HOP {
            return Err(DspError::BadFrameLength { got: frame.len() });
        }
        for (n, x) in self.noisy.iter_mut().zip(frame) {
            *n = x * self.gain;
        }
        self.lsnr = self
            .df
            .process(self.noisy.view(), self.enh.view_mut())
            .map_err(|e| DspError::Model(format!("{e:#}")))?;
        Ok(self.enh.iter().copied().collect())
    }

    /// Delay in samples added by our own pre and post stages. 0 while there are none.
    /// The model's delay is separate: see [`Pipeline::model_latency_samples`].
    pub fn latency_samples(&self) -> usize {
        0
    }

    /// Delay in samples between input and output inside DeepFilterNet: the STFT overlap
    /// (`fft_size - hop_size`) plus the model lookahead in frames. 1440 (30 ms) for DeepFilterNet3.
    /// With the attenuation limit near 0 dB the library passes the input through with no delay.
    pub fn model_latency_samples(&self) -> usize {
        self.df.fft_size - self.df.hop_size + self.df.lookahead * self.df.hop_size
    }

    /// Local SNR in dB the model estimated for the last frame.
    pub fn last_lsnr(&self) -> f32 {
        self.lsnr
    }
}
