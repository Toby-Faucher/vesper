## Summary
Post gate on the model's lsnr: after 150 ms under -3 dB the gain falls at 300 dB/s to -24 dB, and returns to 0 dB within one frame at lsnr >= 0 dB. Expected to cut residual noise in pauses.

## Tried
All on dev, first 120 items, paired against the default (close/open/hold/floor/release):
- -5/0/100 ms/-24/200: OVRL +0.031, BAK +0.034, SIG +0.016, SI-SDRi -0.025, LSD +0.106, lead-in +3.2 dB.
- 0/5/100/-18/200: OVRL +0.019, BAK +0.047, SIG +0.001, SI-SDRi -0.46, lead-in +3.7 dB. Clips speech, rejected.
- -10/-5/100/-30/200: OVRL +0.015, BAK +0.016, SI-SDRi 0.00, lead-in +1.7 dB. Too timid.
- -3/0/150/-24/300 (chosen, pipeline.toml): OVRL +0.035, BAK +0.043, SIG +0.016, SI-SDRi -0.018 (CI spans 0), LSD +0.096, lead-in +3.1 dB.
Per noise type (OVRL / lead-in dB): fan +0.03 / +9.9, ac_hiss +0.06 / +6.1, freesound2530 +0.07 / +4.9, keyboard +0.01 / +1.7, babble +0.01 / +1.9, street, tv, freesound573577 about 0 / 0.

## Dropped
Release shape tuning, a lookahead on lsnr (the output is about 30 ms behind lsnr; not needed so far).

## Surprises
The default lead-in reduction is already 39 dB, so the headroom is small. Keyboard barely improves: bursts are short and the hold keeps the gate open. LSD gets worse (+0.1) because the floor is lower than the reference noise floor in pauses.

## Corpus v2 re-check (dev, 160 items, default vs pipeline.toml, same binary)
OVRL +0.029 [+0.018,+0.040], BAK +0.029, SIG +0.024, lsd_wb +0.138 dB (worse), SI-SDRi -0.046, lead-in +3.2 dB, rtf 0.053 to 0.056.
Per noise (OVRL): ac_hiss +0.145, freesound2530 +0.045, fan +0.035, street +0.035, vacuum +0.032, freesound573577 +0.031, keyboard +0.029; cafe -0.005, others about 0.
The v1 numbers hold on v2, so the constants were not re-tuned.
