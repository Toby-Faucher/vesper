# Baseline on corpus v2

The shipped capture path (`PipelineConfig::default()` on main 7f914e8: DeepFilterNet3, atten 100 dB,
post filter 0.02, E0 stage-skip thresholds at 1000) against the unprocessed mixtures, on the dev split of
corpus v2 (160 items, 10 VCTK speakers, 48 kHz speech; see [docs/research-loop.md](../docs/research-loop.md)).
Measured with `audiolab` at commit 7d5b148, `--jobs 6`, inside `scripts/capped.sh` (5 GiB cap, nice 19)
on the shared dev box. Raw results: `research/results/v2/dev-{default,unprocessed}.json` (git-ignored).

```bash
audiolab eval --config research/configs/default.toml --split dev --corpus research/corpus48/built \
  --second-mos research/models/utmos22_strong.onnx --out research/results/v2/dev-default.json
audiolab eval --unprocessed ...same flags... --out research/results/v2/dev-unprocessed.json
```

## Whole split

| | OVRL | SIG | BAK | UTMOS | lsd_wb dB | hb_err dB | MCD dB | SI-SDR dB | SI-SDRi dB | lead-in reduction dB | RTF | peak RSS MiB | wall s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| unprocessed | 2.157 | 2.682 | 2.697 | 1.855 | 9.74 | +7.14 | 18.79 | 6.49 | 0.00 | 0.0 | none | 2195 | 313 |
| default (E0) | 2.591 | 2.882 | 3.945 | 2.347 | 9.11 | -1.16 | 16.56 | 8.23 | 1.74 | 32.1 | 0.0286 | 2209 | 345 |

Paired deltas, default minus control, mean with a 95 percent normal interval (and the per-item sd):
OVRL +0.434 [+0.363, +0.505] (sd 0.46), SIG +0.199 [+0.102, +0.297], BAK +1.247 [+1.110, +1.385],
UTMOS +0.492 [+0.418, +0.565] (sd 0.47), lsd_wb -0.63 dB [-1.31, +0.05] (sd 4.4), hb_err -8.3 dB,
MCD -2.2 dB [-4.0, -0.4], SI-SDRi +1.74 dB [+0.34, +3.15].

- "Residual" is the lead-in reduction: how many dB quieter the 1.5 s noise-only lead-in is after
  processing (the noise left in pauses). Its mean hides a wide spread (sd 39 dB). Mean per noise:
  second talker 3 dB, street 6, babble 8, cafe 9, TV 12, AC hiss 16, fan 23, door slam 27, music 36,
  vacuum 43, mouth clicks 53, freesound2530 84, keyboard 105 (sparse events: the lead-in is near
  silent once they are gone).
- Wall time and peak RSS include UTMOS (one shared 0.9 GB model, scoring takes most of the 5 to 6
  minutes). Without UTMOS a dev eval takes about a minute and peaks near 0.9 GB (the Opus runs below).
