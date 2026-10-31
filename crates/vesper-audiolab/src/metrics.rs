//! Intrusive metrics against the clean reference, plus level checks. All inputs are 48 kHz and
//! already time-aligned (the caller removes the pipeline delay).

use realfft::RealFftPlanner;

use crate::audio::db;

/// Scale-invariant SDR in dB (Le Roux et al. 2019), both signals made zero-mean first.
pub fn si_sdr(est: &[f32], reference: &[f32]) -> f64 {
    let n = est.len().min(reference.len());
    let mean = |x: &[f32]| x[..n].iter().map(|&v| v as f64).sum::<f64>() / n as f64;
    let (me, mr) = (mean(est), mean(reference));
    let (mut er, mut rr) = (0.0, 0.0);
    for i in 0..n {
        let (e, r) = (est[i] as f64 - me, reference[i] as f64 - mr);
        er += e * r;
        rr += r * r;
    }
    let alpha = er / rr;
    let (mut t, mut d) = (0.0, 0.0);
    for i in 0..n {
        let (e, r) = (est[i] as f64 - me, reference[i] as f64 - mr);
        t += (alpha * r).powi(2);
        d += (e - alpha * r).powi(2);
    }
    db(t / d.max(1e-20))
}

/// LSD framing: 1024-sample Hann frames (21.3 ms) with a 512 hop at 48 kHz.
pub const LSD_FRAME: usize = 1024;
pub const LSD_HOP: usize = 512;
/// Highest bin used: 8 kHz. LibriSpeech is 16 kHz audio, so the clean reference has nothing above it.
pub const LSD_MAX_HZ: f64 = 8_000.0;
/// A frame is speech-active when its clean energy (bins up to 8 kHz) is within 40 dB of the loudest frame.
pub const LSD_ACTIVE_DB: f64 = 40.0;
/// Power floor added to both spectra: 80 dB below the clean reference's largest bin power.
/// (`lsd`; `lsd_wb` uses the same depth but below each signal's own strongest band.)
pub const LSD_FLOOR_DB: f64 = 80.0;

/// Log-spectral distance in dB on speech-active frames:
/// mean over active frames of sqrt(mean over bins 1..=8 kHz of (10 log10 (P_ref + e) / (P_est + e))^2).
/// None when no frame is active.
pub fn lsd(est: &[f32], reference: &[f32]) -> Option<f64> {
    let n = est.len().min(reference.len());
    if n < LSD_FRAME {
        return None;
    }
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(LSD_FRAME);
    let win: Vec<f64> = (0..LSD_FRAME)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / LSD_FRAME as f64).cos())
        .collect();
    let top = (LSD_MAX_HZ * LSD_FRAME as f64 / crate::audio::SR as f64) as usize;
    let mut buf = vec![0.0; LSD_FRAME];
    let mut out = fft.make_output_vec();
    let mut power = |x: &[f32]| -> Vec<f64> {
        buf.iter_mut()
            .zip(x.iter().zip(&win))
            .for_each(|(b, (&s, &w))| *b = s as f64 * w);
        fft.process(&mut buf, &mut out).expect("fft length");
        out[1..=top].iter().map(|c| c.norm_sqr()).collect()
    };
    let frames: Vec<(Vec<f64>, Vec<f64>)> = (0..=(n - LSD_FRAME) / LSD_HOP)
        .map(|f| {
            let r = f * LSD_HOP..f * LSD_HOP + LSD_FRAME;
            (power(&reference[r.clone()]), power(&est[r]))
        })
        .collect();
    let energy = |p: &[f64]| p.iter().sum::<f64>();
    let max_e = frames.iter().map(|(r, _)| energy(r)).fold(0.0, f64::max);
    let max_bin = frames
        .iter()
        .flat_map(|(r, _)| r.iter().copied())
        .fold(0.0, f64::max);
    if max_e <= 0.0 {
        return None;
    }
    let floor = max_bin * 10f64.powf(-LSD_FLOOR_DB / 10.0);
    let gate = max_e * 10f64.powf(-LSD_ACTIVE_DB / 10.0);
    let per_frame: Vec<f64> = frames
        .iter()
        .filter(|(r, _)| energy(r) >= gate)
        .map(|(r, e)| {
            let ms = r
                .iter()
                .zip(e)
                .map(|(&a, &b)| db((a + floor) / (b + floor)).powi(2))
                .sum::<f64>()
                / r.len() as f64;
            ms.sqrt()
        })
        .collect();
    Some(per_frame.iter().sum::<f64>() / per_frame.len() as f64)
}

