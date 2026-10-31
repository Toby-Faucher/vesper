# Audio research loop

A measured loop for the voice path. One hypothesis per experiment, each in its own git worktree. Agents make the
change; only the controller runs the gate. A statistical gate decides, and `research/CATALOG.md` records every
result, failures included. The design follows the Lemonate chess research loop, without the chess parts.

The corpus, the metrics and the `audiolab` CLI come first: see `audiolab --help`. The gate uses corpus v2:

```bash
research/scripts/fetch-corpus48.sh     # VCTK subset, 118 MB, sha256-checked
research/scripts/fetch-dnsmos.sh       # DNSMOS P.835
research/scripts/fetch-utmos.sh        # UTMOS, 411 MB (optional: second_mos)
audiolab corpus build --name v2 --seed 1   # research/corpus48/built, 1.3 GB, about 10 s
```

The old corpus (v1, LibriSpeech) still builds with `research/scripts/fetch-corpus.sh` and
`audiolab corpus build --seed 1` into `research/corpus/built`.

## Corpus v2

- **Speech:** 40 speakers of the CSTR VCTK Corpus 0.92 (University of Edinburgh, CC BY 4.0,
  https://doi.org/10.7488/ds/2645), 20 female and 20 male, 24 newspaper sentences each (utterance 030
  and up, so no two speakers read the same text), the mic2 (Sennheiser MKH 800) FLAC files. They are
  recorded at 96 kHz and released at 48 kHz 16-bit, and used as they are: no resampling. The fetch reads
  only these 960 files out of the 11 GB zip with HTTP range requests. Speakers are split 10 / 20 / 10
  (dev, gate, holdout), half of each gender in each split, disjoint.
- **Measured bandwidth.** `audio::bandwidth_hz` gives each file's spectral rolloff: the top of the
  highest 250 Hz band whose long-term power is within 50 dB of the strongest band, over active frames.
  Each item lists it per speech file (`speech_bandwidth_hz`), each manifest gives min, median and max.
  Over the 887 files the corpus uses: median 16 kHz, 10th percentile 11.75 kHz, 90th percentile
  20.25 kHz. Six files of one holdout speaker (p263) fall at 6.25 to 8 kHz. LibriSpeech, for comparison,
  stops at 8 kHz by construction.
- **Noise:** the 8 v1 types (two DeepFilterNet recordings, fan, keyboard, AC hiss, babble, street, TV)
  plus 6 synthetic ones: a second talker 5 to 15 dB below the target, music (notes, bass line, hi-hat
  bursts), cafe (6 talkers, murmur, dish clinks), vacuum or hair dryer (motor harmonics on broadband
  noise up to 20 kHz), door slams (damped 50 to 150 Hz thuds with a click), mouth clicks (sub-2 ms
  clicks at 10 to 30 dB SNR). Talkers come from the same split, so splits stay disjoint.
- **Rooms:** half the items dry; the rest split evenly over the DeepFilterNet RIR (RT60 0.79 s) and
  synthetic RIRs of RT60 0.3, 0.6 and 1.0 s (`corpus::synth_rir`: a direct path and an exponentially
  decaying noise tail, direct-to-reverberant ratio +3 dB at 0.3 s down to -3 dB at 1.0 s; no discrete
  early reflections). The clean reference of a reverberant item is the reverberant speech.
- **Grid:** SNR -5, 0, 5, 10, 20 dB; whole-item level -40, -32, -24 dBFS RMS; 10 s items (1.5 s noise
  lead-in) at 48 kHz mono; 160 / 400 / 160 items. 14 noises x 5 SNRs x 3 levels x 8 room slots does not
  fit as a full grid, so each factor is balanced over the split on its own and shuffled with the seed.
- **Headroom.** v1 asked for -15 dBFS RMS, which speech cannot reach without clipping, so about a third
  of those items were scaled down by up to 15 dB. v2 tops out at -24 dBFS. An item that would still peak
  above 0.98 is scaled down as a whole and `level_dbfs_actual` records it: 9 of 160 dev items, 21 of 400
  gate, 6 of 160 holdout, almost all at -24 dBFS with impulsive noise (door slams, keyboard, mouth
  clicks).

## Judges

