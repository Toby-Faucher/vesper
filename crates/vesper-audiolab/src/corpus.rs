//! Deterministic noisy mixtures: LibriSpeech speakers (disjoint across splits) plus recorded and
//! synthesised noise, on a balanced grid of noise type x SNR x level.

use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::audio::{self, SR, db, rms};

/// 10 s items.
pub const ITEM_LEN: usize = 10 * SR as usize;
/// 1.5 s of noise only before the speech starts.
pub const LEAD_IN: usize = 3 * SR as usize / 2;
pub const SNRS_DB: [f64; 5] = [-5.0, 0.0, 5.0, 10.0, 20.0];
pub const LEVELS_DBFS: [f64; 3] = [-35.0, -25.0, -15.0];
/// Peaks are kept at or below this. An item that would go over is scaled down as a whole and
/// its `level_dbfs_actual` says so.
pub const PEAK_LIMIT: f32 = 0.98;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Noise {
    /// DeepFilterNet asset noise_freesound_2530.wav (CC BY 3.0).
    Freesound2530,
    /// DeepFilterNet asset noise_freesound_573577.wav (CC0).
    Freesound573577,
    Fan,
    Keyboard,
    AcHiss,
    Babble,
    Street,
    Tv,
}

pub const NOISES: [Noise; 8] = [
    Noise::Freesound2530,
    Noise::Freesound573577,
    Noise::Fan,
    Noise::Keyboard,
    Noise::AcHiss,
    Noise::Babble,
    Noise::Street,
    Noise::Tv,
];

pub const SPLITS: [&str; 3] = ["dev", "gate", "holdout"];

/// How many times each split covers the grid (8 x 5 x 3 = 120 cells).
fn reps(split: &str) -> usize {
    if split == "gate" { 2 } else { 1 }
}

/// One speaker's utterances at 16 kHz (LibriSpeech's rate), with their source file names.
pub struct Speaker {
    pub id: String,
    pub utts: Vec<(String, Vec<f32>)>,
}

/// Everything the builder reads. Real runs load it with [`Sources::load`]; tests make tiny ones.
pub struct Sources {
    pub speakers: Vec<Speaker>,
    /// 48 kHz, in [`Noise::Freesound2530`], [`Noise::Freesound573577`] order.
    pub sample_noises: [Vec<f32>; 2],
    /// 48 kHz room impulse response, starting at its direct path.
    pub rir: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub speaker: String,
    /// LibriSpeech utterances used for the speech, in order.
    pub speech_files: Vec<String>,
    pub noise: Noise,
    pub snr_db: f64,
    pub level_dbfs: f64,
    pub reverb: bool,
    /// Paths relative to the manifest's folder.
    pub mix: String,
    pub clean: String,
    /// Measured on the written 16-bit files over the speech region.
    pub snr_db_actual: f64,
    /// Whole-mixture RMS of the written file. Below `level_dbfs` only when the peak limit applied.
    pub level_dbfs_actual: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub split: String,
    pub seed: u64,
    pub sample_rate: u32,
    pub lead_in_samples: usize,
    pub item_samples: usize,
    pub speakers: Vec<String>,
    pub items: Vec<Item>,
}

impl Manifest {
    pub fn load(path: &Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| {
            format!(
                "read {} (build it with: audiolab corpus build)",
                path.display()
            )
        })?;
        Ok(serde_json::from_str(&s)?)
    }
}

