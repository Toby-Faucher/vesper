# Self-test: a deliberately bad config (the gate must reject it)

## Change
pipeline.toml only: atten_db 3 (almost no noise removal) and post_filter_beta 0.3 (a heavy post filter).

## Mechanism
With only 3 dB of attenuation most of the noise stays, so BAK and OVRL fall far below the baseline.

## Expected effect
SPRT accepts H0 quickly. Status `rejected`.

## Risk
If this is ever `accepted`, the gate is broken.

Added latency (samples): 0
