//! WAV and FLAC I/O, 3x resampling and FFT convolution.

use std::path::Path;

use anyhow::{Context, Result, bail};
use realfft::RealFftPlanner;

/// Lab sample rate (the browser capture rate).
pub const SR: u32 = 48_000;

/// Reads a WAV file as mono f32 in [-1, 1] (channels averaged). Returns the samples and the rate.
pub fn read_wav(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut r = hound::WavReader::open(path).with_context(|| format!("open {}", path.display()))?;
    let spec = r.spec();
    let ch = spec.channels as usize;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>()
                .map(|s| s.map(|s| s as f32 * scale))
                .collect::<Result<_, _>>()?
        }
    };
    let mono = raw
        .chunks(ch)
        .map(|c| c.iter().sum::<f32>() / ch as f32)
        .collect();
    Ok((mono, spec.sample_rate))
}

/// Reads a mono FLAC file as f32 in [-1, 1].
pub fn read_flac(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut r =
        claxon::FlacReader::open(path).with_context(|| format!("open {}", path.display()))?;
    let info = r.streaminfo();
    if info.channels != 1 {
        bail!("{}: expected mono FLAC", path.display());
    }
    let scale = 1.0 / (1u64 << (info.bits_per_sample - 1)) as f32;
    let x = r
        .samples()
        .map(|s| s.map(|s| s as f32 * scale))
        .collect::<Result<_, _>>()?;
    Ok((x, info.sample_rate))
}

/// Writes 16-bit mono WAV at [`SR`] (rounded, clamped to the i16 range).
pub fn write_wav_i16(path: &Path, x: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in x {
        w.write_sample(to_i16(s))?;
    }
    w.finalize()?;
    Ok(())
}

