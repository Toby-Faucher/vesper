use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::Array2;

use crate::{DspError, HOP};

pub struct Denoiser {
    df: DfTract,
}

impl Denoiser {
    pub fn new(
        model_tar_gz: &[u8],
        atten_db: f32,
        post_filter_beta: f32,
    ) -> Result<Self, DspError> {
        let params =
            DfParams::from_bytes(model_tar_gz).map_err(|e| DspError::Model(format!("{e:#}")))?;
        let rt = RuntimeParams::default_with_ch(1).with_atten_lim(atten_db);
        let mut df = DfTract::new(params, &rt).map_err(|e| DspError::Model(format!("{e:#}")))?;
        df.set_pf_beta(post_filter_beta);
        Ok(Self { df })
    }

    pub fn process(&mut self, input: &[f32]) -> Result<Vec<f32>, DspError> {
        if input.len() != HOP {
            return Err(DspError::BadFrameLength { got: input.len() });
        }
        let noisy = Array2::from_shape_vec((1, HOP), input.to_vec()).expect("shape matches HOP");
        let mut enh = Array2::<f32>::zeros((1, HOP));
        self.df
            .process(noisy.view(), enh.view_mut())
            .map_err(|e| DspError::Model(format!("{e:#}")))?;
        Ok(enh.into_raw_vec())
    }
}