Every eval scores all of these; `compare` shows paired deltas, `compare --by noise|rir|snr|level`
splits OVRL, lsd_wb and SI-SDRi by condition.

| score | needs the clean reference | what it rewards | how it can be gamed |
|---|---|---|---|
| DNSMOS SIG, BAK, OVRL | no | P.835 ratings predicted at 16 kHz: clean-sounding speech (SIG), quiet background (BAK) | blind above 8 kHz; BAK rewards over-suppression, SIG rewards leaving noise in (E2-atten24) |
| `second_mos` (UTMOS22) | no | natural-sounding speech (trained on TTS and natural speech ratings), 16 kHz | blind above 8 kHz; not trained on enhancement artefacts or noise; reported, not gated |
| `lsd_wb` | yes | a mel spectrum close to the clean one over 0 to 24 kHz, at 10.7, 21.3 and 42.7 ms resolution | leaving noise in can lower it (`atten_db = 24` improved it by 1 dB on gate while BAK fell); ignores phase and a whole-item gain; the 80 dB floor caps the cost of a deleted band |
| `hb_err_db` | yes | the right share of energy in 8 to 20 kHz (negative: high band lost, positive: high-band noise or artefacts left) | a mean of signed errors can cancel across items; adding noise in the high band can hide a lost high band |
| `mcd` | yes | the right spectral envelope (mel cepstrum 1 to 24, level-free) | blind to fine structure and noise in valleys below its 50 dB floor |
| SI-SDR, SI-SDRi | yes | the waveform itself | punishes harmless phase changes (all-pass filters, the Opus VOIP high-pass) |

`lsd_wb`, `hb_err_db` and `mcd` (`metrics::wideband`) use speech-active frames only: frames whose clean
energy is within 40 dB of the loudest frame. They return nothing when the reference has no energy in 8 to
20 kHz (the v1 corpus).

## Honesty rules

- "Better" means a higher DNSMOS P.835 OVRL score, a learned quality model, plus guard metrics, including a
  second judge that compares with the clean speech over the full band (`lsd_wb`). They are proxies. They can be
  gamed: over-suppression scores well on BAK, leaving noise in scores well on SIG and lsd_wb. The guards are
  there for that, but they do not replace ears.
- Every accepted change gets A/B files (`audiolab listen`) for Toby to hear. A number alone never proves a change sounds better.
- Nothing from the loop is merged into `main` or deployed without Toby. Work stays on branches.
- A rejected experiment is a result. Report it with its numbers.

## Run it

```bash
# 1. Start an attempt. Prints the path of the filled-in agent prompt.
research/scripts/attempt.sh E0-lsnr-thresh            # base: research/audio-main
research/scripts/attempt.sh E10-stack --base <ref>    # any other base

# 2. Give the prompt to one researcher agent. It writes hypothesis.md first, changes
#    crates/vesper-dsp/src/ or adds pipeline.toml, commits on exp/<id>, then writes trace.md.

# 3. Run the gate yourself (about 5 minutes; the baseline eval is cached per base commit).
research/scripts/gate.sh E0-lsnr-thresh
research/scripts/gate.sh --dry-run E0-lsnr-thresh    # preflight, then print the eval commands only

# 4. Read research/CATALOG.md and research/experiments/<id>/result.json.
```

The first `attempt.sh` creates `research/audio-main` from the `research/audio-lab` tip and writes that sha into
`repo.origin_sha` in `research/config.toml`. That sha is the original baseline for the final holdout check.

