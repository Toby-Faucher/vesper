# A causal 2nd-order Butterworth high-pass before DeepFilterNet removes sub-speech rumble and thumps

## Change
New `PreConfig.highpass: Option<HighpassConfig>` (`[pre.highpass]`, field `cutoff_hz`), off by default. A per-instance biquad (Butterworth, Q=1/sqrt2, f64 coefficients, transposed direct form II) runs after input gain and before DeepFilterNet. Cutoffs 60, 80, 120 Hz tried on dev only.

## Mechanism
Fan rumble and keyboard thumps put energy below ~100 Hz, where speech has little. The model sees less low-frequency noise it must model, and DNSMOS BAK may rise without touching speech bands.

## Expected effect
Small BAK/OVRL gain on fan and keyboard items; SIG and SI-SDRi roughly flat. Higher cutoffs gain more noise removal but start to thin male voices (F0 ~85-120 Hz).

## Risk
120 Hz may lower SIG or SI-SDRi by thinning low voices. Biquad phase shift near cutoff is small for speech. Guard: BAK and SIG must not drop.

Added latency (samples): 0
