//! Opus round trip with simulated packet loss: encode at 48 kHz mono in 20 ms (or 10 ms) packets,
//! drop packets (independent or Gilbert-Elliott bursts, seeded), decode a lost packet from the
//! next one when it arrived: its RED copy (RFC 2198, distance 1) if RED is on, else its in-band
//! FEC if FEC is on, else packet loss concealment. libopus is the bundled
//! build of the `opus` crate (opusic-sys), so results do not depend on the system library.

use std::str::FromStr;

use anyhow::{Context, Result, bail};
use opus::{Application, Bitrate, Channels, Decoder, Encoder};
use rand::{RngExt, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::audio::SR;

/// 20 ms at 48 kHz, the browser's default packet size.
pub const FRAME: usize = 960;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpusCfg {
    pub bitrate: i32,
    pub complexity: i32,
    pub fec: bool,
    pub dtx: bool,
    /// Packet loss rate, 0 to 1. Also told to the encoder (`OPUS_SET_PACKET_LOSS_PERC`), as WebRTC does.
    pub loss: f64,
    /// Mean burst length in packets for Gilbert-Elliott loss. None: independent losses.
    pub burst: Option<f64>,
    pub seed: u64,
    /// `OPUS_APPLICATION_AUDIO` instead of `VOIP`. WebRTC uses VOIP for mono, and VOIP high-passes
    /// the input (libopus `hp_cutoff`), which shifts the phase of low harmonics: SI-SDR after a
    /// VOIP round trip stays near 10 dB on low voices at any bitrate.
    #[serde(default)]
    pub audio: bool,
    /// Packet length in ms: 20 (the default) or 10.
    #[serde(default = "default_frame_ms")]
    pub frame_ms: u32,
    /// RFC 2198 redundancy with distance 1: each packet also carries the previous packet's
    /// payload, so a single lost packet decodes at full quality when the next one arrived.
    /// Extra bits are not counted against `bitrate` (WebRTC sends RED on top of the Opus rate).
    #[serde(default)]
    pub red: bool,
}

fn default_frame_ms() -> u32 {
    20
}

impl Default for OpusCfg {
    fn default() -> Self {
        Self {
            bitrate: 64_000,
            complexity: 10,
            fec: true,
            dtx: false,
            loss: 0.0,
            burst: None,
            seed: 1,
            audio: false,
            frame_ms: 20,
            red: false,
        }
    }
}

impl FromStr for OpusCfg {
    type Err = anyhow::Error;

    /// `bitrate=64000,complexity=10,fec=1,dtx=0,loss=0.0,seed=1[,burst=3][,app=voip|audio][,frame=10|20][,red=1]`;
    /// missing keys keep the default.
    fn from_str(s: &str) -> Result<Self> {
        let mut c = OpusCfg::default();
        let flag = |v: &str| -> Result<bool> {
            match v {
                "1" | "true" => Ok(true),
                "0" | "false" => Ok(false),
                _ => bail!("expected 0 or 1, got {v}"),
            }
        };
        for kv in s.split(',').filter(|x| !x.is_empty()) {
            let (k, v) = kv
                .split_once('=')
                .with_context(|| format!("expected key=value, got {kv}"))?;
            match k {
                "bitrate" => c.bitrate = v.parse()?,
                "complexity" => c.complexity = v.parse()?,
                "fec" => c.fec = flag(v)?,
                "dtx" => c.dtx = flag(v)?,
                "loss" => c.loss = v.parse()?,
                "burst" => c.burst = Some(v.parse()?),
                "seed" => c.seed = v.parse()?,
                "frame" => c.frame_ms = v.parse()?,
                "red" => c.red = flag(v)?,
                "app" => {
                    c.audio = match v {
                        "voip" => false,
                        "audio" => true,
                        _ => bail!("app must be voip or audio"),
                    }
                }
                _ => bail!("unknown opus option {k}"),
            }
        }
        if !(0.0..1.0).contains(&c.loss) {
            bail!("loss must be in [0, 1)");
        }
        if c.frame_ms != 10 && c.frame_ms != 20 {
            bail!("frame must be 10 or 20 (ms)");
        }
        if c.burst.is_some_and(|b| b < 1.0) {
            bail!("burst must be at least 1 packet");
        }
        Ok(c)
    }
}

/// Which of `n` packets are lost. Gilbert-Elliott: every packet in the bad state is lost, the
/// bad state lasts `burst` packets on average, and the long-run loss rate is `loss`.
pub fn loss_pattern(n: usize, loss: f64, burst: Option<f64>, seed: u64) -> Vec<bool> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    match burst {
        None => (0..n).map(|_| rng.random::<f64>() < loss).collect(),
        Some(b) => {
            let r = 1.0 / b; // bad to good
            let p = loss * r / (1.0 - loss); // good to bad
            let mut bad = false;
            (0..n)
                .map(|_| {
                    bad = if bad {
                        rng.random::<f64>() >= r
                    } else {
                        rng.random::<f64>() < p
                    };
                    bad
                })
                .collect()
        }
    }
}