/// Wideband scores against the clean reference (the second judge, independent of DNSMOS).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wideband {
    /// Multi-resolution log-mel distance in dB over 0 to 24 kHz, gain offset removed.
    pub lsd_wb: f64,
    /// High-band (8 to 20 kHz) share of the energy, output minus reference, in dB.
    /// Negative: high band lost. Positive: extra high-band energy (residual noise, artefacts).
    pub hb_err_db: f64,
    /// Mel-cepstral distortion in dB (coefficients 1 to 24, so level-independent).
    pub mcd: f64,
}

/// FFT sizes of the multi-resolution log-mel distance (10.7, 21.3 and 42.7 ms at 48 kHz), hop a quarter.
pub const WB_FFTS: [usize; 3] = [512, 1024, 2048];
pub const WB_MELS: usize = 64;
pub const HB_LO_HZ: f64 = 8_000.0;
pub const HB_HI_HZ: f64 = 20_000.0;
const MCD_COEFFS: usize = 24;
/// MCD floor: 50 dB below each signal's strongest mel band, so near-empty bands (where any
/// residual noise is a huge log ratio) do not swamp the speech envelope.
const MCD_FLOOR_DB: f64 = 50.0;

fn hann(n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / n as f64).cos())
        .collect()
}

/// Power spectra (bins 0..=n/2) of Hann frames of `n` samples every `hop`.
fn stft_power(x: &[f32], n: usize, hop: usize) -> Vec<Vec<f64>> {
    if x.len() < n {
        return vec![];
    }
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(n);
    let win = hann(n);
    let mut buf = vec![0.0; n];
    let mut out = fft.make_output_vec();
    (0..=(x.len() - n) / hop)
        .map(|f| {
            let s = &x[f * hop..f * hop + n];
            buf.iter_mut()
                .zip(s.iter().zip(&win))
                .for_each(|(b, (&v, &w))| *b = v as f64 * w);
            fft.process(&mut buf, &mut out).expect("fft length");
            out.iter().map(|c| c.norm_sqr()).collect()
        })
        .collect()
}

