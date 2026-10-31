# Trace

## Summary
Self-test written by the framework author: no code change and the default config.

## Tried
Gate run: all 240 paired deltas are exactly zero, the SPRT accepted H0 at the first look (20 items), the
determinism check matched bit for bit. Status `rejected`.

## Dropped
Nothing.

## Surprises
Zero variance makes the LLR huge and negative (the variance is floored at 1e-12), so H0 comes at once.
RTF was 0.043 for both sides, measured minutes apart (ratio 0.95): the RTF guard has about 5 percent noise.