impl Sources {
    /// `corpus` is research/corpus as written by research/scripts/fetch-corpus.sh.
    pub fn load(corpus: &Path) -> Result<Self> {
        let libri = corpus.join("LibriSpeech/dev-clean-2");
        let mut speakers = Vec::new();
        let mut ids: Vec<_> = std::fs::read_dir(&libri)
            .with_context(|| {
                format!(
                    "read {} (run research/scripts/fetch-corpus.sh)",
                    libri.display()
                )
            })?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()?;
        ids.sort();
        for spk in ids.iter().filter(|p| p.is_dir()) {
            let mut files = Vec::new();
            for ch in std::fs::read_dir(spk)? {
                for f in std::fs::read_dir(ch?.path())? {
                    let f = f?.path();
                    if f.extension().is_some_and(|e| e == "flac") {
                        files.push(f);
                    }
                }
            }
            files.sort();
            let utts = files
                .par_iter()
                .map(|f| {
                    let (x, sr) = audio::read_flac(f)?;
                    if sr != 16_000 {
                        bail!("{}: expected 16 kHz", f.display());
                    }
                    Ok((f.file_name().unwrap().to_string_lossy().into_owned(), x))
                })
                .collect::<Result<_>>()?;
            let id = spk.file_name().unwrap().to_string_lossy().into_owned();
            speakers.push(Speaker { id, utts });
        }
        let wav48 = |name: &str| -> Result<Vec<f32>> {
            let (x, sr) = audio::read_wav(&corpus.join("assets").join(name))?;
            if sr != SR {
                bail!("{name}: expected 48 kHz");
            }
            Ok(x)
        };
        let mut rir = wav48("rir_sim_1001_w11.7_l2.6_h2.5_rt60_0.7919.wav")?;
        let direct = rir
            .iter()
            .enumerate()
            .fold(
                (0, 0.0f32),
                |m, (i, &v)| if v.abs() > m.1 { (i, v.abs()) } else { m },
            )
            .0;
        rir.drain(..direct);
        Ok(Self {
            speakers,
            sample_noises: [
                wav48("noise_freesound_2530.wav")?,
                wav48("noise_freesound_573577.wav")?,
            ],
            rir,
        })
    }
}

/// Shuffles speaker ids with the seed and cuts them into dev, gate, holdout (about 1:2:1).
pub fn split_speakers(ids: &[String], seed: u64) -> [Vec<String>; 3] {
    let mut ids = ids.to_vec();
    ids.sort();
    shuffle(&mut ids, &mut ChaCha8Rng::seed_from_u64(seed));
    let k = ids.len().div_ceil(4);
    let holdout = ids.split_off(ids.len() - k);
    let gate = ids.split_off(k);
    [ids, gate, holdout]
}

fn shuffle<T>(v: &mut [T], rng: &mut ChaCha8Rng) {
    for i in (1..v.len()).rev() {
        v.swap(i, rng.random_range(0..=i));
    }
}

/// What to build for one item, before any audio exists.
#[derive(Clone, Debug)]
pub struct Plan {
    pub id: String,
    pub speaker: String,
    pub noise: Noise,
    pub snr_db: f64,
    pub level_dbfs: f64,
    pub reverb: bool,
    pub seed: u64,
}

/// The split's items: the full grid `reps` times, in a seeded order. Speakers go round-robin over
/// that order and every fourth item is reverberant, so both spread evenly over the grid.
pub fn plan_split(split: &str, speakers: &[String], seed: u64) -> Vec<Plan> {
    let s = SPLITS
        .iter()
        .position(|&x| x == split)
        .expect("known split") as u64;
    let mut cells = Vec::new();
    for _ in 0..reps(split) {
        for &noise in &NOISES {
            for &snr_db in &SNRS_DB {
                for &level_dbfs in &LEVELS_DBFS {
                    cells.push((noise, snr_db, level_dbfs));
                }
            }
        }
    }
    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ (0x5eed_0000 + s));
    shuffle(&mut cells, &mut rng);
    cells
        .into_iter()
        .enumerate()
        .map(|(k, (noise, snr_db, level_dbfs))| Plan {
            id: format!("{split}-{k:04}"),
            speaker: speakers[k % speakers.len()].clone(),
            noise,
            snr_db,
            level_dbfs,
            reverb: k % 4 == 3,
            seed: seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (s << 32 | k as u64),
        })
        .collect()
}

