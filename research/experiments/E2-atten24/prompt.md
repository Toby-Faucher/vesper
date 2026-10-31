You are attempting one hypothesis to make the Vesper voice path sound better.

- Experiment: E2-atten24
- Work only in this git worktree: /home/toby/Projects/vesper-wt/lab/.worktrees/E2-atten24 (branch `exp/E2-atten24`).
- Your experiment folder: /home/toby/Projects/vesper-wt/lab/research/experiments/E2-atten24 (in the controller checkout, not in your worktree).
- Read first: /home/toby/Projects/vesper-wt/lab/CLAUDE.md, /home/toby/Projects/vesper-wt/lab/research/backlog.md (your line), /home/toby/Projects/vesper-wt/lab/research/CATALOG.md (results so far:
  do not retry a rejected idea), and `crates/vesper-dsp/src/pipeline.rs` (the `Pipeline` and `PipelineConfig`).

## Order of work

1. Fill in /home/toby/Projects/vesper-wt/lab/research/experiments/E2-atten24/hypothesis.md BEFORE you write any code: Change, Mechanism, Expected effect, Risk.
   If your change adds algorithmic delay, set the `Added latency (samples):` line (at most 240). Undeclared delay fails the gate.
2. Make the change.
   - Code: edit files under `crates/vesper-dsp/src/` only. New behaviour goes behind a `PipelineConfig` field
     that is off by default. The default config must stay bit exact: the `GOLDEN_HASH` tests must pass unchanged.
   - Config: put the settings that turn your change on in /home/toby/Projects/vesper-wt/lab/research/experiments/E2-atten24/pipeline.toml (same format as
     /home/toby/Projects/vesper-wt/lab/research/configs/default.toml). A config-only experiment has a pipeline.toml and no code change.
     Without a pipeline.toml the gate runs the default config.
   - Stages must be causal, per frame (480 samples at 48 kHz), allocation free in the hot path, and use no `unsafe`.
3. Commit your code on branch `exp/E2-atten24`. The gate measures the committed HEAD and refuses uncommitted changes.
4. Write /home/toby/Projects/vesper-wt/lab/research/experiments/E2-atten24/trace.md with these sections:
   - `## Summary`: one line: what you changed and why you expect it to help
   - `## Tried`: what you tried, in order, with the dev numbers you saw
   - `## Dropped`: ideas you discarded and why
   - `## Surprises`: anything unexpected in the code or the audio

## What you may run

Every build, test and eval goes through the memory cap. One at a time. At most 6 threads.
Share the controller's target folder so you do not rebuild the world:

```bash
cd /home/toby/Projects/vesper-wt/lab/.worktrees/E2-atten24
export CARGO_TARGET_DIR=/home/toby/Projects/vesper-wt/lab/target
VESPER_CAP_MIB=5120 scripts/capped.sh cargo test --locked --release -p vesper-dsp --features tract
VESPER_CAP_MIB=5120 scripts/capped.sh cargo build --locked --release -p vesper-audiolab
VESPER_CAP_MIB=5120 scripts/capped.sh /home/toby/Projects/vesper-wt/lab/target/release/audiolab eval --split dev --jobs 6 \
  --config /home/toby/Projects/vesper-wt/lab/research/experiments/E2-atten24/pipeline.toml --out /tmp/E2-atten24-dev.json \
  --corpus /home/toby/Projects/vesper-wt/lab/research/corpus/built --dnsmos /home/toby/Projects/vesper-wt/lab/research/models/sig_bak_ovr.onnx
VESPER_CAP_MIB=5120 scripts/capped.sh /home/toby/Projects/vesper-wt/lab/target/release/audiolab compare --a A.json --b B.json
```

The dev split is yours. Score the default config the same way (`--config /home/toby/Projects/vesper-wt/lab/research/configs/default.toml`) and
compare paired. Keep a dev run under 2 minutes; use `--limit` while you iterate.

## Rules

1. Never edit outside `crates/vesper-dsp/src/` and /home/toby/Projects/vesper-wt/lab/research/experiments/E2-atten24. Changes to `research/`, `crates/vesper-audiolab/`,
   `Cargo.toml`, `Cargo.lock`, `rust-toolchain*`, `deny.toml`, `.github/`, `web/`, `docs/` or `scripts/` make the attempt `broken`.
   Do not remove tests or asserts.
2. Never run `research/scripts/gate.sh`, never use `--split gate` or `--split holdout`, and never read files under
   `/home/toby/Projects/vesper-wt/lab/research/results/gate/` or another experiment's `result.json`. The gate is run by someone else, so you cannot tune against it.
3. In /home/toby/Projects/vesper-wt/lab/research/experiments/E2-atten24 write only `hypothesis.md`, `trace.md` and `pipeline.toml`. Never write `result.json` or `baseline_commit`.
4. Do not rebase, merge or cherry-pick other branches. Do not push. Do not start subagents. Never run `sudo`.
5. DNSMOS is a learned proxy and can be gamed (over-suppression scores well on BAK). Aim for speech that sounds better,
   not for a number. Report what you measured, including losses.
