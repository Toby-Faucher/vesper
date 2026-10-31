# Backlog

One entry per hypothesis, in the order to try them. Start one with `research/scripts/attempt.sh <id>`.
Mark it `[x]` with the result once the gate has run. Each entry: hypothesis, expected effect, risk.

- [ ] **E0-lsnr-thresh**: config only. Raise DeepFilterNet's local-SNR stage-skip thresholds
  (`min_db_thresh`, `max_db_erb_thresh`, `max_db_df_thresh`) to 1000, so the model never skips a stage.
  - Hypothesis: at high local SNR the library skips the ERB stage (above 30 dB) and the deep filter (above 20 dB),
    and that wrecks near-clean speech (C1: 20 dB items came out at about 5.5 dB SI-SDR).
  - Expected: large gains on 10 and 20 dB items. On a 12-item scratch set (dev-like, not gate) C1 saw OVRL +0.36,
    SIG +0.25, SI-SDRi +9.9 dB.
  - Risk: more residual noise or artefacts on items the skip used to pass through; slightly higher RTF (no stage skipped).
- [ ] **E1-input-gain**: input gain before the model for quiet mics (`PipelineConfig.input_gain_db`), sweep +6, +12, +18 dB
  (one gate run per value, or pick on dev and gate one).
  - Hypothesis: at -35 dBFS speech sits near the model's floor, so it is suppressed with the noise.
    The web app's input boost is a gain node AFTER the denoiser (`capture.ts`), so it cannot help this today.
  - Expected: gains on the -35 dBFS items, none on loud items.
  - Risk: clipping on the -15 dBFS items (the peak guard catches it). Needs a level-aware gain to ship (see E5).
    If accepted, wiring it into the worker and removing the post gain is controller work (`web/` is protected).
- [ ] **E2-atten-lim**: sweep `atten_db` over 6, 12, 24, 40, 100 (100 is today).
  - Hypothesis: a limit keeps some noise but less speech damage, and DNSMOS SIG rewards that.
  - Expected: SIG and SI-SDRi up, BAK down; OVRL could go either way. Probably best near 24 to 40.
  - Risk: audible residual noise. BAK drops by design.
- [ ] **E3-post-filter**: sweep `post_filter_beta` over 0, 0.01, 0.02 (today), 0.05.
  - Hypothesis: the post filter trades speech detail for noise suppression; the shipped 0.02 was not measured.
  - Expected: small, under 0.05 OVRL either way.
  - Risk: low.
- [ ] **E4-highpass**: a 70 Hz high-pass pre-stage (second-order, causal) before the model.
  - Hypothesis: fan hum and desk rumble below 70 Hz carry no speech and use up the model's attention.
  - Expected: gains on fan and street items; nothing elsewhere.
  - Risk: thin male voices if the corner is too high. Latency stays 0.
- [ ] **E5-input-agc**: an input leveler (slow AGC) pre-stage that brings speech near -25 dBFS before the model.
  - Hypothesis: a level-aware version of E1 that does not clip loud mics.
  - Expected: the E1 gain on quiet items without the clipping on loud ones.
  - Risk: pumping on noise-only stretches (gate the AGC on speech activity); slow attack misses the first syllable.
- [ ] **E6-post-leveler**: a post leveler plus a true-peak limiter after the model.
  - Hypothesis: steady output level and no overs sound better and help Opus.
  - Expected: small DNSMOS change; the clipping guard stays green with a boost.
  - Risk: a limiter with lookahead adds latency (declare it). DNSMOS may not reward loudness at all.
- [ ] **E7-residual-gate**: a residual gate driven by the model's last local SNR (`last_lsnr`).
  - Hypothesis: in pauses the model leaves low-level noise; muting it when local SNR is very low raises BAK.
  - Expected: BAK up on noise-heavy items, lead-in residual down.
  - Risk: chopped word onsets and tails (SIG and SI-SDRi guards). Use hold and release times.
- [ ] **E8-model**: model choice: DeepFilterNet2, DeepFilterNet3, DeepFilterNet3 low latency, if the model tarballs can be fetched with a sha256.
  - Hypothesis: another model trades quality, latency and RTF differently.
  - Expected: unknown.
  - Risk: needs a gate that can pass `--model` (today the gate uses one model for both sides), and a fetch script.
    This is framework work for the controller before an agent can try it.
- [ ] **E9-opus**: Opus parameters (bitrate, complexity, FEC, DTX, packet loss) judged on decoded audio.
  - Hypothesis: the 64 kbps default is not where quality stops rising, and DTX may clip onsets after the denoiser.
  - Expected: unknown until the Opus stage exists.
  - Risk: needs the Opus stage in `audiolab` first (controller work, `crates/vesper-audiolab/` is protected).
- [ ] **E10-stack**: stack the accepted and promising inconclusive changes and gate them together.
  - Hypothesis: small gains that did not reach a verdict alone add up.
  - Expected: the sum of the parts, less overlap.
  - Risk: stages interact (for example E1 gain and E2 limit). Then run the final holdout check against `repo.origin_sha`, once.