/// Scales `noise` so the SNR over `speech` is `snr_db`, adds it, then scales the sum to
/// `level_dbfs` RMS over the whole item. Returns (mixture, clean) with the same gain applied.
pub fn mix(
    clean: &[f32],
    noise: &[f32],
    speech: std::ops::Range<usize>,
    snr_db: f64,
    level_dbfs: f64,
) -> (Vec<f32>, Vec<f32>) {
    let ps = rms(&clean[speech.clone()]);
    let pn = rms(&noise[speech]);
    let k = ps / pn / 10f64.powf(snr_db / 20.0);
    let m: Vec<f64> = clean
        .iter()
        .zip(noise)
        .map(|(&c, &n)| c as f64 + k * n as f64)
        .collect();
    let mr = (m.iter().map(|v| v * v).sum::<f64>() / m.len() as f64).sqrt();
    let g = 10f64.powf(level_dbfs / 20.0) / mr;
    (
        m.iter().map(|v| (v * g) as f32).collect(),
        clean.iter().map(|&c| (c as f64 * g) as f32).collect(),
    )
}

/// Builds one item's audio. Returns (mixture, clean, speech files used).
pub fn build_item(
    src: &Sources,
    split_speakers: &[String],
    p: &Plan,
) -> (Vec<f32>, Vec<f32>, Vec<String>) {
    let mut rng = ChaCha8Rng::seed_from_u64(p.seed);
    let spk = src
        .speakers
        .iter()
        .find(|s| s.id == p.speaker)
        .expect("planned speaker exists");
    let (speech16, files) = speech(spk, (ITEM_LEN - LEAD_IN) / 3, 0.25, &mut rng);
    let mut speech = audio::up3(&speech16);
    fade(&mut speech, SR as usize / 200);
    if p.reverb {
        speech = audio::convolve(&speech, &src.rir);
    }
    let mut clean = vec![0.0f32; ITEM_LEN];
    clean[LEAD_IN..].copy_from_slice(&speech);
    let others: Vec<&Speaker> = src
        .speakers
        .iter()
        .filter(|s| s.id != p.speaker && split_speakers.contains(&s.id))
        .collect();
    let noise = make_noise(p.noise, src, &others, &mut rng);
    let (mut m, mut c) = mix(&clean, &noise, LEAD_IN..ITEM_LEN, p.snr_db, p.level_dbfs);
    let peak = m.iter().fold(0.0f32, |a, &v| a.max(v.abs()));
    if peak > PEAK_LIMIT {
        let g = PEAK_LIMIT / peak;
        m.iter_mut().chain(c.iter_mut()).for_each(|v| *v *= g);
    }
    audio::quantize_i16(&mut m);
    audio::quantize_i16(&mut c);
    (m, c, files)
}

/// Joins a speaker's utterances (from a random start, `gap_s` of silence between) to `len` samples at 16 kHz.
fn speech(spk: &Speaker, len: usize, gap_s: f64, rng: &mut ChaCha8Rng) -> (Vec<f32>, Vec<String>) {
    let mut out = Vec::with_capacity(len);
    let mut files = Vec::new();
    let mut i = rng.random_range(0..spk.utts.len());
    while out.len() < len {
        if !out.is_empty() {
            out.extend(std::iter::repeat_n(0.0, (gap_s * 16_000.0) as usize));
        }
        let (name, x) = &spk.utts[i];
        out.extend_from_slice(x);
        files.push(name.clone());
        i = (i + 1) % spk.utts.len();
    }
    out.truncate(len);
    (out, files)
}

fn fade(x: &mut [f32], n: usize) {
    let len = x.len();
    for i in 0..n.min(len / 2) {
        let g = i as f32 / n as f32;
        x[i] *= g;
        x[len - 1 - i] *= g;
    }
}

fn gauss(rng: &mut ChaCha8Rng) -> f64 {
    let u: f64 = rng.random::<f64>().max(1e-300);
    let v: f64 = rng.random();
    (-2.0 * u.ln()).sqrt() * (2.0 * PI * v).cos()
}

fn white(n: usize, rng: &mut ChaCha8Rng) -> Vec<f64> {
    (0..n).map(|_| gauss(rng)).collect()
}

fn normalize(x: &mut [f64]) {
    let r = (x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64).sqrt();
    if r > 0.0 {
        x.iter_mut().for_each(|v| *v /= r);
    }
}

