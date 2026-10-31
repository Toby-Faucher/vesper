//! Checks on the real corpus and models. Ignored by default: they need
//! research/scripts/fetch-corpus.sh, fetch-dnsmos.sh and `audiolab corpus build --seed 1` (v1),
//! and fetch-corpus48.sh, fetch-utmos.sh and `audiolab corpus build --name v2 --seed 1` (v2).
//! Run: cargo test -p vesper-audiolab --release -- --ignored --nocapture

use std::path::{Path, PathBuf};

use vesper_audiolab::corpus::{
    self, ITEM_LEN, LEAD_IN, LEVELS_DBFS, LEVELS_DBFS_V2, Manifest, NOISES, NOISES_V2, SNRS_DB,
};
use vesper_audiolab::dnsmos::{Dnsmos, Scores};
use vesper_audiolab::{audio, eval};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest(split: &str) -> (Manifest, PathBuf) {
    manifest_in("research/corpus/built", split)
}

fn manifest_in(corpus: &str, split: &str) -> (Manifest, PathBuf) {
    let (m, dir) = corpus::manifest_path(&root().join(corpus), split);
    (Manifest::load(&m).unwrap(), dir)
}

#[test]
#[ignore]
fn built_v2_corpus_is_balanced_wideband_and_disjoint() {
    let mut seen = Vec::new();
    for (split, n) in [("dev", 160), ("gate", 400), ("holdout", 160)] {
        let (m, dir) = manifest_in("research/corpus48/built", split);
        assert_eq!((m.corpus.as_str(), m.items.len()), ("v2", n), "{split}");
        assert_eq!((m.sample_rate, m.speech_sample_rate), (48_000, 48_000));
        for s in &m.speakers {
            assert!(!seen.contains(s), "speaker {s} in two splits");
        }
        seen.extend(m.speakers.clone());
        for &noise in &NOISES_V2 {
            let c = m.items.iter().filter(|i| i.noise == noise).count();
            assert!(c == n / 14 || c == n / 14 + 1, "{split} {noise:?} {c}");
        }
        for &l in &LEVELS_DBFS_V2 {
            let c = m.items.iter().filter(|i| i.level_dbfs == l).count();
            assert!(c.abs_diff(n / 3) <= 1, "{split} {l} {c}");
        }
        let mut limited = 0;
        for it in &m.items {
            assert!((it.snr_db_actual - it.snr_db).abs() < 0.1, "{}", it.id);
            assert_eq!(it.speech_bandwidth_hz.len(), it.speech_files.len());
            if (it.level_dbfs_actual - it.level_dbfs).abs() >= 0.5 {
                assert!(it.level_dbfs_actual < it.level_dbfs, "{}", it.id);
                limited += 1;
            }
        }
        // Headroom: v1 scaled down about a third of its -15 dBFS items; v2 must stay under 8 percent.
        assert!(limited * 100 < 8 * n, "{split}: {limited} limited");
        let [lo, med, hi] = m.speech_bandwidth_hz.unwrap();
        println!(
            "{split}: {n} items, {} speakers, {limited} peak-limited, bandwidth min {lo} median {med} max {hi} Hz",
            m.speakers.len()
        );
        // Real wideband speech: the median file reaches well past LibriSpeech's 8 kHz.
        assert!(med >= 12_000.0, "{split}: median bandwidth {med}");
        // The clean reference really has high-band energy (not upsampled 16 kHz audio).
        let it = m.items.iter().find(|i| i.rir.is_none()).unwrap();
        let (_, c) = eval::read_item(&dir, it).unwrap();
        assert!(
            audio::bandwidth_hz(&c[LEAD_IN..], 48_000) > 10_000.0,
            "{}",
            it.id
        );
    }
}

/// UTMOS (second_mos) on v2 dev: clean above 0 dB SNR, and rising with SNR on average.
#[test]
#[ignore]
fn utmos_ranks_clean_above_noisy() {
    let u = vesper_audiolab::second_mos::Utmos::load(
        &root().join("research/models/utmos22_strong.onnx"),
    )
    .unwrap();
    let (m, dir) = manifest_in("research/corpus48/built", "dev");
    let items: Vec<_> = NOISES_V2
        .iter()
        .filter_map(|&n| m.items.iter().find(|i| i.noise == n && i.rir.is_none()))
        .take(6)
        .collect();
    let (mut clean, mut by_snr) = (0.0, [0.0f64; 3]);
    for it in &items {
        let (mx, c) = eval::read_item(&dir, it).unwrap();
        let noise: Vec<f32> = mx.iter().zip(&c).map(|(a, b)| a - b).collect();
        for (k, snr) in [0.0, 10.0, 20.0].into_iter().enumerate() {
            let (x, _) = corpus::mix(&c, &noise, LEAD_IN..ITEM_LEN, snr, -28.0);
            by_snr[k] += u.score_48k(&x).unwrap() / items.len() as f64;
        }
        let s = u.score_48k(&c).unwrap();
        assert_eq!(s, u.score_48k(&c).unwrap(), "not deterministic");
        clean += s / items.len() as f64;
    }
    println!("UTMOS 0/10/20 dB {by_snr:?}, clean {clean}");
    assert!(by_snr[0] < by_snr[1] && by_snr[1] < by_snr[2], "{by_snr:?}");
    assert!(clean > by_snr[0] + 0.5, "{clean} vs {by_snr:?}");
}

