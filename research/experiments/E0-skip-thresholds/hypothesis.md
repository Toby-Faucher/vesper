# Disable DeepFilterNet's high-SNR stage skipping

## Change
pipeline.toml only: max_db_erb_thresh and max_db_df_thresh set to 1000 so the ERB and DF stages never switch off.

## Mechanism
At high local SNR the model skips its stages, which leaves near-clean speech badly processed (20 dB items reach only about 5.5 dB SI-SDR).

## Expected effect
Gains on 10 and 20 dB items. Scratch run on 12 items: OVRL +0.36, SIG +0.25, SI-SDRi +9.9 dB.

## Risk
More processing on clean speech may add artefacts.

Added latency (samples): 0