/// Leaky integrator of white noise (about 6 dB per octave down above 4 Hz), unit RMS.
fn brown(n: usize, rng: &mut ChaCha8Rng) -> Vec<f64> {
    let mut y = 0.0;
    let mut x: Vec<f64> = white(n, rng)
        .into_iter()
        .map(|w| {
            y = 0.9995 * y + w;
            y
        })
        .collect();
    normalize(&mut x);
    x
}

/// Pink noise (Paul Kellet's filter), unit RMS.
fn pink(n: usize, rng: &mut ChaCha8Rng) -> Vec<f64> {
    let mut b = [0.0f64; 7];
    let mut x: Vec<f64> = white(n, rng)
        .into_iter()
        .map(|w| {
            b[0] = 0.99886 * b[0] + w * 0.0555179;
            b[1] = 0.99332 * b[1] + w * 0.0750759;
            b[2] = 0.96900 * b[2] + w * 0.1538520;
            b[3] = 0.86650 * b[3] + w * 0.3104856;
            b[4] = 0.55000 * b[4] + w * 0.5329522;
            b[5] = -0.7616 * b[5] - w * 0.0168980;
            let y = b.iter().sum::<f64>() + w * 0.5362;
            b[6] = w * 0.115926;
            y
        })
        .collect();
    normalize(&mut x);
    x
}

fn one_pole_highpass(x: &mut [f64], fc: f64) {
    let a = (-2.0 * PI * fc / SR as f64).exp();
    let (mut px, mut py) = (0.0, 0.0);
    for v in x {
        let y = a * (py + *v - px);
        px = *v;
        py = y;
        *v = y;
    }
}

fn tone(t: f64, f: f64, phase: f64) -> f64 {
    (2.0 * PI * f * t + phase).sin()
}

/// Another speaker's running speech, 10 s at 48 kHz, unit RMS.
fn talker(spk: &Speaker, rng: &mut ChaCha8Rng) -> Vec<f64> {
    let (x, _) = speech(spk, ITEM_LEN / 3, 0.1, rng);
    let mut x: Vec<f64> = audio::up3(&x).into_iter().map(f64::from).collect();
    normalize(&mut x);
    x
}

