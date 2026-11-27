#![forbid(unsafe_code)]
//! Native cost of the capture denoisers, for the benchmark suite (`docs/benchmarks/README.md`).
//! In the app DeepFilterNet runs in the browser as WASM; this times the same Rust code natively,
//! so it tracks changes in our code and the model, not the browser's speed.
//!
//!   cargo run --release -p vesper-dsp --features tract --example dsp_bench -- MODEL.tar.gz [SECONDS] [RUNS]
//!
//! Prints one JSON object: milliseconds of CPU per 10 ms frame for each run, per engine.

use std::time::Instant;

use vesper_dsp::{Denoiser, HOP, Rnnoise};

/// Speech-band tone plus white noise from a fixed LCG, so every run sees the same input.
fn signal(frames: usize) -> Vec<f32> {
    let mut seed: u32 = 0x1234_5678;
    (0..frames * HOP)
        .map(|i| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5;
            let t = i as f32 / 48_000.0;
            0.3 * (2.0 * std::f32::consts::PI * 220.0 * t).sin() + 0.1 * noise
        })
        .collect()
}

/// Runs `process` over every frame `runs` times after one warm-up pass; ms per frame for each run.
fn time(
    audio: &[f32],
    runs: usize,
    mut process: impl FnMut(&[f32]) -> Result<Vec<f32>, vesper_dsp::DspError>,
) -> Result<Vec<f64>, String> {
    let frames = audio.len() / HOP;
    let mut out = vec![];
    for run in 0..=runs {
        let start = Instant::now();
        for f in audio.as_chunks::<HOP>().0 {
            std::hint::black_box(process(f.as_slice()).map_err(|e| e.to_string())?);
        }
        if run > 0 {
            out.push(start.elapsed().as_secs_f64() * 1000.0 / frames as f64);
        }
    }
    Ok(out)
}

fn list(v: &[f64]) -> String {
    let items: Vec<String> = v.iter().map(|x| format!("{x:.5}")).collect();
    format!("[{}]", items.join(", "))
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: dsp_bench MODEL.tar.gz [SECONDS] [RUNS]";
    let model = std::fs::read(args.first().ok_or(usage)?).map_err(|e| e.to_string())?;
    let num = |i: usize, d: usize| args.get(i).map_or(Ok(d), |v| v.parse().map_err(|_| usage));
    let (seconds, runs) = (num(1, 5)?, num(2, 5)?);
    let audio = signal(seconds * 100);
    let mut dfn = Denoiser::new(&model, 100.0, 0.0).map_err(|e| e.to_string())?;
    let deepfilternet = time(&audio, runs, |f| dfn.process(f))?;
    let mut rnn = Rnnoise::default();
    let rnnoise = time(&audio, runs, |f| rnn.process(f))?;
    println!(
        "{{\"frame_ms\": 10, \"seconds\": {seconds}, \"deepfilternet_ms_per_frame\": {}, \"rnnoise_ms_per_frame\": {}}}",
        list(&deepfilternet),
        list(&rnnoise)
    );
    Ok(())
}