Files of an experiment (`research/experiments/<id>/`): `baseline_commit` and `prompt.md` (attempt.sh),
`hypothesis.md`, `trace.md`, optional `pipeline.toml` and optional `eval.toml` (the agent, see
[Model and Opus experiments](#model-and-opus-experiments)), `result.json` (the gate), and the gate logs
`build.log`, `test.log`, `eval.log` (git-ignored). Eval JSON lives in `research/results/gate/` (git-ignored).

## What the gate does

1. **Preflight.** The base commit must be an ancestor of the branch. Every changed path must be under
   `crates/vesper-dsp/src/` or the experiment's own folder, and under no protected prefix (`paths_policy` in
   `research/config.toml`). Removing tests or asserts, or touching `GOLDEN_HASH`, also fails. Any of these: `broken`.
2. **Build** `audiolab` from the candidate tree, and run `cargo test -p vesper-dsp --features tract`. A failure: `broken`.
3. **Configs.** The candidate runs `pipeline.toml` if the experiment has one, else the default. The default is
   `research/configs/default.toml` as it is at the base commit.
4. **Evaluate** the baseline (the base commit with the default config, cached) and the candidate on the `gate` split.
   Then run the candidate again on the first 5 items. Every score must match the full run bit for bit (JSON floats
   round-trip exactly), else `broken`.
5. **Paired SPRT** on the per-item OVRL deltas, candidate minus baseline: H0 gain <= 0, H1 gain >= 0.02 MOS,
   alpha = beta = 0.05. At each look the variance is re-estimated from all items so far, and
   LLR = (d1 / s^2) * sum(x_i - d1 / 2). Accept H1 at LLR >= ln(19), accept H0 at LLR <= -ln(19).
   Looks come every 20 items in manifest order, up to the whole split (400 items on v2).
   `result.json` also records `ovrl_delta_sd_full`, the spread of the per-item deltas over the whole split.
6. **Guards**, on the same items the test used: mean SIG delta >= -0.03; 10th percentile of the SIG deltas
   >= -0.15; mean BAK delta >= -0.05; mean SI-SDRi delta >= -0.3 dB; native RTF <= 0.06 and at most 1.25
   times the baseline's (the browser runs 5 to 8 times slower, by our WASM bench); extra latency
   at most the `Added latency (samples):` line in hypothesis.md and at most 240 samples (5 ms); no clipped item;
   and the second judge: mean `lsd_wb` delta at most +0.05 dB (`guards.lsd_wb_mean_max`). So a candidate is
   accepted only if DNSMOS OVRL passes the SPRT and the wideband distance to the clean speech does not grow.
   The guard fails closed when an eval has no `lsd_wb`. UTMOS (`second_mos`) is scored when
   `paths.second_mos` exists and reported in the deltas, but it does not decide.
7. **Record** `result.json` (status, reason, deltas with 95 percent intervals, guards, items used, LLR, shas)
   and regenerate the catalog.

The sequential test is a replay. The gate scores the whole split once (about 1.5 minutes per side), then applies the
batched stopping rule to the deltas in manifest order. The verdict and the item count are what a live sequential
run would give; only the compute is not saved.

## Statuses

| status | meaning |
|---|---|
| `accepted` | SPRT accepted H1 and every guard holds. Merging into `research/audio-main` is the controller's call. |
| `rejected` | SPRT accepted H0, or a guard failed (see `reason`), whatever the SPRT said. |
| `inconclusive` | Used the whole split without a verdict, and every guard holds. Never treated as accepted; a candidate for stacking. |
| `broken` | Protected path touched, build or test failed, nondeterministic output, or an eval failed. |
| `pending` | In the catalog only: an attempt with no result yet. |

## Stacking

Small gains can be stacked and gated together. Start a new attempt (for example `E10-stack`) and list the
experiments to combine in `research/config.toml`:

```toml
[stack]
E10-stack = ["E3-beta05", "E2-atten24"]
```

The gate then reads each listed experiment's `pipeline.toml` (from `research/experiments/<id>/` in the controller
checkout, else from its `exp/<id>` branch), then the stack's own
`pipeline.toml` if it has one, and passes them all to `audiolab eval --config A --config B ...`. They merge
key by key; a key set to two different values makes the attempt `broken`, so a `pipeline.toml` should list only
the keys it changes (one that writes out the whole default would clash with any stack partner). Code changes are not merged by the
stack option: cherry-pick those commits onto `exp/E10-stack`. Gate it like any other.
After an accepted change is merged into `research/audio-main`, update `research/configs/default.toml` there if the
change was config only, so later baselines include it. Shipping a config change also means changing
`PipelineConfig::default()` and its golden hash, which is controller work with Toby's review.

The final check runs everything accepted against `repo.origin_sha` on the `holdout` split, once.

## Opus stage