- RTF is one thread on 30 s (the split's first three items), median of 3, on a shared machine.
- Latency: 0 samples from our stages, 1440 from the model (unchanged).
- No clipped output item.

Both judges agree on direction: DNSMOS OVRL and UTMOS rise by about the same amount. The wideband
distance improves only a little on average (9.74 to 9.11 dB, interval touches 0), and the high band
goes from far too much (+7.1 dB of noise share in 8 to 20 kHz) to slightly too little (-1.2 dB).

## Per noise type (dev, sorted by OVRL after processing)

| noise | n | OVRL control | OVRL default | delta | UTMOS control | UTMOS default | lsd_wb default | hb_err default dB | SI-SDRi default dB |
|---|---|---|---|---|---|---|---|---|---|
| vacuum | 11 | 1.533 | 2.237 | +0.704 | 1.597 | 2.018 | 10.14 | -14.32 | +6.82 |
| freesound2530 | 12 | 1.906 | 2.364 | +0.458 | 1.570 | 2.186 | 10.15 | -4.19 | +4.06 |
| babble | 12 | 2.402 | 2.424 | +0.021 | 1.307 | 1.558 | 14.79 | +4.79 | -9.38 |
| ac_hiss | 12 | 1.933 | 2.429 | +0.496 | 1.795 | 2.263 | 9.38 | -8.16 | +5.20 |
| cafe | 11 | 1.958 | 2.431 | +0.473 | 1.348 | 1.701 | 14.73 | +2.00 | -1.42 |
| keyboard | 12 | 1.974 | 2.460 | +0.485 | 1.875 | 2.537 | 7.48 | -0.63 | +7.60 |
| door_slam | 11 | 2.343 | 2.587 | +0.244 | 2.234 | 2.582 | 6.83 | +0.50 | +8.83 |
| fan | 12 | 1.988 | 2.668 | +0.679 | 2.117 | 2.583 | 6.49 | -0.52 | +6.48 |
| tv | 11 | 2.333 | 2.718 | +0.385 | 1.519 | 2.146 | 10.85 | +1.84 | -4.72 |
| music | 11 | 1.748 | 2.726 | +0.979 | 1.917 | 2.839 | 6.67 | -0.34 | +2.67 |
| mouth_clicks | 11 | 2.543 | 2.745 | +0.202 | 2.463 | 2.773 | 7.46 | +0.98 | -3.73 |
| freesound573577 | 12 | 2.225 | 2.773 | +0.548 | 2.003 | 2.551 | 7.13 | +0.55 | +2.59 |
| street | 11 | 2.512 | 2.844 | +0.332 | 2.275 | 2.883 | 7.37 | +0.05 | +2.04 |
| second_talker | 11 | 2.849 | 2.912 | +0.063 | 1.990 | 2.272 | 8.04 | +1.34 | -3.21 |

## Per room

| room | n | OVRL control | OVRL default | delta | UTMOS control | UTMOS default | lsd_wb default | hb_err default dB | SI-SDRi default dB |
|---|---|---|---|---|---|---|---|---|---|
| RT60 1.0 s | 20 | 1.558 | 2.017 | +0.459 | 1.297 | 1.359 | 11.13 | -0.61 | -5.09 |
| RT60 0.6 s | 20 | 1.700 | 2.349 | +0.649 | 1.296 | 1.542 | 11.81 | +0.52 | -5.03 |
| RT60 0.3 s | 20 | 1.760 | 2.434 | +0.674 | 1.475 | 1.901 | 8.61 | -4.67 | +3.14 |
| DeepFilterNet RIR (0.79 s) | 20 | 1.832 | 2.456 | +0.624 | 1.348 | 1.740 | 9.75 | -1.34 | +0.80 |
| dry | 80 | 2.602 | 2.869 | +0.266 | 2.356 | 3.058 | 7.90 | -0.79 | +5.03 |

Each cell is 11 to 20 items: read the differences between rows as leads, not results.

## Where the model is weak

- **Speech on speech.** Babble (+0.02 OVRL), second talker (+0.06) and cafe gain little or nothing,
  and SI-SDRi goes negative (babble -9.4 dB): the model keeps or damages the wrong voice. lsd_wb is worst
  here (14.7 to 14.8 dB). A single-channel denoiser cannot pick the target talker; this is the biggest
  gap, and the old corpus had only babble and TV to show it.
- **Impulsive noise.** Mouth clicks (+0.20) and door slams (+0.24) gain the least of the non-speech
  noises. For clicks SI-SDRi drops (-3.7 dB): the clicks sit at 10 to 30 dB SNR, so the model mostly
  damages clean-ish speech.
- **High band under broadband noise.** Vacuum (-14.3 dB) and AC hiss (-8.2 dB) lose most of the 8 to
  20 kHz speech band with the noise. DNSMOS and UTMOS work at 16 kHz and cannot see this; hb_err_db can.
- **Reverberation.** RT60 1.0 s items end lowest (OVRL 2.02, UTMOS 1.36) and SI-SDRi is negative at
  0.6 and 1.0 s: the model removes part of the reverberant tail that the (reverberant) reference keeps.
  Dry items gain least on OVRL (+0.27) because they start highest.
- **Low rumble.** Street noise (brown and pink, passing cars) keeps all but 6 dB of its lead-in energy,
  the least of the non-speech noises, though OVRL ends high (2.84). Not checked: whether what is left
  is mostly low-frequency energy that DNSMOS weighs little.
- **Loudness.** The quietest level (-40 dBFS) gains least (+0.32 vs +0.47 to +0.52).

## Opus round trip on dev (default pipeline, no UTMOS)

`--opus` with the defaults (complexity 10, VOIP, DTX off) and the keys shown. 10 percent loss is
independent, seeded per item: the same 8134 of about 80 000 packets are lost in every lossy run. Deltas
are against the unencoded default output above.

| Opus | OVRL | SIG | BAK | lsd_wb dB | hb_err dB | MCD dB | SI-SDR dB |
|---|---|---|---|---|---|---|---|
| none (unencoded) | 2.591 | 2.882 | 3.945 | 9.11 | -1.16 | 16.56 | 8.23 |
| 128 kbps, no loss | 2.719 (+0.127) | 3.009 | 4.000 | 9.40 (+0.29) | -1.09 | 17.53 | 4.75 (-3.48) |
| 64 kbps, no loss | 2.704 (+0.112) | 2.986 | 4.030 | 9.29 (+0.18) | -0.90 | 17.69 | 2.49 (-5.74) |
| 64 kbps, 10 % loss, FEC | 2.684 (+0.093) | 2.961 | 4.045 | 9.33 (+0.22) | -1.12 | 17.93 | 2.00 (-6.23) |
| 64 kbps, 10 % loss, no FEC (PLC) | 2.547 (-0.044) | 2.818 | 4.005 | 9.49 (+0.38) | -0.60 | 18.72 | 0.52 (-7.71) |
| 24 kbps, 10 % loss, FEC | 2.623 (+0.032) | 2.899 | 4.037 | 10.33 (+1.22) | -16.30 | 18.67 | 1.57 (-6.66) |
| 24 kbps, 10 % loss, no FEC (PLC) | 2.506 (-0.085) | 2.775 | 4.004 | 9.52 (+0.41) | -0.52 | 19.12 | 0.11 (-8.11) |

- FEC beats concealment at 10 percent loss at both rates on DNSMOS, MCD and SI-SDR, and on lsd_wb at 64 kbps.
- A lossless round trip scores HIGHER on DNSMOS than no codec at all (+0.11 to +0.13). Not explained;
  one guess is the VOIP high-pass removing low rumble. Compare Opus runs only with Opus runs.
- At 24 kbps with FEC, libopus drops to a band-limited mode: hb_err -16.3 dB, lsd_wb +1.2 dB. DNSMOS
  (16 kHz) rates that run above PLC; only the wideband judge sees the lost high band.
- SI-SDR falls 3.5 dB even at 128 kbps without loss: the VOIP high-pass shifts the phase of low
  harmonics (docs/research-loop.md, Opus stage).
