# Trace

## Summary
Self-test written by the framework author: a bad config (atten_db 3, post_filter_beta 0.3), no code.

## Tried
1. Gate run 1: SPRT inconclusive at 240 items (LLR -1.18), OVRL -0.077, SIG p10 guard failed. Status was then
   `inconclusive`, because a guard failure only rejected after an H1 verdict.
2. Changed the gate rule: a failed guard rejects whatever the SPRT says. Gate run 2: `rejected`.

## Dropped
Making the config worse until the SPRT alone says H0. The point is that the gate rejects it, and it does.

## Surprises
The SPRT did not reach H0 for a clear loss. Per-item OVRL deltas have a standard deviation of about 0.6 MOS,
so with H1 at +0.02 the test needs several hundred items for effects this size. SIG went UP by 0.26 on average
(less suppression keeps more speech), while BAK fell by 0.94: DNSMOS rewards less processing on SIG.
