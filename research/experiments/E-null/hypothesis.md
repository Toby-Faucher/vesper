# Self-test: no change at all (the gate must not accept a no-op)

## Change
None. Same code as the base, default config, no pipeline.toml.

## Mechanism
None. Every paired delta should be exactly zero.

## Expected effect
SPRT accepts H0 at the first look (20 items). Status `rejected`, deltas 0, determinism check passes.

## Risk
If this is ever `accepted`, the gate is broken.

Added latency (samples): 0
