//! Runs a pipeline over a corpus split and scores every item; compares two runs item by item.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};
use vesper_dsp::{HOP, Pipeline, PipelineConfig};

use crate::audio::{self, SR};
use crate::codec::{self, OpusCfg};
use crate::corpus::{Item, Manifest, Rir, manifest_path};
use crate::dnsmos::Dnsmos;
use crate::metrics;
use crate::second_mos::Utmos;

/// What runs on the mixtures: the real pipeline, or nothing (the unprocessed control).
#[derive(Clone, Debug)]
pub enum Proc {
    Pipeline {
        cfg: PipelineConfig,
        model: std::sync::Arc<Vec<u8>>,
    },
    Unprocessed,
    /// Output made by another tool: `<dir>/<item id>.wav`, 48 kHz mono, already aligned with the
    /// mixture (no delay to drop) and at least as long. Used to score a reconstruction of another
    /// product's chain on the same items.
    External(PathBuf),
}

impl Proc {
    /// Processes `x` frame by frame like the browser worker (480 samples, from the first sample,
    /// zero-padded at the end), then drops the total delay so the result lines up with `x`.
    /// The result is `x.len() - delay` long. Returns it with the two delays (ours, the model's).
    pub fn run(&self, x: &[f32]) -> Result<(Vec<f32>, usize, usize)> {
        match self {
            Proc::Unprocessed => Ok((x.to_vec(), 0, 0)),
            Proc::External(_) => bail!("external output is read per item (Proc::run_item)"),
            Proc::Pipeline { cfg, model } => {
                let mut p = Pipeline::new(model, cfg)?;
                let mut out = Vec::with_capacity(x.len() + HOP);
                let mut frame = [0.0f32; HOP];
                for c in x.chunks(HOP) {
                    frame[..c.len()].copy_from_slice(c);
                    frame[c.len()..].fill(0.0);
                    out.extend(p.process(&frame)?);
                }
                let (ours, model) = (p.latency_samples(), p.model_latency_samples());
                let delay = ours + model;
                out.truncate(x.len());
                Ok((out.split_off(delay.min(out.len())), ours, model))
            }
        }
    }

    /// `run`, but an external output is read from its file for item `id`.
    pub fn run_item(&self, id: &str, x: &[f32]) -> Result<(Vec<f32>, usize, usize)> {
        let Proc::External(dir) = self else {
            return self.run(x);
        };
        let path = dir.join(format!("{id}.wav"));
        let (mut y, sr) = audio::read_wav(&path)?;
        if sr != SR || y.len() < x.len() {
            bail!(
                "{}: expected 48 kHz and at least {} samples",
                path.display(),
                x.len()
            );
        }
        y.truncate(x.len());
        Ok((y, 0, 0))
    }