`audiolab eval --opus bitrate=64000,complexity=10,fec=1,dtx=0,loss=0.0,seed=1` runs the pipeline output
through libopus (the `opus` 0.4.0 crate, which builds its own libopus 1.5 with cmake, so the system
library does not matter), 20 ms packets at 48 kHz mono, and scores the DECODED audio with every metric.
Missing keys keep these defaults. More keys: `burst=3` (Gilbert-Elliott loss with a mean burst of 3
packets at the same long-run rate; without it, losses are independent) and `app=audio` (the default is
`voip`, which is what WebRTC uses for mono).

- Loss is seeded per item (`seed` and the item index), so two runs match bit for bit.
- A lost packet is decoded from the next packet's in-band FEC when FEC is on and that packet arrived (one
  packet of jitter buffer), else concealed (PLC). The encoder is told the loss rate
  (`OPUS_SET_PACKET_LOSS_PERC`), as WebRTC does; without it libopus sends no FEC.
- The output is shifted back by the encoder's lookahead (`OPUS_GET_LOOKAHEAD`), so
  it lines up with the clean reference. The RTF in the JSON is still the pipeline's alone.
- Verified in unit tests on a synthetic voice: at 128 kbps with no loss and `app=audio`, SI-SDR against
  the unencoded signal is above 20 dB and a one-sample shift scores lower (alignment). In `voip` mode
  libopus high-passes the input, and the phase shift holds SI-SDR near 10 dB at any bitrate, so SI-SDR
  after a VOIP round trip measures that filter more than the codec. At 24 kbps (SILK) and 10 percent
  loss, FEC beats concealment on SI-SDR and LSD. At 64 kbps FEC makes libopus switch from CELT to hybrid
  mode, and on the synthetic voice that scored lower than CELT with concealment. On the v2 dev split
  (real speech, research/BASELINE-v2.md) FEC beats concealment at 64 kbps as well, on every score.
- Two things the judges get wrong after Opus (measured on dev): a lossless VOIP round trip RAISES DNSMOS
  OVRL by about 0.12 over the unencoded output, so compare Opus runs with Opus runs only; and SI-SDR
  falls by 3.5 to 5.7 dB from the VOIP high-pass alone.

## Model and Opus experiments

An experiment folder may hold an `eval.toml`. Both keys are optional:

```toml
model = "research/models/DeepFilterNet3_ll_onnx.tar.gz"   # repo-relative, under research/models/
opus = "bitrate=48000,complexity=10,fec=1,dtx=1"
allow_more_bitrate = false
```

- `model` changes the candidate's model only. The baseline uses `[eval_baseline] model`, by default
  `paths.model` (the shipped DeepFilterNet3). Models are git-ignored; `research/scripts/fetch-model-ll.sh`
  fetches the low-latency DeepFilterNet3 with its sha256.
- `opus` puts BOTH sides through Opus and scores the decoded audio. The baseline uses `[eval_baseline] opus`:
  `bitrate=64000,complexity=10,fec=1,dtx=1,loss=0.05,seed=1`. Bitrate, DTX and FEC are what the web client
  sends (`web/src/lib/audio/settings.ts`, `signaling.ts`). The client does not set complexity, so 10 is the
  libopus default; what the browser's WebRTC stack uses is not verified. The 5 percent loss is our choice.
  Keys the candidate leaves out come from the baseline, not from audiolab's defaults. `loss`, `burst` and
  `seed` are the network, not a setting: the candidate may not change them, so both sides lose the same packets.
- Without `opus` neither side goes through Opus, so older experiments and their cached baselines are unchanged.
  The baseline cache key adds a hash of the baseline model file and Opus setting when either differs from
  the old default.
- Extra rules for an Opus experiment: the result carries `bitrate_bps` (baseline and candidate), and the
  `bitrate_bps` guard fails when the candidate uses more bitrate than the baseline. `allow_more_bitrate = true`
  lifts it only if hypothesis.md has a `Bitrate reason: ...` line. All other guards hold. SI-SDR after a VOIP
  round trip sits near 10 dB from the high-pass alone, so only the paired SI-SDRi delta is judged, as always.
  The RTF is the pipeline's alone; Opus cost is not in it.
- A bad `eval.toml` (unknown key, a model outside `research/models/`, a changed loss) makes the attempt `broken`.
  `gate.sh --dry-run <id>` shows the parsed options as the exact eval commands without running them.
