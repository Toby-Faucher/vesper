# A slow, speech-gated input leveler before DeepFilterNet lifts quiet mics without clipping loud ones

## Change
A new pre stage `PreConfig::leveler` (`[pre.leveler]` in pipeline.toml), off by default (`None`).
Per 480-sample frame it measures the input RMS, computes the gain that would bring the frame to a
target (-23 dBFS), clamps it to [0, +18] dB, and moves the current gain toward it in dB: slowly when
rising, faster when falling. The gain only moves while the frame is above a -55 dBFS floor and the
model's local SNR for the previous frame says speech is present; otherwise it holds. The gain is
ramped linearly across each frame, and capped per frame so the frame peak stays below 0.98.
No lookahead, no allocation.

## Mechanism
E1 showed that DeepFilterNet works better on louder input (SI-SDRi rose) but a static +6/+12 dB
clipped and degraded the -15 dBFS items. A leveler gives the -35 dBFS items roughly +10 dB and leaves
the loud items close to unity, so the model sees speech at the level it was trained on without
clipping. Gating on level and on the model's local SNR keeps the gain from rising on noise only
(the lead-in), so the noise floor is not lifted before speech starts.

## Expected effect
-35 dBFS items: OVRL +0.05 to +0.15, SIG up, SI-SDRi up by about 0.5 to 1 dB, mostly at 5 to 20 dB SNR.
-25 dBFS items: small gain. -15 dBFS items: about zero (gain stays near 0 dB).

## Risk
Pumping inside speech (gain moves with syllables), noise lifted in pauses at low SNR where the local SNR
gate opens on noise, a level step when the peak cap fires, and DNSMOS reacting to output loudness
rather than quality. BAK on the -35 dBFS items could drop if the residual noise is now louder.

Added latency (samples): 0