/// HTK mel triangles over 0 to 24 kHz as sparse (bin, weight) lists. Triangles too narrow to
/// hold any bin at this FFT size are dropped, so low resolutions have fewer bands.
fn mel_bank(n_fft: usize, n_mels: usize) -> Vec<Vec<(usize, f64)>> {
    let mel = |f: f64| 2595.0 * (1.0 + f / 700.0).log10();
    let hz = |m: f64| 700.0 * (10f64.powf(m / 2595.0) - 1.0);
    let top = mel(crate::audio::SR as f64 / 2.0);
    let edges: Vec<f64> = (0..n_mels + 2)
        .map(|i| hz(top * i as f64 / (n_mels + 1) as f64))
        .collect();
    let bin_hz = crate::audio::SR as f64 / n_fft as f64;
    (0..n_mels)
        .map(|m| {
            let (lo, c, hi) = (edges[m], edges[m + 1], edges[m + 2]);
            (0..=n_fft / 2)
                .filter_map(|k| {
                    let f = k as f64 * bin_hz;
                    let w = if f > lo && f <= c {
                        (f - lo) / (c - lo)
                    } else if f > c && f < hi {
                        (hi - f) / (hi - c)
                    } else {
                        0.0
                    };
                    (w > 0.0).then_some((k, w))
                })
                .collect::<Vec<_>>()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

fn apply_bank(bank: &[Vec<(usize, f64)>], p: &[f64]) -> Vec<f64> {
    bank.iter()
        .map(|t| t.iter().map(|&(k, w)| w * p[k]).sum())
        .collect()
}

/// Indices of speech-active frames: total reference power within [`LSD_ACTIVE_DB`] of the loudest frame.
fn active(frames: &[Vec<f64>]) -> Vec<usize> {
    let e: Vec<f64> = frames.iter().map(|p| p.iter().sum()).collect();
    let max_e = e.iter().copied().fold(0.0, f64::max);
    if max_e <= 0.0 {
        return vec![];
    }
    let gate = max_e * 10f64.powf(-LSD_ACTIVE_DB / 10.0);
    (0..e.len()).filter(|&i| e[i] >= gate).collect()
}

/// [`Wideband`] scores on speech-active frames. None when the reference has no active frame
/// or no energy in the high band (band-limited speech: the score would mean nothing).
pub fn wideband(est: &[f32], reference: &[f32]) -> Option<Wideband> {
    let n = est.len().min(reference.len());
    let (est, reference) = (&est[..n], &reference[..n]);
    let mut lsd_sum = 0.0;
    let mut mcd = None;
    for &nfft in &WB_FFTS {
        let (pr, pe) = (
            stft_power(reference, nfft, nfft / 4),
            stft_power(est, nfft, nfft / 4),
        );
        let act = active(&pr);
        if act.is_empty() {
            return None;
        }
        let bank = mel_bank(nfft, WB_MELS);
        let mr: Vec<Vec<f64>> = act.iter().map(|&i| apply_bank(&bank, &pr[i])).collect();
        let me: Vec<Vec<f64>> = act.iter().map(|&i| apply_bank(&bank, &pe[i])).collect();
        // Each signal gets a floor below its own strongest band, so a gain change moves both.
        let top = |m: &[Vec<f64>]| m.iter().flatten().copied().fold(0.0, f64::max);
        let (tr, te) = (top(&mr), top(&me).max(1e-30));
        let (fr, fe) = (
            tr * 10f64.powf(-LSD_FLOOR_DB / 10.0),
            te * 10f64.powf(-LSD_FLOOR_DB / 10.0),
        );
        let d: Vec<Vec<f64>> = mr
            .iter()
            .zip(&me)
            .map(|(r, e)| {
                r.iter()
                    .zip(e)
                    .map(|(&a, &b)| db((a + fr) / (b + fe)))
                    .collect()
            })
            .collect();
        // Gain-invariant: a level change of the whole output is not distortion.
        let offset = d.iter().flatten().sum::<f64>() / d.iter().map(Vec::len).sum::<usize>() as f64;
        lsd_sum += d
            .iter()
            .map(|f| (f.iter().map(|v| (v - offset).powi(2)).sum::<f64>() / f.len() as f64).sqrt())
            .sum::<f64>()
            / d.len() as f64;
        if nfft == 1024 {
            let g = 10f64.powf(-MCD_FLOOR_DB / 10.0);
            mcd = Some(mel_cepstral_distortion(&mr, &me, tr * g, te * g));
        }
    }
    // High band share of the energy over the active frames (2048-point frames).
    let (pr, pe) = (stft_power(reference, 2048, 512), stft_power(est, 2048, 512));
    let bin_hz = crate::audio::SR as f64 / 2048.0;
    let band = |p: &Vec<f64>| -> (f64, f64) {
        let hb = p
            .iter()
            .enumerate()
            .filter(|(k, _)| (HB_LO_HZ..HB_HI_HZ).contains(&(*k as f64 * bin_hz)))
            .map(|(_, v)| v)
            .sum::<f64>();
        (hb, p[1..].iter().sum::<f64>())
    };
    let (mut r, mut e) = ((0.0, 0.0), (0.0, 0.0));
    for i in active(&pr) {
        let (a, b) = (band(&pr[i]), band(&pe[i]));
        r = (r.0 + a.0, r.1 + a.1);
        e = (e.0 + b.0, e.1 + b.1);
    }
    if r.0 <= 0.0 || e.1 <= 0.0 {
        return None;
    }
    Some(Wideband {
        lsd_wb: lsd_sum / WB_FFTS.len() as f64,
        hb_err_db: db((e.0 + 1e-30) / e.1) - db(r.0 / r.1),
        mcd: mcd.expect("1024 is in WB_FFTS"),
    })
}

/// Mean over frames of (10 / ln 10) sqrt(2 sum_k (c_ref,k - c_est,k)^2), k = 1..=24, where c is the
/// orthonormal DCT-II of the natural-log mel amplitudes (half the log energy), as the usual MCD.
fn mel_cepstral_distortion(mr: &[Vec<f64>], me: &[Vec<f64>], fr: f64, fe: f64) -> f64 {
    let m = mr[0].len();
    let ceps = |x: &[f64], floor: f64| -> Vec<f64> {
        let l: Vec<f64> = x.iter().map(|&v| 0.5 * (v + floor).ln()).collect();
        (1..=MCD_COEFFS.min(m - 1))
            .map(|k| {
                (2.0 / m as f64).sqrt()
                    * l.iter()
                        .enumerate()
                        .map(|(i, &v)| {
                            v * (std::f64::consts::PI * k as f64 * (i as f64 + 0.5) / m as f64)
                                .cos()
                        })
                        .sum::<f64>()
            })
            .collect()
    };
    let k = 10.0 / std::f64::consts::LN_10;
    mr.iter()
        .zip(me)
        .map(|(r, e)| {
            let s: f64 = ceps(r, fr)
                .iter()
                .zip(ceps(e, fe))
                .map(|(a, b)| (a - b).powi(2))
                .sum();
            k * (2.0 * s).sqrt()
        })
        .sum::<f64>()
        / mr.len() as f64
}

/// dB by which the noise-only lead-in got quieter: 10 log10(E_in / E_out) over the first `lead_in`
/// samples. Positive means less noise left. 0 for an unprocessed signal.
pub fn lead_in_reduction_db(input: &[f32], output: &[f32], lead_in: usize) -> f64 {
    let e = |x: &[f32]| {
        x[..lead_in]
            .iter()
            .map(|&v| (v as f64).powi(2))
            .sum::<f64>()
            + 1e-12
    };
    db(e(input) / e(output))
}

/// Peak absolute sample in dBFS (0 dBFS = 1.0).
pub fn peak_dbfs(x: &[f32]) -> f64 {
    let p = x.iter().fold(0.0f32, |m, &v| m.max(v.abs())) as f64;
    20.0 * p.max(1e-10).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, mut s: u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s as f32 / u32::MAX as f32 - 0.5
            })
            .collect()
    }

    #[test]
    fn si_sdr_is_scale_invariant_and_matches_the_mix_snr() {
        let r = noise(48_000, 1);
        let n = noise(48_000, 2);
        // Uncorrelated noise at a tenth of the amplitude: about 20 dB.
        let e: Vec<f32> = r.iter().zip(&n).map(|(a, b)| 3.0 * (a + 0.1 * b)).collect();
        let v = si_sdr(&e, &r);
        assert!((v - 20.0).abs() < 0.2, "{v}");
        assert!(si_sdr(&r, &r) > 100.0);
    }

    #[test]
    fn lsd_is_zero_for_identity_and_grows_with_distortion() {
        let r = noise(24_000, 3);
        assert!(lsd(&r, &r).unwrap() < 1e-9);
        let half: Vec<f32> = r.iter().map(|v| v * 0.5).collect();
        // A 6.02 dB gain error on every bin.
        assert!((lsd(&half, &r).unwrap() - 6.0206).abs() < 0.01);
        assert_eq!(lsd(&r, &vec![0.0; 24_000]), None);
    }

    #[test]
    fn lsd_ignores_silent_reference_frames() {
        let mut r = noise(24_000, 4);
        r[..12_000].iter_mut().for_each(|v| *v = 0.0);
        let mut e = r.clone();
        // Garbage where the reference is silent is not speech distortion.
        e[..10_000].copy_from_slice(&noise(10_000, 5));
        assert!(lsd(&e, &r).unwrap() < 1e-9);
    }

    /// A crude 4th-order low-pass: four one-pole stages at `fc`.
    fn lowpass(x: &[f32], fc: f64) -> Vec<f32> {
        let a = (-2.0 * std::f64::consts::PI * fc / 48_000.0).exp();
        let mut y: Vec<f64> = x.iter().map(|&v| v as f64).collect();
        for _ in 0..4 {
            let mut s = 0.0;
            for v in &mut y {
                s = (1.0 - a) * *v + a * s;
                *v = s;
            }
        }
        y.into_iter().map(|v| v as f32).collect()
    }

    #[test]
    fn wideband_is_zero_for_identity_and_gain() {
        let r = noise(48_000, 6);
        let w = wideband(&r, &r).unwrap();
        assert!(
            w.lsd_wb.abs() < 1e-9 && w.hb_err_db.abs() < 1e-9 && w.mcd < 1e-9,
            "{w:?}"
        );
        // A pure gain change is not distortion for any of the three.
        let g: Vec<f32> = r.iter().map(|v| v * 0.25).collect();
        let w = wideband(&g, &r).unwrap();
        // (Only the fixed power floor is not scaled, hence the small tolerance.)
        assert!(
            w.lsd_wb < 1e-3 && w.hb_err_db.abs() < 1e-6 && w.mcd < 1e-2,
            "{w:?}"
        );
    }

    #[test]
    fn wideband_sees_a_lost_high_band() {
        let r = noise(48_000, 7);
        let lp = lowpass(&r, 4_000.0);
        let w = wideband(&lp, &r).unwrap();
        assert!(w.hb_err_db < -20.0, "{w:?}");
        assert!(w.lsd_wb > 5.0 && w.mcd > 1.0, "{w:?}");
        // Added broadband noise raises the high band share and the distances, but less.
        let n = noise(48_000, 8);
        let noisy: Vec<f32> = r.iter().zip(&n).map(|(a, b)| a + 0.3 * b).collect();
        let w2 = wideband(&noisy, &r).unwrap();
        assert!(w2.lsd_wb > 0.1 && w2.lsd_wb < w.lsd_wb, "{w2:?}");
        // A band-limited reference has no high band to judge.
        assert_eq!(wideband(&lp, &lowpass(&vec![0.0; 48_000], 4_000.0)), None);
    }

    #[test]
    fn mel_bank_covers_the_band() {
        let b = mel_bank(2048, WB_MELS);
        assert_eq!(b.len(), WB_MELS);
        assert!(
            b.last()
                .unwrap()
                .iter()
                .any(|&(k, _)| k as f64 * 48_000.0 / 2048.0 > 20_000.0)
        );
        // At 512 points the lowest triangles are narrower than a bin and are dropped.
        assert!(mel_bank(512, WB_MELS).len() < WB_MELS);
    }

    #[test]
    fn lead_in_and_peak() {
        let x = vec![0.5f32; 100];
        let y = vec![0.05f32; 100];
        assert!((lead_in_reduction_db(&x, &y, 100) - 20.0).abs() < 1e-6);
        assert!((peak_dbfs(&x) + 6.0206).abs() < 1e-3);
    }
}