#[test]
#[ignore]
fn built_corpus_matches_its_grid() {
    let mut seen = Vec::new();
    for (split, n) in [("dev", 120), ("gate", 240), ("holdout", 120)] {
        let (m, _) = manifest(split);
        assert_eq!(m.items.len(), n, "{split}");
        for s in &m.speakers {
            assert!(!seen.contains(s), "speaker {s} in two splits");
        }
        seen.extend(m.speakers.clone());
        let per_cell = n / (NOISES.len() * SNRS_DB.len() * LEVELS_DBFS.len());
        for &noise in &NOISES {
            for &snr in &SNRS_DB {
                for &lvl in &LEVELS_DBFS {
                    let c = m
                        .items
                        .iter()
                        .filter(|i| i.noise == noise && i.snr_db == snr && i.level_dbfs == lvl)
                        .count();
                    assert_eq!(c, per_cell, "{split} {noise:?} {snr} {lvl}");
                }
            }
        }
        let mut limited = 0;
        for it in &m.items {
            assert!(
                (it.snr_db_actual - it.snr_db).abs() < 0.1,
                "{}: snr {}",
                it.id,
                it.snr_db_actual
            );
            if (it.level_dbfs_actual - it.level_dbfs).abs() >= 0.5 {
                assert!(it.level_dbfs_actual < it.level_dbfs, "{}", it.id);
                limited += 1;
            }
        }
        println!(
            "{split}: {n} items, {} speakers, {limited} peak-limited",
            m.speakers.len()
        );
    }
}

fn mean(v: &[Scores]) -> Scores {
    let n = v.len() as f64;
    Scores {
        sig: v.iter().map(|s| s.sig).sum::<f64>() / n,
        bak: v.iter().map(|s| s.bak).sum::<f64>() / n,
        ovrl: v.iter().map(|s| s.ovrl).sum::<f64>() / n,
    }
}

/// Clean speech must score clearly above the same speech at 0 dB SNR, and scores must rise with SNR.
#[test]
#[ignore]
fn dnsmos_ranks_clean_above_noisy_and_rises_with_snr() {
    let d = Dnsmos::load(&root().join("research/models/sig_bak_ovr.onnx")).unwrap();
    let (m, dir) = manifest("dev");
    // One dry item per noise type: the noise is the mixture minus the clean reference.
    let items: Vec<_> = NOISES
        .iter()
        .filter_map(|&n| m.items.iter().find(|i| i.noise == n && !i.reverb))
        .collect();
    let mut by_snr: Vec<Vec<Scores>> = vec![Vec::new(); SNRS_DB.len()];
    let mut clean = Vec::new();
    for it in &items {
        let (mx, c) = eval::read_item(&dir, it).unwrap();
        let noise: Vec<f32> = mx.iter().zip(&c).map(|(a, b)| a - b).collect();
        for (k, &snr) in SNRS_DB.iter().enumerate() {
            let (x, _) = corpus::mix(&c, &noise, LEAD_IN..ITEM_LEN, snr, -25.0);
            by_snr[k].push(d.score_48k(&x).unwrap());
        }
        let g = 10f64.powf(-25.0 / 20.0) / audio::rms(&c[LEAD_IN..]);
        let c: Vec<f32> = c.iter().map(|&v| (v as f64 * g) as f32).collect();
        let s = d.score_48k(&c).unwrap();
        assert_eq!(s, d.score_48k(&c).unwrap(), "not deterministic");
        println!(
            "{} {:?}: clean {s:?} 0 dB {:?}",
            it.id,
            it.noise,
            by_snr[1].last().unwrap()
        );
        clean.push(s);
    }
    let means: Vec<Scores> = by_snr.iter().map(|v| mean(v)).collect();
    let clean = mean(&clean);
    for (snr, s) in SNRS_DB.iter().zip(&means) {
        println!("SNR {snr:>3} dB: {s:?}");
    }
    println!("clean     : {clean:?}");
    for w in means.windows(2) {
        assert!(w[1].ovrl > w[0].ovrl && w[1].bak > w[0].bak, "{means:?}");
    }
    let zero = means[1];
    assert!(clean.ovrl > zero.ovrl + 0.5, "{clean:?} vs {zero:?}");
    assert!(clean.bak > zero.bak + 0.8, "{clean:?} vs {zero:?}");
    let (fs, _) =
        audio::read_wav(&root().join("research/corpus/assets/clean_freesound_33711.wav")).unwrap();
    let fs = d.score_48k(&fs).unwrap();
    println!("clean_freesound_33711: {fs:?}");
    assert!(fs.ovrl > zero.ovrl + 0.5);
}
