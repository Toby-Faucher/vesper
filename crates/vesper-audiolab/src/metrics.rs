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

    #[test]
    fn lead_in_and_peak() {
        let x = vec![0.5f32; 100];
        let y = vec![0.05f32; 100];
        assert!((lead_in_reduction_db(&x, &y, 100) - 20.0).abs() < 1e-6);
        assert!((peak_dbfs(&x) + 6.0206).abs() < 1e-3);
    }
}
