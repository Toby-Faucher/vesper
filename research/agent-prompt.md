You are attempting one hypothesis to make the Vesper voice path sound better.

- Experiment: {{ID}}
- Work only in this git worktree: {{WORKTREE}} (branch `exp/{{ID}}`).
- Your experiment folder: {{EXPERIMENT}} (in the controller checkout, not in your worktree).
- Read first: {{REPO}}/CLAUDE.md, {{RESEARCH}}/backlog.md (your line), {{RESEARCH}}/CATALOG.md (results so far:
  do not retry a rejected idea), and `crates/vesper-dsp/src/pipeline.rs` (the `Pipeline` and `PipelineConfig`).

## Order of work

1. Fill in {{EXPERIMENT}}/hypothesis.md BEFORE you write any code: Change, Mechanism, Expected effect, Risk.
   If your change adds algorithmic delay, set the `Added latency (samples):` line (at most 240). Undeclared delay fails the gate.
2. Make the change.
   - Code: edit files under `crates/vesper-dsp/src/` only. New behaviour goes behind a `PipelineConfig` field
     that is off by default. The default config must stay bit exact: the `GOLDEN_HASH` tests must pass unchanged.
   - Config: put the settings that turn your change on in {{EXPERIMENT}}/pipeline.toml (same format as
     {{RESEARCH}}/configs/default.toml). A config-only experiment has a pipeline.toml and no code change.
     Without a pipeline.toml the gate runs the default config.
   - Model or Opus: put `model = "research/models/<file>"` or `opus = "bitrate=..."` in {{EXPERIMENT}}/eval.toml
     (see docs/research-loop.md, "Model and Opus experiments"). An Opus change may not raise the bitrate unless
     eval.toml sets `allow_more_bitrate = true` and hypothesis.md has a `Bitrate reason:` line.
   - Stages must be causal, per frame (480 samples at 48 kHz), allocation free in the hot path, and use no `unsafe`.
3. Commit your code on branch `exp/{{ID}}`. The gate measures the committed HEAD and refuses uncommitted changes.
4. Write {{EXPERIMENT}}/trace.md with these sections:
   - `## Summary`: one line: what you changed and why you expect it to help
   - `## Tried`: what you tried, in order, with the dev numbers you saw
   - `## Dropped`: ideas you discarded and why
   - `## Surprises`: anything unexpected in the code or the audio

## What you may run

Every build, test and eval goes through the memory cap. One at a time. At most {{THREADS}} threads.
Share the controller's target folder so you do not rebuild the world:

```bash
cd {{WORKTREE}}
export CARGO_TARGET_DIR={{REPO}}/target
VESPER_CAP_MIB={{CAP}} scripts/capped.sh cargo test --locked --release -p vesper-dsp --features tract
VESPER_CAP_MIB={{CAP}} scripts/capped.sh cargo build --locked --release -p vesper-audiolab
VESPER_CAP_MIB={{CAP}} scripts/capped.sh {{REPO}}/target/release/audiolab eval --split dev --jobs {{THREADS}} \
  --config {{EXPERIMENT}}/pipeline.toml --out /tmp/{{ID}}-dev.json \
  --corpus {{REPO}}/research/corpus/built --dnsmos {{REPO}}/research/models/sig_bak_ovr.onnx
VESPER_CAP_MIB={{CAP}} scripts/capped.sh {{REPO}}/target/release/audiolab compare --a A.json --b B.json
```

The dev split is yours. Score the default config the same way (`--config {{RESEARCH}}/configs/default.toml`) and
compare paired. Keep a dev run under 2 minutes; use `--limit` while you iterate.

## Rules

1. Never edit outside `crates/vesper-dsp/src/` and {{EXPERIMENT}}. Changes to `research/`, `crates/vesper-audiolab/`,
   `Cargo.toml`, `Cargo.lock`, `rust-toolchain*`, `deny.toml`, `.github/`, `web/`, `docs/` or `scripts/` make the attempt `broken`.
   Do not remove tests or asserts.
2. Never run `research/scripts/gate.sh`, never use `--split gate` or `--split holdout`, and never read files under
   `{{REPO}}/research/results/gate/` or another experiment's `result.json`. The gate is run by someone else, so you cannot tune against it.
3. In {{EXPERIMENT}} write only `hypothesis.md`, `trace.md`, `pipeline.toml` and `eval.toml`. Never write `result.json` or `baseline_commit`.
4. Do not rebase, merge or cherry-pick other branches. Do not push. Do not start subagents. Never run `sudo`.
5. DNSMOS is a learned proxy and can be gamed (over-suppression scores well on BAK). Aim for speech that sounds better,
   not for a number. Report what you measured, including losses.