- The catalog shows the candidate's `model` and full `opus` setting when `eval.toml` changes them.

## Self-tests

Two experiments are kept as proof that the gate cannot rubber-stamp:

- `E-null`: no code change, default config. It must come out `rejected` or `inconclusive` with a zero delta.
- `E-neg`: a deliberately bad config (`atten_db = 3`, `post_filter_beta = 0.3`). It must come out `rejected`.

Script tests: `python3 -m unittest discover -s research/scripts/tests` (SPRT, guards, eval.toml, catalog) and
`bash research/scripts/tests/test_paths.sh` (path and test-weakening checks).

## Machine limits

Eval cost on v2 (6 threads, capped): about 1 minute and 0.9 GiB for dev (160 items) and 2.5 to 3.5
minutes for gate (400 items) without UTMOS; with UTMOS about 5.5 minutes and 2.2 GiB for dev, about 14
minutes for gate (estimated from dev, not measured). UTMOS takes 23 s to load.

The dev box has about 15 GiB of RAM and often only 6 GiB free. Every build and eval runs through
`scripts/capped.sh` with `VESPER_CAP_MIB=5120` (`machine.memory_cap_mib`), with at most 6 threads. Run one heavy
job at a time: one gate, or one agent build, never two. The gate and the agents share `target/` in the controller
checkout. RTF is measured inside the capped scope (nice 19) on a shared machine, so it is noisy; the cached baseline
RTF may be from a quieter moment.

## Known limits

- The SPRT's reach depends on how much the per-item deltas spread, and that depends on the change, not
  only the corpus. Measured on the v2 gate split (400 items, default config as the base), and the
  minimum detectable effect (MDE: the smallest true OVRL gain the gate accepts in 80 percent of
  simulated runs, `research/scripts/mde.py`, 1000 runs per gain, normal deltas, the `[sprt]` settings):

  | candidate | sd of per-item OVRL deltas | MDE |
  |---|---|---|
  | `post_filter_beta = 0.05` (a small change) | 0.036 | 0.015 MOS |
  | `atten_db = 24` (a medium change) | 0.179 | 0.030 MOS |
  | sd 0.46 (default vs unprocessed on dev, a large change) | 0.46 | above 0.1 MOS |

  So small, consistent changes now resolve near 0.015 MOS, and changes that move items in both
  directions need 0.03 MOS or more. Round 1's "sd 0.6" was for a large change (E-neg); it is not the
  floor for small ones. The simulation assumes normal deltas; real deltas have heavier tails, so read
  the MDE as optimistic. The lsd_wb guard uses the same items as the SPRT, so an early stop means a
  noisier lsd_wb mean (its per-item sd is 4.4 dB for default vs unprocessed on dev).
- Dry run of the v2 rule on these two (`sprt.py gate` on the eval JSON, not a full `gate.sh` run):
  `post_filter_beta = 0.05` is rejected (H0 at 240 items, OVRL +0.009; lsd_wb +0.33 dB also fails its
  guard). `atten_db = 24` reaches H1 at 120 items (OVRL +0.055) and is rejected by the BAK guard (-0.14);
  its lsd_wb improves by 1.05 dB, which shows that lsd_wb also rewards leaving noise in. That is why the
  BAK guard stays.

- The gate measures the committed HEAD but runs `cargo test` in the live worktree.
- A model or Opus experiment changes the candidate only through `eval.toml`; a code change and an
  `eval.toml` in one attempt are judged together, so keep them apart.
- DNSMOS is checked by behaviour, not against Microsoft's onnxruntime numbers.
- DNSMOS and UTMOS both resample to 16 kHz: above 8 kHz only `lsd_wb`, `hb_err_db` and `mcd` judge.
- UTMOS is checked by behaviour (it ranks 0, 10 and 20 dB SNR and clean in order on dev items), not
  against the PyTorch model. It was trained on TTS and natural speech, not on denoiser output.
- The synthetic noises and rooms are models, not recordings: a real vacuum, cafe or door sounds different,
  and the synthetic RIRs have no discrete early reflections.
- 40 speakers in total: 10 in dev, 20 in gate. A speaker-specific effect can still look like a gain.
- `result.json` is trusted as written; only `gate.sh` should write it.
