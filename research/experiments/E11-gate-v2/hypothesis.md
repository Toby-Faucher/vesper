# A post gate driven by the model's local SNR attenuates residual noise between words

## Change
New `PostConfig.gate: Option<GateConfig>` (`[post.gate]`), off by default. After DeepFilterNet, a per-frame gain follows `last_lsnr`: when lsnr stays under `close_db` for `hold_ms`, the gain ramps down to `floor_db` at `release_db_per_s`; when lsnr reaches `open_db` (above `close_db`, hysteresis) the gain returns to 0 dB within one frame. The gain is ramped linearly across each 480-sample frame. No allocation, no unsafe.

## Mechanism
Fan hum and keyboard leftovers survive the model at low level in pauses. The model already says the frame is noise (low lsnr), so scaling those frames down removes what the model leaves behind. Fast attack and the hold time protect word onsets and word tails.

## Expected effect
Lead-in residual reduction and BAK rise on fan and keyboard items. SIG, SI-SDRi and LSD roughly flat.

## Risk
Lsnr is for the newest frame, the output is about 30 ms older, so onsets may be clipped if lsnr lags. Quiet consonants and breaths have low lsnr and can be gated, which lowers SIG. A deep floor can sound pumpy; BAK can be gamed by it.

Added latency (samples): 0
