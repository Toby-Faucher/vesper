use wasm_bindgen::prelude::*;

use crate::{Denoiser, Rnnoise};

#[wasm_bindgen]
pub struct WasmDenoiser(Denoiser);

#[wasm_bindgen]
impl WasmDenoiser {
    #[wasm_bindgen(constructor)]
    pub fn new(
        model: &[u8],
        atten_db: f32,
        post_filter_beta: f32,
    ) -> Result<WasmDenoiser, JsError> {
        Ok(Self(Denoiser::new(model, atten_db, post_filter_beta)?))
    }

    /// Throws if `input.len() != 480`.
    pub fn process(&mut self, input: &[f32]) -> Result<Vec<f32>, JsError> {
        Ok(self.0.process(input)?)
    }

    #[wasm_bindgen(js_name = setAttenLimit)]
    pub fn set_atten_limit(&mut self, db: f32) {
        self.0.set_atten_limit(db);
    }

    #[wasm_bindgen(js_name = setPostFilter)]
    pub fn set_post_filter(&mut self, beta: f32) {
        self.0.set_post_filter(beta);
    }
}

/// RNNoise: no model to load.
#[wasm_bindgen]
pub struct WasmRnnoise(Rnnoise);

#[wasm_bindgen]
impl WasmRnnoise {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> WasmRnnoise {
        Self(Rnnoise::default())
    }

    /// Throws if `input.len() != 480`.
    pub fn process(&mut self, input: &[f32]) -> Result<Vec<f32>, JsError> {
        Ok(self.0.process(input)?)
    }
}
