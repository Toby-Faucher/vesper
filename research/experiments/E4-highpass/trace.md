## Summary
Added an optional causal 2nd-order Butterworth high-pass (`[pre.highpass] cutoff_hz`) before DeepFilterNet to remove rumble; on dev it lowers every metric at all cutoffs, so the guard (BAK, SIG must not drop) fails.

## Tried
Dev split, 120 items, paired against default (OVRL 2.839, SIG 3.146, BAK 3.926, SI-SDRi 3.68):
- 60 Hz: OVRL 2.792 (-0.047), SIG 3.098 (-0.047), BAK 3.893 (-0.033), SI-SDRi 0.43 (-3.26); CIs exclude 0.
- 80 Hz: OVRL 2.775, SIG 3.081, BAK 3.891, SI-SDRi -1.04.
- 120 Hz: OVRL 2.750, SIG 3.058, BAK 3.878, SI-SDRi -3.54.
60 Hz is the least bad and is in pipeline.toml, but it is a regression, not a candidate to promote.

## Dropped
Higher cutoffs (worse monotonically). No other filter orders tried (scope was a 2nd-order biquad).

## Surprises
- SI-SDRi falls by >3 dB even at 60 Hz, where speech has almost no energy. Likely the biquad phase shift/low-frequency removal moves the output away from the clean reference, which SI-SDR penalises; lead-in reduction jumps from 39 to 73 dB, so the corpus noise has a lot of sub-60 Hz energy (or DC) that SI-SDR was counting.
- The 30 Hz >20 dB unit test only holds at 120 Hz: a 2nd-order Butterworth at 80 Hz gives about -17 dB at 30 Hz. The test uses 120 Hz.
