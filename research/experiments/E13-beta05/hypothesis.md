# post filter 0.05

## Change
pipeline.toml only: post_filter_beta = 0.05

## Mechanism
Sweep point; re-test on corpus v2 (real 48 kHz voices).

## Expected effect
Small (round 1 on corpus v1: inconclusive or rejected).

## Risk
Less suppression leaves more noise; the BAK guard watches that.

Added latency (samples): 0
