#![forbid(unsafe_code)]
//! `audiolab`: build the corpus, evaluate a pipeline config, compare two runs, write listening pairs.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use vesper_audiolab::{corpus, eval};

#[derive(Parser)]
#[command(name = "audiolab", about = "Vesper audio lab")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Corpus commands.
    Corpus {
        #[command(subcommand)]
        cmd: CorpusCmd,
    },
    /// Run a pipeline config (or nothing) over a split and write per-item scores as JSON.
    Eval {
        /// A serialised `PipelineConfig` (TOML). An empty file is the shipped default.
        /// Repeat it to stack configs: they merge, and a key set to two values is an error.
        #[arg(
            long,
            required_unless_present = "unprocessed",
            conflicts_with = "unprocessed"
        )]
        config: Vec<PathBuf>,
        /// Score the mixtures as they are: the no-denoising control.
        #[arg(long)]
        unprocessed: bool,
        #[arg(long, value_parser = ["dev", "gate", "holdout"])]
        split: String,
        #[arg(long)]
        out: PathBuf,
        /// Only the first N items of the split.
        #[arg(long)]
        limit: Option<usize>,
        #[command(flatten)]
        paths: Paths,
        /// Worker threads (one pipeline and one DNSMOS model each).
        #[arg(long, default_value_t = 6)]
        jobs: usize,
        /// Encode the output with Opus, lose packets, decode, then score the decoded audio:
        /// bitrate=64000,complexity=10,fec=1,dtx=0,loss=0.0,seed=1[,burst=3] (missing keys: these defaults).
        #[arg(long)]
        opus: Option<vesper_audiolab::codec::OpusCfg>,
        /// UTMOS ONNX model: also score `second_mos` (research/scripts/fetch-utmos.sh).
        #[arg(long)]
        second_mos: Option<PathBuf>,
    },
    /// Paired deltas (b - a) per metric with bootstrap 95 percent intervals.
    Compare {
        #[arg(long)]
        a: PathBuf,
        #[arg(long)]
        b: PathBuf,
        /// Also print OVRL, lsd_wb and SI-SDRi per group.
        #[arg(long, value_parser = ["noise", "rir", "snr", "level"])]
        by: Option<String>,
    },
    /// Write input and processed WAV pairs for listening.
    Listen {
        #[arg(long)]
        config: PathBuf,
        /// Comma-separated item ids, for example holdout-0003,holdout-0017.
        #[arg(long, value_delimiter = ',')]
        items: Vec<String>,
        #[arg(long)]
        out: PathBuf,
        #[command(flatten)]
        paths: Paths,
    },
}

#[derive(Subcommand)]
enum CorpusCmd {
    /// Build the seeded mixtures. v1: from research/corpus (fetch-corpus.sh) into
    /// research/corpus/built. v2: from research/corpus48 (fetch-corpus48.sh) into research/corpus48/built.
    Build {
        #[arg(long, value_enum, default_value = "v1")]
        name: corpus::Version,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long)]
        sources: Option<PathBuf>,
        /// Only the first N items of each split.
        #[arg(long)]
        limit: Option<usize>,
    },
}

#[derive(clap::Args)]
struct Paths {
    /// DeepFilterNet model (tar.gz).
    #[arg(
        long,
        default_value = "crates/vesper-dsp/model/DeepFilterNet3_onnx.tar.gz"
    )]
    model: PathBuf,
    #[arg(long, default_value = "research/models/sig_bak_ovr.onnx")]
    dnsmos: PathBuf,
    #[arg(long, default_value = "research/corpus/built")]
    corpus: PathBuf,
}

fn pipeline(config: &[PathBuf], model: &std::path::Path) -> Result<eval::Proc> {
    let cfg = eval::load_configs(config)?;
    let bytes = std::fs::read(model)
        .with_context(|| format!("read {} (run scripts/fetch-model.sh)", model.display()))?;
    Ok(eval::Proc::Pipeline {
        cfg,
        model: Arc::new(bytes),
    })
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Corpus {
            cmd:
                CorpusCmd::Build {
                    name,
                    out,
                    seed,
                    sources,
                    limit,
                },
        } => {
            let (src, default_out) = match name {
                corpus::Version::V1 => (
                    corpus::Sources::load(&sources.unwrap_or("research/corpus".into()))?,
                    "research/corpus/built",
                ),
                corpus::Version::V2 => (
                    corpus::Sources::load_v2(&sources.unwrap_or("research/corpus48".into()))?,
                    "research/corpus48/built",
                ),
            };
            corpus::build(&src, name, &out.unwrap_or(default_out.into()), seed, limit)?;
        }
        Cmd::Eval {
            config,
            unprocessed,
            split,
            out,
            limit,
            paths,
            jobs,
            opus,
            second_mos,
        } => {
            let proc = if unprocessed {
                eval::Proc::Unprocessed
            } else {
                pipeline(&config, &paths.model)?
            };
            let r = eval::eval(eval::EvalArgs {
                corpus: &paths.corpus,
                split: &split,
                proc,
                model_path: Some(paths.model.clone()),
                dnsmos: &paths.dnsmos,
                limit,
                jobs,
                opus,
                second_mos: second_mos.as_deref(),
            })?;
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&out, serde_json::to_string_pretty(&r)?)?;
            let m = |k: &str| r.summary.get(k).map_or(f64::NAN, |s| s.mean);
            eprintln!(
                "{split}: {} items, OVRL {:.3} BAK {:.3} SIG {:.3} (SIG p10 {:.3}), UTMOS {:.3}, lsd_wb {:.2} dB, hb_err {:+.2} dB, MCD {:.2} dB, SI-SDRi {:.2} dB, lead-in {:.1} dB, rtf {}, wall {:.0} s, peak RSS {} MiB",
                r.n_items,
                m("ovrl"),
                m("bak"),
                m("sig"),
                r.summary.get("sig").map_or(f64::NAN, |s| s.p10),
                m("second_mos"),
                m("lsd_wb"),
                m("hb_err_db"),
                m("mcd"),
                m("si_sdr_i"),
                m("lead_in_reduction_db"),
                r.rtf.map_or("none".into(), |v| format!("{v:.4}")),
                r.wall_s,
                r.peak_rss_mib.map_or("?".into(), |v| format!("{v:.0}")),
            );
        }
        Cmd::Compare { a, b, by } => {
            let load = |p: &PathBuf| -> Result<eval::EvalResult> {
                let s =
                    std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
                Ok(serde_json::from_str(&s)?)
            };
            print!("{}", eval::compare(&load(&a)?, &load(&b)?, by.as_deref())?);
        }
        Cmd::Listen {
            config,
            items,
            out,
            paths,
        } => {
            eval::listen(
                &paths.corpus,
                &pipeline(std::slice::from_ref(&config), &paths.model)?,
                &items,
                &out,
            )?;
        }
    }
    Ok(())
}