    /// Real-time factor on `clip`: one thread, pipeline construction excluded, median of 3 runs.
    pub fn rtf(&self, clip: &[f32]) -> Result<Vec<f64>> {
        let Proc::Pipeline { cfg, model } = self else {
            return Ok(vec![]);
        };
        let secs = clip.len() as f64 / SR as f64;
        (0..3)
            .map(|_| {
                let mut p = Pipeline::new(model, cfg)?;
                let t = Instant::now();
                for c in clip.as_chunks::<HOP>().0 {
                    std::hint::black_box(p.process(c)?);
                }
                Ok(t.elapsed().as_secs_f64() / secs)
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemScores {
    pub id: String,
    pub noise: crate::corpus::Noise,
    pub snr_db: f64,
    pub level_dbfs: f64,
    pub reverb: bool,
    /// The room (v2 corpus). None: dry, or a v1 item.
    #[serde(default)]
    pub rir: Option<Rir>,
    pub sig: f64,
    pub bak: f64,
    pub ovrl: f64,
    /// SI-SDR of the output against the clean reference.
    pub si_sdr: f64,
    /// SI-SDR of the unprocessed mixture (same span).
    pub si_sdr_input: f64,
    pub si_sdr_i: f64,
    pub lsd: Option<f64>,
    /// Wideband judge ([`metrics::wideband`]). None for band-limited references (v1 corpus).
    #[serde(default)]
    pub lsd_wb: Option<f64>,
    #[serde(default)]
    pub hb_err_db: Option<f64>,
    #[serde(default)]
    pub mcd: Option<f64>,
    /// UTMOS ([`crate::second_mos`]), when the eval was given the model.
    #[serde(default)]
    pub second_mos: Option<f64>,
    pub lead_in_reduction_db: f64,
    pub peak_dbfs: f64,
    pub clipped: bool,
    /// Opus packets lost in this item (0 without the Opus stage).
    #[serde(default)]
    pub lost_packets: usize,
}

/// Metrics that `compare` and the summary cover, in print order.
pub const METRICS: [&str; 13] = [
    "ovrl",
    "bak",
    "sig",
    "second_mos",
    "lsd_wb",
    "hb_err_db",
    "mcd",
    "si_sdr",
    "si_sdr_i",
    "lsd",
    "lead_in_reduction_db",
    "peak_dbfs",
    "si_sdr_input",
];

impl ItemScores {
    pub fn get(&self, metric: &str) -> Option<f64> {
        Some(match metric {
            "ovrl" => self.ovrl,
            "bak" => self.bak,
            "sig" => self.sig,
            "si_sdr" => self.si_sdr,
            "si_sdr_i" => self.si_sdr_i,
            "lsd" => return self.lsd,
            "lsd_wb" => return self.lsd_wb,
            "hb_err_db" => return self.hb_err_db,
            "mcd" => return self.mcd,
            "second_mos" => return self.second_mos,
            "lead_in_reduction_db" => self.lead_in_reduction_db,
            "peak_dbfs" => self.peak_dbfs,
            "si_sdr_input" => self.si_sdr_input,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stat {
    pub mean: f64,
    pub median: f64,
    pub p10: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EvalResult {
    pub split: String,
    /// None for the unprocessed control.
    pub config: Option<PipelineConfig>,
    pub model: Option<String>,
    pub n_items: usize,
    /// Median of `rtf_runs`. None for the unprocessed control.
    pub rtf: Option<f64>,
    pub rtf_runs: Vec<f64>,
    pub rtf_clip: String,
    pub latency_samples: usize,
    pub model_latency_samples: usize,
    /// The Opus stage applied to the output before scoring, if any.
    #[serde(default)]
    pub opus: Option<OpusCfg>,
    #[serde(default)]
    pub second_mos_model: Option<String>,
    pub clipped_items: usize,
    pub threads: usize,
    pub wall_s: f64,
    /// VmHWM of the process at the end of the run, in MiB (Linux only).
    pub peak_rss_mib: Option<f64>,
    pub summary: BTreeMap<String, Stat>,
    pub items: Vec<ItemScores>,
}

pub struct EvalArgs<'a> {
    pub corpus: &'a Path,
    pub split: &'a str,
    pub proc: Proc,
    pub model_path: Option<PathBuf>,
    pub dnsmos: &'a Path,
    pub limit: Option<usize>,
    pub jobs: usize,
    pub opus: Option<OpusCfg>,
    /// UTMOS model path: score `second_mos` too (about 1 GB of RAM, shared by all threads).
    pub second_mos: Option<&'a Path>,
}

pub fn read_item(dir: &Path, it: &Item) -> Result<(Vec<f32>, Vec<f32>)> {
    let (m, sr) = audio::read_wav(&dir.join(&it.mix))?;
    let (c, sr2) = audio::read_wav(&dir.join(&it.clean))?;
    if sr != SR || sr2 != SR || m.len() != c.len() {
        bail!("{}: expected two 48 kHz files of equal length", it.id);
    }
    Ok((m, c))
}

/// Scores one item. `out` is the aligned output (`Proc::run`), at most `mix.len()` long.
pub fn score(
    dnsmos: &Dnsmos,
    utmos: Option<&Utmos>,
    it: &Item,
    lead_in: usize,
    mix: &[f32],
    clean: &[f32],
    out: &[f32],
) -> Result<ItemScores> {
    let n = out.len();
    let s = dnsmos.score_48k(out)?;
    let si_sdr = metrics::si_sdr(out, &clean[..n]);
    let si_sdr_input = metrics::si_sdr(&mix[..n], &clean[..n]);
    let peak_dbfs = metrics::peak_dbfs(out);
    let wb = metrics::wideband(out, &clean[..n]);
    Ok(ItemScores {
        id: it.id.clone(),
        noise: it.noise,
        snr_db: it.snr_db,
        level_dbfs: it.level_dbfs,
        reverb: it.reverb,
        rir: it.rir,
        sig: s.sig,
        bak: s.bak,
        ovrl: s.ovrl,
        si_sdr,
        si_sdr_input,
        si_sdr_i: si_sdr - si_sdr_input,
        lsd: metrics::lsd(out, &clean[..n]),
        lsd_wb: wb.map(|w| w.lsd_wb),
        hb_err_db: wb.map(|w| w.hb_err_db),
        mcd: wb.map(|w| w.mcd),
        second_mos: utmos.map(|u| u.score_48k(out)).transpose()?,
        lead_in_reduction_db: metrics::lead_in_reduction_db(mix, out, lead_in),
        peak_dbfs,
        clipped: out.iter().any(|v| v.abs() > 1.0),
        lost_packets: 0,
    })
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    // Linear interpolation, as numpy's default.
    let pos = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

pub fn summarize(items: &[ItemScores]) -> BTreeMap<String, Stat> {
    METRICS
        .iter()
        .filter_map(|&m| {
            let mut v: Vec<f64> = items.iter().filter_map(|i| i.get(m)).collect();
            if v.is_empty() {
                return None;
            }
            v.sort_by(f64::total_cmp);
            let stat = Stat {
                mean: v.iter().sum::<f64>() / v.len() as f64,
                median: percentile(&v, 0.5),
                p10: percentile(&v, 0.1),
            };
            Some((m.to_string(), stat))
        })
        .collect()
}

pub fn peak_rss_mib() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

pub fn eval(a: EvalArgs) -> Result<EvalResult> {
    let start = Instant::now();
    let (mpath, dir) = manifest_path(a.corpus, a.split);
    let manifest = Manifest::load(&mpath)?;
    let mut items = manifest.items.clone();
    items.truncate(a.limit.unwrap_or(usize::MAX));
    if items.is_empty() {
        bail!("no items in {}", mpath.display());
    }
    // RTF first, on an otherwise idle pool: the split's first three mixtures joined (30 s).
    let rtf_items: Vec<&Item> = manifest.items.iter().take(3).collect();
    let mut clip = Vec::new();
    for it in &rtf_items {
        clip.extend(read_item(&dir, it)?.0);
    }
    let rtf_runs = a.proc.rtf(&clip)?;
    let mut sorted = rtf_runs.clone();
    sorted.sort_by(f64::total_cmp);
    let rtf = sorted.get(sorted.len() / 2).copied();

    let utmos = a.second_mos.map(Utmos::load).transpose()?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(a.jobs)
        .build()?;
    let next = AtomicUsize::new(0);
    // Per item: the scores and the two delays (ours, the model's).
    type Scored = Result<(ItemScores, usize, usize)>;
    let results: Mutex<Vec<Option<Scored>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    // One DNSMOS and one pipeline at a time per thread keeps memory flat.
    pool.broadcast(|_| {
        let dnsmos = match Dnsmos::load(a.dnsmos) {
            Ok(d) => d,
            Err(e) => {
                let i = next.fetch_add(1, Ordering::SeqCst);
                if i < items.len() {
                    results.lock().unwrap()[i] = Some(Err(e));
                }
                return;
            }
        };
        loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            let Some(it) = items.get(i) else { break };
            let r = (|| -> Result<_> {
                let (m, c) = read_item(&dir, it)?;
                let (mut out, ours, model) = a.proc.run_item(&it.id, &m)?;
                let mut lost = 0;
                if let Some(o) = &a.opus {
                    // A different loss pattern per item, the same on every run.
                    let seed = o.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ i as u64;
                    (out, lost) = codec::roundtrip(&out, o, seed)?;
                }
                let mut s = score(
                    &dnsmos,
                    utmos.as_ref(),
                    it,
                    manifest.lead_in_samples,
                    &m,
                    &c,
                    &out,
                )?;
                s.lost_packets = lost;
                Ok((s, ours, model))
            })()
            .with_context(|| format!("item {}", it.id));
            results.lock().unwrap()[i] = Some(r);
        }
    });
    let mut scores = Vec::with_capacity(items.len());
    let mut lat = (0, 0);
    for r in results.into_inner().unwrap() {
        let Some(r) = r else {
            bail!("an item was not scored (DNSMOS model failed to load?)");
        };
        let (s, ours, model) = r?;
        lat = (ours, model);
        scores.push(s);
    }
    let (config, model) = match &a.proc {
        Proc::Pipeline { cfg, .. } => (
            Some(cfg.clone()),
            a.model_path.as_ref().map(|p| p.display().to_string()),
        ),
        Proc::Unprocessed | Proc::External(_) => (None, None),
    };
    Ok(EvalResult {
        split: a.split.to_string(),
        config,
        model,
        n_items: scores.len(),
        rtf,
        rtf_runs,
        rtf_clip: rtf_items
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>()
            .join("+"),
        latency_samples: lat.0,
        model_latency_samples: lat.1,
        opus: a.opus.clone(),
        second_mos_model: a.second_mos.map(|p| p.display().to_string()),
        clipped_items: scores.iter().filter(|s| s.clipped).count(),
        threads: a.jobs,
        wall_s: start.elapsed().as_secs_f64(),
        peak_rss_mib: peak_rss_mib(),
        summary: summarize(&scores),
        items: scores,
    })
}

/// Mean paired delta (b - a) with a 95 percent percentile-bootstrap interval (10000 resamples, seed 0).
pub fn paired_delta(d: &[f64]) -> (f64, f64, f64) {
    let n = d.len();
    let mean = d.iter().sum::<f64>() / n as f64;
    let mut rng = ChaCha8Rng::seed_from_u64(0);
    let mut means: Vec<f64> = (0..10_000)
        .map(|_| (0..n).map(|_| d[rng.random_range(0..n)]).sum::<f64>() / n as f64)
        .collect();
    means.sort_by(f64::total_cmp);
    (mean, percentile(&means, 0.025), percentile(&means, 0.975))
}

/// The group an item belongs to for `compare --by`.
pub fn group_key(it: &ItemScores, by: &str) -> Result<String> {
    let name = |v: serde_json::Value| v.as_str().map_or(v.to_string(), str::to_string);
    Ok(match by {
        "noise" => name(serde_json::to_value(it.noise)?),
        "rir" => it
            .rir
            .map_or(Ok("dry".to_string()), |r| serde_json::to_value(r).map(name))?,
        "snr" => format!("{:+}", it.snr_db),
        "level" => format!("{}", it.level_dbfs),
        _ => bail!("--by must be noise, rir, snr or level"),
    })
}

/// Metrics `compare --by` prints per group.
const GROUP_METRICS: [&str; 3] = ["ovrl", "lsd_wb", "si_sdr_i"];

pub fn compare(a: &EvalResult, b: &EvalResult, by: Option<&str>) -> Result<String> {
    use std::fmt::Write;
    if a.split != b.split {
        bail!("different splits: {} vs {}", a.split, b.split);
    }
    let bmap: BTreeMap<&str, &ItemScores> = b.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let pairs: Vec<(&ItemScores, &ItemScores)> = a
        .items
        .iter()
        .filter_map(|x| bmap.get(x.id.as_str()).map(|y| (x, *y)))
        .collect();
    if pairs.is_empty() {
        bail!("no items in common");
    }
    let mut s = String::new();
    writeln!(s, "split {}: {} paired items (b - a)", a.split, pairs.len())?;
    writeln!(
        s,
        "{:<22} {:>9} {:>9} {:>9}  {:>21}",
        "metric", "mean a", "mean b", "delta", "95% CI"
    )?;
    for m in METRICS {
        let (va, vb): (Vec<f64>, Vec<f64>) = pairs
            .iter()
            .filter_map(|(x, y)| Some((x.get(m)?, y.get(m)?)))
            .unzip();
        if va.is_empty() {
            continue;
        }
        let d: Vec<f64> = va.iter().zip(&vb).map(|(x, y)| y - x).collect();
        let (mean, lo, hi) = paired_delta(&d);
        let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
        writeln!(
            s,
            "{m:<22} {:>9.3} {:>9.3} {mean:>+9.3}  [{lo:>+9.3}, {hi:>+9.3}]",
            avg(&va),
            avg(&vb)
        )?;
    }
    let f = |r: Option<f64>| r.map_or("none".into(), |v| format!("{v:.4}"));
    writeln!(s, "rtf: a {} b {}", f(a.rtf), f(b.rtf))?;
    writeln!(
        s,
        "latency samples (ours + model): a {} + {} b {} + {}",
        a.latency_samples, a.model_latency_samples, b.latency_samples, b.model_latency_samples
    )?;
    if let Some(by) = by {
        let mut groups: BTreeMap<String, Vec<(&ItemScores, &ItemScores)>> = BTreeMap::new();
        for &(x, y) in &pairs {
            groups.entry(group_key(x, by)?).or_default().push((x, y));
        }
        writeln!(s, "\nby {by}: mean a, mean b, delta (b - a)")?;
        write!(s, "{:<18} {:>4}", by, "n")?;
        for m in GROUP_METRICS {
            write!(s, " {:>8} {:>8} {:>8}", format!("{m} a"), "b", "delta")?;
        }
        writeln!(s)?;
        for (k, g) in &groups {
            write!(s, "{k:<18} {:>4}", g.len())?;
            for m in GROUP_METRICS {
                let (va, vb): (Vec<f64>, Vec<f64>) = g
                    .iter()
                    .filter_map(|(x, y)| Some((x.get(m)?, y.get(m)?)))
                    .unzip();
                if va.is_empty() {
                    write!(s, " {:>8} {:>8} {:>8}", "-", "-", "-")?;
                    continue;
                }
                let avg = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
                let (ma, mb) = (avg(&va), avg(&vb));
                write!(s, " {ma:>8.3} {mb:>8.3} {:>+8.3}", mb - ma)?;
            }
            writeln!(s)?;
        }
    }
    Ok(s)
}

pub fn load_config(path: &Path) -> Result<PipelineConfig> {
    load_configs(&[path.to_path_buf()])
}

/// Several config files merged into one (a stack): tables merge key by key, and a key set to
/// two different values is an error, so stacked experiments cannot silently override each other.
pub fn load_configs(paths: &[PathBuf]) -> Result<PipelineConfig> {
    fn merge(into: &mut toml::Table, from: toml::Table, path: &str) -> Result<()> {
        for (k, v) in from {
            match (into.get_mut(&k), v) {
                (Some(toml::Value::Table(a)), toml::Value::Table(b)) => {
                    merge(a, b, &format!("{path}{k}."))?;
                }
                (Some(old), v) if *old != v => {
                    bail!("stacked configs disagree on {path}{k}: {old} vs {v}")
                }
                (Some(_), _) => {}
                (None, v) => {
                    into.insert(k, v);
                }
            }
        }
        Ok(())
    }
    let mut all = toml::Table::new();
    for p in paths {
        let s = std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
        let t: toml::Table =
            toml::from_str(&s).with_context(|| format!("parse {}", p.display()))?;
        merge(&mut all, t, "").with_context(|| format!("merge {}", p.display()))?;
    }
    all.try_into()
        .with_context(|| format!("parse the merged config of {paths:?}"))
}

/// Writes `<id>.input.wav` (the 16-bit mixture) and `<id>.processed.wav` (32-bit float, delay
/// removed and zero-padded back to the input length) for each item id.
pub fn listen(corpus: &Path, proc: &Proc, ids: &[String], out: &Path) -> Result<()> {
    std::fs::create_dir_all(out)?;
    for id in ids {
        let split = id.split('-').next().unwrap_or_default();
        let (mpath, dir) = manifest_path(corpus, split);
        let manifest = Manifest::load(&mpath)?;
        let it = manifest
            .items
            .iter()
            .find(|i| &i.id == id)
            .with_context(|| format!("no item {id} in {}", mpath.display()))?;
        let (m, _) = read_item(&dir, it)?;
        let (mut y, _, _) = proc.run(&m)?;
        y.resize(m.len(), 0.0);
        audio::write_wav_i16(&out.join(format!("{id}.input.wav")), &m)?;
        audio::write_wav_f32(&out.join(format!("{id}.processed.wav")), &y)?;
        eprintln!(
            "{id}: {:?} snr {} level {} reverb {}",
            it.noise, it.snr_db, it.level_dbfs, it.reverb
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> std::sync::Arc<Vec<u8>> {
        let p = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../vesper-dsp/model/DeepFilterNet3_onnx.tar.gz"
        );
        std::sync::Arc::new(
            std::fs::read(p).unwrap_or_else(|e| {
                panic!("model missing at {p} ({e}); run scripts/fetch-model.sh")
            }),
        )
    }

    #[test]
    fn pipeline_config_toml_round_trip() {
        let cfg = PipelineConfig {
            atten_db: 30.0,
            min_db_thresh: Some(-15.0),
            input_gain_db: 3.0,
            ..PipelineConfig::default()
        };
        let s = toml::to_string(&cfg).unwrap();
        let back: PipelineConfig = toml::from_str(&s).unwrap();
        assert_eq!(format!("{back:?}"), format!("{cfg:?}"));
        // An empty file is the shipped default; unknown keys are refused.
        let d: PipelineConfig = toml::from_str("").unwrap();
        assert_eq!(format!("{d:?}"), format!("{:?}", PipelineConfig::default()));
        assert!(toml::from_str::<PipelineConfig>("atten_dB = 3.0").is_err());
        assert!(toml::from_str::<PipelineConfig>("[pre]\nhighpass = 1").is_err());
    }

    #[test]
    fn run_aligns_the_output_with_the_input() {
        let proc = Proc::Pipeline {
            cfg: PipelineConfig::default(),
            model: model(),
        };
        // 1 s with a 0.2 s chirp (300 Hz to 3 kHz): one clear correlation peak.
        let x: Vec<f32> = (0..SR as usize)
            .map(|i| {
                let t = i as f64 / SR as f64 - 0.4;
                if (0.0..0.2).contains(&t) {
                    (0.3 * (2.0 * std::f64::consts::PI * (300.0 * t + 6_750.0 * t * t)).sin())
                        as f32
                } else {
                    0.0
                }
            })
            .collect();
        let (y, ours, model) = proc.run(&x).unwrap();
        assert_eq!((ours, model), (0, 1440));
        assert_eq!(y.len(), x.len() - 1440);
        // Search both ways: a positive lag means the output is still late, negative means early.
        let corr = |l: isize| -> f64 {
            (0..y.len())
                .filter_map(|i| x.get((i as isize - l) as usize).map(|&v| (v * y[i]) as f64))
                .sum()
        };
        let c: Vec<(isize, f64)> = (-2000..=2000).map(|l| (l, corr(l))).collect();
        let lag = c.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap().0;
        assert!(lag.abs() < 3, "residual lag {lag}");
        assert_eq!(Proc::Unprocessed.run(&x).unwrap().0, x);
    }

    #[test]
    fn external_output_is_read_per_item_and_trimmed() {
        let dir = std::env::temp_dir().join(format!("audiolab-ext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let y: Vec<f32> = (0..1200).map(|i| i as f32 / 4000.0).collect();
        audio::write_wav_f32(&dir.join("it-1.wav"), &y).unwrap();
        let p = Proc::External(dir.clone());
        let x = vec![0.0f32; 1000];
        assert_eq!(p.run_item("it-1", &x).unwrap().0, y[..1000]);
        assert!(p.run_item("it-1", &vec![0.0; 1300]).is_err(), "too short");
        assert!(p.run_item("missing", &x).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn stacked_configs_merge_and_refuse_conflicts() {
        let dir = std::env::temp_dir().join(format!("audiolab-stack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = |n: &str, s: &str| {
            let p = dir.join(n);
            std::fs::write(&p, s).unwrap();
            p
        };
        let a = w("a.toml", "atten_db = 24.0\n");
        let b = w("b.toml", "post_filter_beta = 0.05\natten_db = 24.0\n");
        let c = w("c.toml", "atten_db = 12.0\n");
        let m = load_configs(&[a.clone(), b]).unwrap();
        assert_eq!((m.atten_db, m.post_filter_beta), (24.0, 0.05));
        assert_eq!(
            m.max_db_df_thresh,
            PipelineConfig::default().max_db_df_thresh
        );
        let e = load_configs(&[a, c]).unwrap_err();
        assert!(format!("{e:#}").contains("disagree on atten_db"), "{e:#}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bootstrap_interval_covers_the_mean() {
        let d: Vec<f64> = (0..100).map(|i| (i % 7) as f64 * 0.01).collect();
        let (m, lo, hi) = paired_delta(&d);
        assert!(lo < m && m < hi && hi - lo < 0.02, "{lo} {m} {hi}");
        assert_eq!(paired_delta(&[0.5; 20]), (0.5, 0.5, 0.5));
    }

    #[test]
    fn summary_percentiles() {
        let v: Vec<f64> = (0..11).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.1), 1.0);
        assert_eq!(percentile(&v, 0.5), 5.0);
    }
}
