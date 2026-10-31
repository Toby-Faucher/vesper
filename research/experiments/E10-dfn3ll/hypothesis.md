# E10-dfn3ll

## Change
eval.toml only (default pipeline).

## Mechanism
Low-latency DeepFilterNet3 cuts model delay from 1440 to 480 samples. Latency data point: it will fail the native RTF ceiling (3.4x slower), expected. Hypothesis: quality is about equal.

## Expected effect
See the hypothesis line.

## Risk
See the gate guards.

Added latency (samples): 0