/// A 10 s 48 kHz noise of the given type (any scale: [`mix`] sets the level).
fn make_noise(kind: Noise, src: &Sources, others: &[&Speaker], rng: &mut ChaCha8Rng) -> Vec<f32> {
    let n = ITEM_LEN;
    let t = |i: usize| i as f64 / SR as f64;
    let x: Vec<f64> = match kind {
        Noise::Freesound2530 | Noise::Freesound573577 => {
            let s = &src.sample_noises[(kind == Noise::Freesound573577) as usize];
            // Ping-pong loop: no jump at the seams.
            let period = 2 * s.len() - 2;
            let off = rng.random_range(0..period);
            (0..n)
                .map(|i| {
                    let j = (i + off) % period;
                    s[if j < s.len() { j } else { period - j }] as f64
                })
                .collect()
        }
        Noise::Fan => {
            let hum = rng.random_range(100.0..130.0);
            let blade = rng.random_range(180.0..420.0);
            let wobble = rng.random_range(0.5..2.0);
            let ph: Vec<f64> = (0..6).map(|_| rng.random_range(0.0..2.0 * PI)).collect();
            brown(n, rng)
                .into_iter()
                .enumerate()
                .map(|(i, b)| {
                    let t = t(i);
                    let h = 0.35 * tone(t, hum, ph[0])
                        + 0.18 * tone(t, 2.0 * hum, ph[1])
                        + 0.1 * tone(t, 3.0 * hum, ph[2]);
                    let am = 1.0 + 0.3 * tone(t, wobble, ph[3]);
                    let bp =
                        am * (0.25 * tone(t, blade, ph[4]) + 0.1 * tone(t, 2.0 * blade, ph[5]));
                    b + h + bp
                })
                .collect()
        }
        Noise::Keyboard => {
            let mut x: Vec<f64> = white(n, rng).into_iter().map(|w| 0.01 * w).collect();
            let mut pos = rng.random_range(0..SR as usize / 2);
            while pos < n {
                let amp = rng.random_range(0.4..1.0);
                for (start, a) in [
                    (pos, amp),
                    (pos + rng.random_range(2_900..4_800), 0.4 * amp),
                ] {
                    // 3 to 8 ms decays: spiky, but not so spiky that a -25 dBFS mix clips.
                    let tau = rng.random_range(150.0..400.0);
                    for k in 0..(6.0 * tau) as usize {
                        if start + k < n {
                            x[start + k] += a * gauss(rng) * (-(k as f64) / tau).exp();
                        }
                    }
                }
                // Typing bursts: mostly short gaps, sometimes a pause.
                let gap = if rng.random::<f64>() < 0.15 {
                    rng.random_range(0.4..1.2)
                } else {
                    rng.random_range(0.08..0.25)
                };
                pos += (gap * SR as f64) as usize;
            }
            one_pole_highpass(&mut x, 300.0);
            x
        }
        Noise::AcHiss => {
            let mut hiss = white(n, rng);
            one_pole_highpass(&mut hiss, 1_000.0);
            normalize(&mut hiss);
            let p = pink(n, rng);
            let b = brown(n, rng);
            (0..n)
                .map(|i| 0.7 * hiss[i] + 0.5 * p[i] + 0.25 * b[i])
                .collect()
        }
        Noise::Babble => {
            let mut x = vec![0.0; n];
            for spk in pick(others, 4, rng) {
                x.iter_mut()
                    .zip(talker(spk, rng))
                    .for_each(|(a, b)| *a += b);
            }
            x
        }
        Noise::Street => {
            let b = brown(n, rng);
            let p = pink(n, rng);
            // Passing cars: slow bumps of 1 to 3 s on top of a steady bed.
            let cars: Vec<(f64, f64, f64)> = (0..rng.random_range(2..6))
                .map(|_| {
                    (
                        rng.random_range(0.0..10.0),
                        rng.random_range(0.5..1.5),
                        rng.random_range(1.0..3.0),
                    )
                })
                .collect();
            (0..n)
                .map(|i| {
                    let t = t(i);
                    let env = 1.0
                        + cars
                            .iter()
                            .map(|&(c, w, a)| a * (-((t - c) / w).powi(2)).exp())
                            .sum::<f64>();
                    env * (0.7 * b[i] + 0.4 * p[i])
                })
                .collect()
        }
        Noise::Tv => {
            let voice = talker(pick(others, 1, rng)[0], rng);
            // Music-like: notes from an A minor pentatonic, three harmonics, decaying, in bursts.
            const SCALE: [f64; 10] = [
                220.0, 261.63, 293.66, 329.63, 392.0, 440.0, 523.25, 587.33, 659.25, 783.99,
            ];
            let mut music = vec![0.0; n];
            let mut pos = 0;
            while pos < n {
                let len = (rng.random_range(0.15..0.6) * SR as f64) as usize;
                if rng.random::<f64>() < 0.7 {
                    let f = SCALE[rng.random_range(0..SCALE.len())];
                    for k in 0..len.min(n - pos) {
                        let tk = t(k);
                        let env = (tk * 200.0).min(1.0) * (-tk * 4.0).exp();
                        music[pos + k] += env
                            * (tone(tk, f, 0.0)
                                + 0.5 * tone(tk, 2.0 * f, 0.0)
                                + 0.25 * tone(tk, 3.0 * f, 0.0));
                    }
                }
                pos += len;
            }
            normalize(&mut music);
            (0..n).map(|i| voice[i] + 0.5 * music[i]).collect()
        }
    };
    x.into_iter().map(|v| v as f32).collect()
}

fn pick<'a>(v: &[&'a Speaker], k: usize, rng: &mut ChaCha8Rng) -> Vec<&'a Speaker> {
    let mut v = v.to_vec();
    shuffle(&mut v, rng);
    v.truncate(k.min(v.len()));
    assert!(
        !v.is_empty(),
        "babble and tv need other speakers in the split"
    );
    v
}

