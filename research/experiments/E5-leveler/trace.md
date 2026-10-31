## Summary
Added an optional speech-gated input leveler before DeepFilterNet (`[pre.leveler]`). The idea was to lift quiet mics toward -23 dBFS so the model gets a stronger input without clipping loud ones. On dev it is close to neutral on DNSMOS and loses SI-SDR.

## Tried
All runs on the dev split, 120 items, paired against the default config (OVRL 2.839, SI-SDRi 3.68 dB). Deltas are b - a.

1. v1: per-frame target gain, smoothed in dB (rise tau 1 s, fall tau 0.3 s), lsnr gate 0 dB, floor -55 dBFS.
   Overall: OVRL +0.010, SIG +0.011, BAK -0.055, SI-SDRi -2.01. At -35 dBFS: OVRL +0.046, SIG +0.047, BAK -0.112, SI-SDRi -2.79.
   -15 dBFS items also got gain (soft frames and pauses sit under -23 dBFS), and their SI-SDRi fell by 1.6.
2. Reference, static `input_gain_db = 12` (the config E1 rejected): -35 dBFS gave OVRL +0.016, BAK -0.150, SI-SDRi +2.26. -25 and -15 clip (OVRL -0.16).
   So a constant gain on quiet items is worth very little DNSMOS on this corpus. Its SI-SDR gain comes from the gain being constant.
3. v2, the committed candidate: gain from a power-domain speech level estimate (rise_s 2, fall_s 0.5). The estimate starts at the target, so the gain starts at 0 dB.
   Overall: OVRL +0.008 [-0.002, +0.020], SIG +0.005, BAK -0.011, SI-SDRi -0.99 [-1.51, -0.53].
   -35: OVRL +0.026, SIG +0.017, BAK -0.029, SI-SDRi -2.87. -25 and -15: within ±0.003 on DNSMOS and within 0.1 dB on SI-SDRi. No clipped items.
4. rise_s 0.5, fall_s 0.5: overall OVRL +0.013, SI-SDRi -1.81. -35: OVRL +0.037, BAK -0.106, SI-SDRi -3.35.
5. rise_s 0.5, fall_s 2.0: overall OVRL +0.004, SI-SDRi -2.44. Worse everywhere.
6. rise_s 4, fall_s 1: overall OVRL +0.006, SI-SDRi -0.64. -35: OVRL +0.019, BAK -0.011, SI-SDRi -1.93.

The time constants trade OVRL on the -35 dBFS items against SI-SDR. No setting wins both.

## Dropped
- Letting the leveler attenuate loud items (min gain below 0 dB). Loud items do not need help, and attenuating them only adds more time-varying gain.
- Undoing the leveler gain after the model (delayed by the 1440-sample model latency). That would keep SI-SDR fair and test only whether the model works better on a leveled input. It does not lift quiet output, which was the goal, and run 2 shows the possible DNSMOS gain is about +0.02. Worth trying only to answer that model question.
- More tuning: every run moves along the same OVRL against SI-SDR curve.

## Surprises
- SI-SDR against the unleveled clean reference penalises any gain that changes over time. The 10 s items count the leveler's convergence ramp (several seconds at -35 dBFS) as distortion, so SI-SDRi drops most at 20 dB SNR (-3 dB in v2), where the model itself changes little.
- DNSMOS hardly cares about input level here. A static +12 dB on the -35 dBFS items gives +0.016 OVRL. The premise that the model gets "too weak" input at -35 dBFS mixtures does not show up in DNSMOS.
- Gating the gain on the model's previous-frame local SNR worked as a speech detector at no cost: lead-in reduction fell by only 0.2 dB in v2.
