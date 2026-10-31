# Audio research loop

A measured loop for the voice path. One hypothesis per experiment, each in its own git worktree. Agents make the
change; only the controller runs the gate. A statistical gate decides, and `research/CATALOG.md` records every
result, failures included. The design follows the Lemonate chess research loop, without the chess parts.

The corpus, the metrics and the `audiolab` CLI come first: see `research/scripts/fetch-corpus.sh`,
`research/scripts/fetch-dnsmos.sh` and `audiolab --help`. Build the corpus once with
`audiolab corpus build --out research/corpus/built --seed 1`.

## Honesty rules

- "Better" means a higher DNSMOS P.835 OVRL score, a learned quality model, plus guard metrics. It is a proxy.
  It can be gamed: over-suppression scores well on BAK. The guards are there for that, but they do not replace ears.
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

# 4. Read research/CATALOG.md and research/experiments/<id>/result.json.
```

The first `attempt.sh` creates `research/audio-main` from the `research/audio-lab` tip and writes that sha into
`repo.origin_sha` in `research/config.toml`. That sha is the original baseline for the final holdout check.

Files of an experiment (`research/experiments/<id>/`): `baseline_commit` and `prompt.md` (attempt.sh),
`hypothesis.md`, `trace.md` and optional `pipeline.toml` (the agent), `result.json` (the gate), and the gate logs
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
   Looks come every 20 items in manifest order, up to the split size (240).
6. **Guards**, on the same items the test used: mean SIG delta >= -0.03; 10th percentile of the SIG deltas
   >= -0.15; mean SI-SDRi delta >= -0.3 dB; RTF <= 0.30 and at most 1.25 times the baseline's; extra latency
   at most the `Added latency (samples):` line in hypothesis.md and at most 240 samples (5 ms); no clipped item.
7. **Record** `result.json` (status, reason, deltas with 95 percent intervals, guards, items used, LLR, shas)
   and regenerate the catalog.

The sequential test is a replay. The gate scores the whole split once (about 1.5 minutes per side), then applies the
batched stopping rule to the deltas in manifest order. The verdict and the item count are what a live sequential
run would give; only the compute is not saved.

## Statuses

| status | meaning |
|---|---|
| `accepted` | SPRT accepted H1 and every guard holds. Merging into `research/audio-main` is the controller's call. |
| `rejected` | SPRT accepted H0, or it accepted H1 but a guard failed (see `reason`). |
| `inconclusive` | Hit the 240-item cap without a verdict. Never treated as accepted; a candidate for stacking. |
| `broken` | Protected path touched, build or test failed, nondeterministic output, or an eval failed. |
| `pending` | In the catalog only: an attempt with no result yet. |

## Stacking

Small gains can be stacked and gated together. Start a new attempt (for example `E10-stack`), cherry-pick the
experiment commits into its worktree, and combine their `pipeline.toml` settings into its own. Gate it like any other.
After an accepted change is merged into `research/audio-main`, update `research/configs/default.toml` there if the
change was config only, so later baselines include it. Shipping a config change also means changing
`PipelineConfig::default()` and its golden hash, which is controller work with Toby's review.

The final check runs everything accepted against `repo.origin_sha` on the `holdout` split, once.

## Self-tests

Two experiments are kept as proof that the gate cannot rubber-stamp:

- `E-null`: no code change, default config. It must come out `rejected` or `inconclusive` with a zero delta.
- `E-neg`: a deliberately bad config (`atten_db = 3`, `post_filter_beta = 0.3`). It must come out `rejected`.

Script tests: `python3 -m unittest discover -s research/scripts/tests` (SPRT, guards, catalog) and
`bash research/scripts/tests/test_paths.sh` (path and test-weakening checks).

## Machine limits

The dev box has about 15 GiB of RAM and often only 6 GiB free. Every build and eval runs through
`scripts/capped.sh` with `VESPER_CAP_MIB=5120` (`machine.memory_cap_mib`), with at most 6 threads. Run one heavy
job at a time: one gate, or one agent build, never two. The gate and the agents share `target/` in the controller
checkout. RTF is measured inside the capped scope (nice 19) on a shared machine, so it is noisy; the cached baseline
RTF may be from a quieter moment.

## Known limits

- The gate measures the committed HEAD but runs `cargo test` in the live worktree.
- The gate uses one DeepFilterNet model for both sides, so a model-choice experiment (E8) needs a gate change first.
- DNSMOS is checked by behaviour, not against Microsoft's onnxruntime numbers.
- The speech is 16 kHz LibriSpeech upsampled to 48 kHz, so nothing above 8 kHz is speech.
- `result.json` is trusted as written; only `gate.sh` should write it.