/// Encodes `x` (48 kHz mono), loses packets with `seed`, decodes, and removes the codec delay so
/// the result lines up with `x` and has its length. Returns it with the number of lost packets.
pub fn roundtrip(x: &[f32], cfg: &OpusCfg, seed: u64) -> Result<(Vec<f32>, usize)> {
    let app = if cfg.audio {
        Application::Audio
    } else {
        Application::Voip
    };
    let mut enc = Encoder::new(SR, Channels::Mono, app)?;
    enc.set_bitrate(Bitrate::Bits(cfg.bitrate))?;
    enc.set_complexity(cfg.complexity)?;
    enc.set_inband_fec(cfg.fec)?;
    enc.set_dtx(cfg.dtx)?;
    enc.set_packet_loss_perc((cfg.loss * 100.0).round() as i32)?;
    let delay = enc.get_lookahead()? as usize;
    let fl = SR as usize * cfg.frame_ms as usize / 1000;
    let mut input = x.to_vec();
    input.resize((x.len() + delay).div_ceil(fl) * fl, 0.0);
    let mut packets = Vec::with_capacity(input.len() / fl);
    let mut buf = [0u8; 1500];
    for f in input.chunks_exact(fl) {
        let n = enc.encode_float(f, &mut buf)?;
        packets.push(buf[..n].to_vec());
    }
    let lost = loss_pattern(packets.len(), cfg.loss, cfg.burst, seed);
    let mut dec = Decoder::new(SR, Channels::Mono)?;
    let mut out = Vec::with_capacity(input.len());
    let mut frame = vec![0.0f32; fl];
    for i in 0..packets.len() {
        let next_arrived = i + 1 < packets.len() && !lost[i + 1];
        let n = if !lost[i] || (cfg.red && next_arrived) {
            // Arrived, or its RED copy rode in the next packet: the same bytes either way.
            dec.decode_float(&packets[i], &mut frame, false)?
        } else if cfg.fec && next_arrived {
            // One packet of jitter buffer: the next packet carries this one's FEC copy.
            dec.decode_float(&packets[i + 1], &mut frame, true)?
        } else {
            dec.decode_float(&[], &mut frame, false)?
        };
        if n != fl {
            bail!("opus decoded {n} samples, expected {fl}");
        }
        out.extend_from_slice(&frame);
    }
    Ok((
        out[delay..delay + x.len()].to_vec(),
        lost.iter().filter(|&&l| l).count(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::si_sdr;

    /// 6 s of speech-like sound: a 140 Hz harmonic voice with moving formants, syllable
    /// envelopes, and some broadband noise.
    fn voice() -> Vec<f32> {
        let mut s = 3u32;
        (0..6 * SR as usize)
            .map(|i| {
                let t = i as f64 / SR as f64;
                let syll = (0.5 + 0.5 * (2.0 * std::f64::consts::PI * 4.0 * t).sin()).powi(2);
                let f1 = 500.0 + 300.0 * (2.0 * std::f64::consts::PI * 1.3 * t).sin();
                let v: f64 = (1..40)
                    .map(|h| {
                        let f = 140.0 * h as f64;
                        let w = (-((f - f1) / 400.0).powi(2)).exp() + 0.3 / h as f64;
                        w * (2.0 * std::f64::consts::PI * f * t).sin()
                    })
                    .sum();
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let n = s as f64 / u32::MAX as f64 - 0.5;
                (0.05 * syll * v + 0.002 * n) as f32
            })
            .collect()
    }

    #[test]
    fn parses_options() {
        let c: OpusCfg =
            "bitrate=32000,complexity=5,fec=0,dtx=1,loss=0.1,seed=4,burst=2.5,app=audio"
                .parse()
                .unwrap();
        assert_eq!(
            c,
            OpusCfg {
                bitrate: 32_000,
                complexity: 5,
                fec: false,
                dtx: true,
                loss: 0.1,
                burst: Some(2.5),
                seed: 4,
                audio: true,
                frame_ms: 20,
                red: false,
            }
        );
        let r: OpusCfg = "frame=10,red=1".parse().unwrap();
        assert_eq!((r.frame_ms, r.red), (10, true));
        assert!("frame=5".parse::<OpusCfg>().is_err());
        assert!("loss=1.0".parse::<OpusCfg>().is_err());
        assert!("bitrat=1".parse::<OpusCfg>().is_err());
        assert_eq!("".parse::<OpusCfg>().unwrap(), OpusCfg::default());
    }

    #[test]
    fn loss_patterns_hit_their_rate() {
        let n = 200_000;
        let iid = loss_pattern(n, 0.1, None, 1);
        let rate = |v: &[bool]| v.iter().filter(|&&l| l).count() as f64 / v.len() as f64;
        assert!((rate(&iid) - 0.1).abs() < 0.005);
        let ge = loss_pattern(n, 0.1, Some(4.0), 1);
        assert!((rate(&ge) - 0.1).abs() < 0.01, "{}", rate(&ge));
        // Mean run length of losses is the burst length.
        let runs = ge.windows(2).filter(|w| w[0] && !w[1]).count();
        let mean_run = ge.iter().filter(|&&l| l).count() as f64 / runs as f64;
        assert!((mean_run - 4.0).abs() < 0.3, "{mean_run}");
        assert_eq!(
            loss_pattern(100, 0.3, None, 7),
            loss_pattern(100, 0.3, None, 7)
        );
    }

    #[test]
    fn lossless_128k_is_transparent_enough_and_aligned() {
        let x = voice();
        let cfg = OpusCfg {
            bitrate: 128_000,
            audio: true,
            ..OpusCfg::default()
        };
        let (y, lost) = roundtrip(&x, &cfg, 1).unwrap();
        assert_eq!((y.len(), lost), (x.len(), 0));
        let s = si_sdr(&y, &x);
        assert!(s > 20.0, "SI-SDR {s}");
        // Aligned: a one-sample shift either way scores lower.
        assert!(si_sdr(&y[1..], &x[..x.len() - 1]) < s);
        assert!(si_sdr(&y[..y.len() - 1], &x[1..]) < s);
        // VOIP (the WebRTC mode) high-passes: same delay, but the phase shift caps SI-SDR.
        let voip = OpusCfg {
            audio: false,
            ..cfg
        };
        let v = si_sdr(&roundtrip(&x, &voip, 1).unwrap().0, &x);
        assert!((5.0..15.0).contains(&v), "VOIP SI-SDR {v}");
    }

    /// At 24 kbps (SILK, where in-band FEC lives) FEC beats concealment. At 64 kbps FEC
    /// forces hybrid mode on this test signal and scores lower than plain CELT with
    /// concealment, so this test does not claim it there (see docs/research-loop.md).
    #[test]
    fn fec_beats_plain_concealment_at_10_percent_loss() {
        let x = voice();
        let mean = |fec: bool| -> (f64, f64) {
            let cfg = OpusCfg {
                bitrate: 24_000,
                fec,
                loss: 0.1,
                ..OpusCfg::default()
            };
            let runs: Vec<Vec<f32>> = (0..5).map(|s| roundtrip(&x, &cfg, s).unwrap().0).collect();
            (
                runs.iter().map(|y| si_sdr(y, &x)).sum::<f64>() / 5.0,
                runs.iter()
                    .map(|y| crate::metrics::lsd(y, &x).unwrap())
                    .sum::<f64>()
                    / 5.0,
            )
        };
        let (with, without) = (mean(true), mean(false));
        assert!(
            with.0 > without.0 + 0.3,
            "SI-SDR fec {with:?} vs plc {without:?}"
        );
        assert!(
            with.1 < without.1 - 0.2,
            "LSD fec {with:?} vs plc {without:?}"
        );
    }

    /// RED (distance 1) decodes an isolated lost packet from its full copy; the test checks only that with
    /// independent 5 percent loss it beats FEC alone; and 10 ms packets keep the output aligned and the right length.
    #[test]
    fn red_recovers_isolated_losses_and_10ms_aligns() {
        let x = voice();
        let base = OpusCfg {
            loss: 0.05,
            frame_ms: 10,
            ..OpusCfg::default()
        };
        let clean = roundtrip(
            &x,
            &OpusCfg {
                loss: 0.0,
                ..base.clone()
            },
            1,
        )
        .unwrap()
        .0;
        assert_eq!(clean.len(), x.len());
        let s0 = si_sdr(&clean, &x);
        assert!(
            si_sdr(&clean[1..], &x[..x.len() - 1]) < s0,
            "10 ms output misaligned"
        );
        let (red, lost) = roundtrip(
            &x,
            &OpusCfg {
                red: true,
                ..base.clone()
            },
            1,
        )
        .unwrap();
        assert!(lost > 0);
        let fec = roundtrip(&x, &base, 1).unwrap().0;
        assert!(
            si_sdr(&red, &x) > si_sdr(&fec, &x),
            "RED {} vs FEC {}",
            si_sdr(&red, &x),
            si_sdr(&fec, &x)
        );
    }
}
