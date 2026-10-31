use crate::{DspError, Pipeline, PipelineConfig};

/// The shipped denoiser: a [`Pipeline`] with the default config apart from the two DeepFilterNet knobs.
pub struct Denoiser(Pipeline);

impl Denoiser {
    pub fn new(
        model_tar_gz: &[u8],
        atten_db: f32,
        post_filter_beta: f32,
    ) -> Result<Self, DspError> {
        let cfg = PipelineConfig {
            atten_db,
            post_filter_beta,
            ..PipelineConfig::default()
        };
        Ok(Self(Pipeline::new(model_tar_gz, &cfg)?))
    }

    pub fn set_atten_limit(&mut self, db: f32) {
        self.0.set_atten_limit(db);
    }

    pub fn set_post_filter(&mut self, beta: f32) {
        self.0.set_post_filter(beta);
    }

    pub fn process(&mut self, input: &[f32]) -> Result<Vec<f32>, DspError> {
        self.0.process(input)
    }
}