/// Writes 32-bit float mono WAV at [`SR`], so values above full scale survive.
pub fn write_wav_f32(path: &Path, x: &[f32]) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SR,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in x {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

pub fn to_i16(s: f32) -> i16 {
    (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

/// Rounds to the 16-bit grid, as a WAV round trip would.
pub fn quantize_i16(x: &mut [f32]) {
    for s in x {
        *s = to_i16(*s) as f32 / 32768.0;
    }
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > 1e-12 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

/// Kaiser-windowed sinc low-pass at 48 kHz for 3x rate changes: cutoff 7.7 kHz, 481 taps,
/// about 90 dB stopband from about 8 kHz. Unity DC gain.
fn lowpass_3x() -> &'static [f64] {
    static TAPS: std::sync::OnceLock<Vec<f64>> = std::sync::OnceLock::new();
    TAPS.get_or_init(|| {
        const HALF: usize = 240;
        const BETA: f64 = 9.0;
        let fc = 7_700.0 / SR as f64;
        let mut h: Vec<f64> = (0..=2 * HALF)
            .map(|i| {
                let n = i as f64 - HALF as f64;
                let sinc = if n == 0.0 {
                    2.0 * fc
                } else {
                    (2.0 * std::f64::consts::PI * fc * n).sin() / (std::f64::consts::PI * n)
                };
                let r = n / HALF as f64;
                sinc * bessel_i0(BETA * (1.0 - r * r).sqrt()) / bessel_i0(BETA)
            })
            .collect();
        let dc: f64 = h.iter().sum();
        h.iter_mut().for_each(|t| *t /= dc);
        h
    })
}

/// 48 kHz to 16 kHz: low-pass, then keep every third sample. Zero-phase (no delay).
pub fn down3(x: &[f32]) -> Vec<f32> {
    let h = lowpass_3x();
    let half = (h.len() / 2) as isize;
    (0..x.len() / 3)
        .map(|m| {
            let c = (3 * m) as isize;
            let mut acc = 0.0f64;
            for (k, &t) in h.iter().enumerate() {
                let i = c + half - k as isize;
                if i >= 0 && (i as usize) < x.len() {
                    acc += t * x[i as usize] as f64;
                }
            }
            acc as f32
        })
        .collect()
}

/// 16 kHz to 48 kHz: insert two zeros per sample, then low-pass with gain 3. Zero-phase.
pub fn up3(x: &[f32]) -> Vec<f32> {
    let h = lowpass_3x();
    let half = h.len() / 2;
    (0..x.len() * 3)
        .map(|n| {
            // y[n] = 3 * sum_j h[n + half - 3j] * x[j]
            let mut acc = 0.0f64;
            let top = n + half;
            let j_lo = top.saturating_sub(h.len() - 1).div_ceil(3);
            let j_hi = (top / 3).min(x.len().saturating_sub(1));
            for j in j_lo..=j_hi {
                acc += h[top - 3 * j] * x[j] as f64;
            }
            (3.0 * acc) as f32
        })
        .collect()
}

/// Linear convolution of `x` with `h`, cut to `x.len()` samples (FFT based).
pub fn convolve(x: &[f32], h: &[f32]) -> Vec<f32> {
    let n = (x.len() + h.len()).next_power_of_two();
    let mut planner = RealFftPlanner::<f64>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    let spec = |s: &[f32]| {
        let mut buf = vec![0.0f64; n];
        buf.iter_mut().zip(s).for_each(|(b, &v)| *b = v as f64);
        let mut out = fwd.make_output_vec();
        fwd.process(&mut buf, &mut out).expect("fft length");
        out
    };
    let (a, b) = (spec(x), spec(h));
    let mut prod: Vec<_> = a.iter().zip(&b).map(|(p, q)| p * q).collect();
    let mut out = inv.make_output_vec();
    inv.process(&mut prod, &mut out).expect("fft length");
    out.truncate(x.len());
    out.iter().map(|&v| (v / n as f64) as f32).collect()
}

/// Measured bandwidth (spectral rolloff) of speech at rate `sr`, in Hz: the upper edge of the
/// highest 250 Hz band whose long-term power is within 50 dB of the strongest band. Long-term
/// power is the mean over active 2048-point Hann frames (within 40 dB of the loudest frame), so
/// pauses and the recording's noise floor do not count. LibriSpeech gives at most 8000 (its
/// Nyquist); 16 kHz audio upsampled 3x gives about 7750.
pub fn bandwidth_hz(x: &[f32], sr: u32) -> f64 {
    const N: usize = 2048;
    const BAND_HZ: f64 = 250.0;
    if x.len() < N {
        return 0.0;
    }
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(N);
    let win: Vec<f64> = (0..N)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N as f64).cos())
        .collect();
    let mut buf = vec![0.0; N];
    let mut out = fft.make_output_vec();
    let frames: Vec<Vec<f64>> = (0..=(x.len() - N) / (N / 2))
        .map(|f| {
            let s = &x[f * N / 2..f * N / 2 + N];
            buf.iter_mut()
                .zip(s.iter().zip(&win))
                .for_each(|(b, (&v, &w))| *b = v as f64 * w);
            fft.process(&mut buf, &mut out).expect("fft length");
            out.iter().map(|c| c.norm_sqr()).collect()
        })
        .collect();
    let e: Vec<f64> = frames.iter().map(|p| p.iter().sum()).collect();
    let max_e = e.iter().copied().fold(0.0, f64::max);
    if max_e <= 0.0 {
        return 0.0;
    }
    let bin_hz = sr as f64 / N as f64;
    let n_bands = ((sr as f64 / 2.0) / BAND_HZ).ceil() as usize;
    let mut bands = vec![0.0; n_bands];
    for (p, _) in frames.iter().zip(&e).filter(|(_, e)| **e >= max_e * 1e-4) {
        for (k, v) in p.iter().enumerate().skip(1) {
            bands[((k as f64 * bin_hz / BAND_HZ) as usize).min(n_bands - 1)] += v;
        }
    }
    let top = bands.iter().copied().fold(0.0, f64::max);
    let last = bands.iter().rposition(|&b| b >= top * 1e-5).unwrap_or(0);
    ((last + 1) as f64 * BAND_HZ).min(sr as f64 / 2.0)
}

pub fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| (v as f64).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

pub fn db(power_ratio: f64) -> f64 {
    10.0 * power_ratio.log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f64, sr: f64, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / sr).sin() as f32)
            .collect()
    }

    fn mid_rms(x: &[f32]) -> f64 {
        rms(&x[x.len() / 4..3 * x.len() / 4])
    }

    #[test]
    fn down3_keeps_passband_and_kills_aliases() {
        let pass = down3(&sine(1_000.0, 48_000.0, 48_000));
        assert!((mid_rms(&pass) / std::f64::consts::FRAC_1_SQRT_2 - 1.0).abs() < 1e-3);
        // 12 kHz would alias to 4 kHz at 16 kHz.
        let stop = down3(&sine(12_000.0, 48_000.0, 48_000));
        assert!(db(mid_rms(&stop).powi(2) * 2.0) < -80.0);
    }

    #[test]
    fn up3_then_down3_is_near_identity() {
        let x = sine(440.0, 16_000.0, 16_000);
        let y = down3(&up3(&x));
        let err: Vec<f32> = x.iter().zip(&y).map(|(a, b)| a - b).collect();
        assert!(db((mid_rms(&err) / mid_rms(&x)).powi(2)) < -60.0);
    }

    #[test]
    fn bandwidth_of_upsampled_and_full_band_noise() {
        let mut s = 9u32;
        let mut white = |n: usize| -> Vec<f32> {
            (0..n)
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 17;
                    s ^= s << 5;
                    s as f32 / u32::MAX as f32 - 0.5
                })
                .collect()
        };
        let up = up3(&white(16_000));
        let bw = bandwidth_hz(&up, SR);
        assert!((7_500.0..=8_250.0).contains(&bw), "{bw}");
        assert_eq!(bandwidth_hz(&white(48_000), SR), 24_000.0);
        assert_eq!(bandwidth_hz(&white(16_000), 16_000), 8_000.0);
    }

    #[test]
    fn convolve_matches_direct() {
        let x: Vec<f32> = (0..300).map(|i| ((i * 7919) % 13) as f32 - 6.0).collect();
        let h = [0.5f32, -0.25, 0.0, 0.125];
        let y = convolve(&x, &h);
        for n in 0..x.len() {
            let d: f32 = (0..h.len())
                .filter(|&k| k <= n)
                .map(|k| h[k] * x[n - k])
                .sum();
            assert!((y[n] - d).abs() < 1e-4, "{n}: {} vs {d}", y[n]);
        }
    }
}
