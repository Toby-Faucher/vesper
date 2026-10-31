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
        #[arg(
            long,
            required_unless_present = "unprocessed",
            conflicts_with = "unprocessed"
        )]
        config: Option<PathBuf>,
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
    },
    /// Paired deltas (b - a) per metric with bootstrap 95 percent intervals.
    Compare {
        #[arg(long)]
        a: PathBuf,
        #[arg(long)]
        b: PathBuf,
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
    /// Build the seeded mixtures from research/corpus (see research/scripts/fetch-corpus.sh).
    Build {
        #[arg(long, default_value = "research/corpus/built")]
        out: PathBuf,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        #[arg(long, default_value = "research/corpus")]
        sources: PathBuf,
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

fn pipeline(config: &std::path::Path, model: &std::path::Path) -> Result<eval::Proc> {
    let cfg = eval::load_config(config)?;
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
                    out,
                    seed,
                    sources,
                    limit,
                },
        } => {
            let src = corpus::Sources::load(&sources)?;
            corpus::build(&src, &out, seed, limit)?;
        }
        Cmd::Eval {
            config,
            unprocessed,
            split,
            out,
            limit,
            paths,
            jobs,
        } => {
            let proc = match &config {
                Some(c) if !unprocessed => pipeline(c, &paths.model)?,
                _ => eval::Proc::Unprocessed,
            };
            let r = eval::eval(eval::EvalArgs {
                corpus: &paths.corpus,
                split: &split,
                proc,
                model_path: Some(paths.model.clone()),
                dnsmos: &paths.dnsmos,
                limit,
                jobs,
            })?;
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&out, serde_json::to_string_pretty(&r)?)?;
            let m = |k: &str| r.summary.get(k).map_or(f64::NAN, |s| s.mean);
            eprintln!(
                "{split}: {} items, OVRL {:.3} BAK {:.3} SIG {:.3} (SIG p10 {:.3}), SI-SDRi {:.2} dB, lead-in {:.1} dB, rtf {}, wall {:.0} s, peak RSS {} MiB",
                r.n_items,
                m("ovrl"),
                m("bak"),
                m("sig"),
                r.summary.get("sig").map_or(f64::NAN, |s| s.p10),
                m("si_sdr_i"),
                m("lead_in_reduction_db"),
                r.rtf.map_or("none".into(), |v| format!("{v:.4}")),
                r.wall_s,
                r.peak_rss_mib.map_or("?".into(), |v| format!("{v:.0}")),
            );
        }
        Cmd::Compare { a, b } => {
            let load = |p: &PathBuf| -> Result<eval::EvalResult> {
                let s =
                    std::fs::read_to_string(p).with_context(|| format!("read {}", p.display()))?;
                Ok(serde_json::from_str(&s)?)
            };
            print!("{}", eval::compare(&load(&a)?, &load(&b)?)?);
        }
        Cmd::Listen {
            config,
            items,
            out,
            paths,
        } => {
            eval::listen(
                &paths.corpus,
                &pipeline(&config, &paths.model)?,
                &items,
                &out,
            )?;
        }
    }
    Ok(())
}