/// SNR over the speech region and whole-item level of a written item, from its two signals.
pub fn measure(m: &[f32], c: &[f32]) -> (f64, f64) {
    let noise: Vec<f32> = m.iter().zip(c).map(|(a, b)| a - b).collect();
    let snr = db((rms(&c[LEAD_IN..]) / rms(&noise[LEAD_IN..])).powi(2));
    (snr, db(rms(m).powi(2)))
}

/// Builds every split under `out/<split>/` with a `manifest.json` each.
/// `limit` keeps only the first N items of each split (for quick tests).
pub fn build(src: &Sources, out: &Path, seed: u64, limit: Option<usize>) -> Result<()> {
    let ids: Vec<String> = src.speakers.iter().map(|s| s.id.clone()).collect();
    let groups = split_speakers(&ids, seed);
    for (split, spks) in SPLITS.iter().zip(&groups) {
        let dir = out.join(split);
        std::fs::create_dir_all(&dir)?;
        let mut plan = plan_split(split, spks, seed);
        plan.truncate(limit.unwrap_or(usize::MAX));
        let items = plan
            .par_iter()
            .map(|p| -> Result<Item> {
                let (m, c, files) = build_item(src, spks, p);
                let (mix, clean) = (format!("{}.mix.wav", p.id), format!("{}.clean.wav", p.id));
                audio::write_wav_i16(&dir.join(&mix), &m)?;
                audio::write_wav_i16(&dir.join(&clean), &c)?;
                let (snr_db_actual, level_dbfs_actual) = measure(&m, &c);
                Ok(Item {
                    id: p.id.clone(),
                    speaker: p.speaker.clone(),
                    speech_files: files,
                    noise: p.noise,
                    snr_db: p.snr_db,
                    level_dbfs: p.level_dbfs,
                    reverb: p.reverb,
                    mix,
                    clean,
                    snr_db_actual,
                    level_dbfs_actual,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let scaled = items
            .iter()
            .filter(|i| i.level_dbfs_actual < i.level_dbfs - 0.5)
            .count();
        let manifest = Manifest {
            split: split.to_string(),
            seed,
            sample_rate: SR,
            lead_in_samples: LEAD_IN,
            item_samples: ITEM_LEN,
            speakers: spks.clone(),
            items,
        };
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest)?,
        )?;
        eprintln!(
            "{split}: {} items, {} speakers, {scaled} scaled down for the peak limit",
            manifest.items.len(),
            spks.len()
        );
    }
    Ok(())
}

/// research/corpus/built/<split>/manifest.json and the folder its paths are relative to.
pub fn manifest_path(corpus: &Path, split: &str) -> (PathBuf, PathBuf) {
    let dir = corpus.join(split);
    (dir.join("manifest.json"), dir)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Tiny sources: 9 "speakers" of AM tones at 16 kHz, short noises, a 3-tap RIR.
    pub fn tiny_sources() -> Sources {
        let speakers = (0..9)
            .map(|s| Speaker {
                id: format!("{s}"),
                utts: (0..3)
                    .map(|u| {
                        let f = 120.0 + 20.0 * s as f64 + 5.0 * u as f64;
                        let x = (0..40_000)
                            .map(|i| {
                                let t = i as f64 / 16_000.0;
                                (0.3 * (1.0 + (2.0 * PI * 3.0 * t).sin()) * tone(t, f, 0.0)) as f32
                            })
                            .collect();
                        (format!("{s}-{u}.flac"), x)
                    })
                    .collect(),
            })
            .collect();
        let sn = |f: f64| {
            (0..30_000)
                .map(|i| tone(t48(i), f, 0.0) as f32 * 0.1)
                .collect()
        };
        Sources {
            speakers,
            sample_noises: [sn(1_000.0), sn(3_000.0)],
            rir: vec![1.0, 0.0, 0.3, 0.1],
        }
    }

    fn t48(i: usize) -> f64 {
        i as f64 / SR as f64
    }

    #[test]
    fn speakers_are_disjoint_and_cover_all() {
        let ids: Vec<String> = (0..27).map(|i| format!("{i}")).collect();
        let [d, g, h] = split_speakers(&ids, 1);
        assert_eq!((d.len(), g.len(), h.len()), (7, 13, 7));
        let mut all: Vec<_> = d.iter().chain(&g).chain(&h).cloned().collect();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 27);
    }

    #[test]
    fn plan_is_balanced_over_the_grid() {
        let spk: Vec<String> = (0..7).map(|i| format!("{i}")).collect();
        let p = plan_split("gate", &spk, 1);
        assert_eq!(p.len(), 240);
        for &n in &NOISES {
            for &s in &SNRS_DB {
                for &l in &LEVELS_DBFS {
                    let c = p
                        .iter()
                        .filter(|x| x.noise == n && x.snr_db == s && x.level_dbfs == l)
                        .count();
                    assert_eq!(c, 2);
                }
            }
        }
        assert_eq!(p.iter().filter(|x| x.reverb).count(), 60);
        assert_eq!(plan_split("dev", &spk, 1).len(), 120);
    }

    #[test]
    fn mix_hits_snr_and_level() {
        let src = tiny_sources();
        let spk: Vec<String> = (0..9).map(|i| format!("{i}")).collect();
        // Every noise type, every SNR, every level, through the real 16-bit output path.
        for (k, p) in plan_split("dev", &spk[..3], 7).iter().enumerate() {
            let (m, c, _) = build_item(&src, &spk[..3], p);
            let (snr, level) = measure(&m, &c);
            assert!((snr - p.snr_db).abs() < 0.1, "{k} {:?}: snr {snr}", p);
            let limited = m.iter().any(|v| v.abs() >= PEAK_LIMIT - 1e-4);
            if limited {
                assert!(level < p.level_dbfs, "{k}: {level}");
            } else {
                assert!(
                    (level - p.level_dbfs).abs() < 0.5,
                    "{k} {:?}: level {level}",
                    p
                );
            }
            assert!(c[..LEAD_IN].iter().all(|&v| v == 0.0));
        }
    }

    #[test]
    fn mix_without_peak_limit_is_exact() {
        let clean: Vec<f32> = (0..10_000)
            .map(|i| {
                if i < 2_000 {
                    0.0
                } else {
                    tone(t48(i), 300.0, 0.0) as f32
                }
            })
            .collect();
        let noise: Vec<f32> = (0..10_000)
            .map(|i| tone(t48(i), 2_100.0, 1.0) as f32)
            .collect();
        let (m, c) = mix(&clean, &noise, 2_000..10_000, 5.0, -25.0);
        let n: Vec<f32> = m.iter().zip(&c).map(|(a, b)| a - b).collect();
        let snr = db((rms(&c[2_000..]) / rms(&n[2_000..])).powi(2));
        assert!((snr - 5.0).abs() < 1e-3, "{snr}");
        assert!((db(rms(&m).powi(2)) + 25.0).abs() < 1e-3);
    }

    #[test]
    fn same_seed_gives_identical_bytes() {
        let src = tiny_sources();
        let dir = std::env::temp_dir().join(format!("audiolab-test-{}", std::process::id()));
        let (a, b) = (dir.join("a"), dir.join("b"));
        build(&src, &a, 3, Some(4)).unwrap();
        build(&src, &b, 3, Some(4)).unwrap();
        for split in SPLITS {
            let ma = std::fs::read(a.join(split).join("manifest.json")).unwrap();
            assert_eq!(
                ma,
                std::fs::read(b.join(split).join("manifest.json")).unwrap()
            );
            for f in std::fs::read_dir(a.join(split)).unwrap() {
                let name = f.unwrap().file_name();
                let x = std::fs::read(a.join(split).join(&name)).unwrap();
                assert_eq!(
                    x,
                    std::fs::read(b.join(split).join(&name)).unwrap(),
                    "{name:?}"
                );
            }
        }
        let other = dir.join("c");
        build(&src, &other, 4, Some(1)).unwrap();
        assert_ne!(
            std::fs::read(a.join("dev/dev-0000.mix.wav")).unwrap(),
            std::fs::read(other.join("dev/dev-0000.mix.wav")).unwrap()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
