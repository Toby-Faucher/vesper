# Bad network call (tc netem)

A real two-browser call on one machine with loss, burst loss, jitter or delay on the media path. Use it to see what a weak
link does to the call and to compare a change before and after (for example RED, if it is ever built). The audio lab models
the same losses offline (`audiolab eval --opus ...,loss=0.05,burst=3`, see [research-loop.md](../research-loop.md)).

**Status: not verified.** The script's printed commands are checked by `scripts/test-netem-call.sh`. They have not been run
as root on this machine, and no call has been measured with them.

## What it shapes

`scripts/netem-call.sh` puts netem on UDP packets going **into** the SFU (destination port 40000, the `VESPER_UDP_BIND`
default; `--port` for another). Everything else on the device is untouched, so SSH and the WebSocket keep working. Each
sender's audio is hit once on its way in, so the loss a listener sees is about the configured loss. RTCP from the browsers
to the SFU is hit too; the SFU's packets back out are not.

| Profile | netem arguments | Meaning |
|---|---|---|
| `loss5`, `loss10` | `loss 5%` | independent loss |
| `burst5` | `loss gemodel 1.754% 33.333% 100% 0%` | 5 % loss in bursts of 3 packets on average (60 ms) |
| `burst10` | `loss gemodel 2.222% 20.000% 100% 0%` | 10 % loss in bursts of 5 packets on average (100 ms) |
| `jitter` | `delay 40ms 20ms` | 40 ms delay, varying by 20 ms; packets get reordered |
| `delay` | `delay 150ms` | fixed 150 ms one way |
| `wifi` | `delay 30ms 15ms` plus the `burst5` loss | a poor Wi-Fi link |
| `off` | | removes it |

The burst profiles use the same Gilbert-Elliott model as the audio lab: a packet in the bad state is always lost, one in
the good state never, the mean burst is `1/r` packets and the long-run loss is `p/(p+r)`.

## Run it

Root is needed for `tc`. Agents never run it without Toby: `--apply` goes through `sudoq`, which waits for his approval.

```sh
scripts/netem-call.sh burst5            # prints the three tc commands, changes nothing
scripts/netem-call.sh burst5 --apply    # runs them through: sudoq run --sh --why "..." -- '<the commands>'
tc -s qdisc show dev lo                 # the netem line counts dropped packets
scripts/netem-call.sh off --apply       # always remove it afterwards
```

To switch profiles, run `off --apply` first: `add` fails when the qdisc is already there. `--dev` takes another device,
for example one end of a veth pair between two network namespaces; that setup is not described or tried here. Do not
use this on a network interface you reach the machine through (on bawn, `bond0`).

## The call

1. Start the server and the web app as in [dev.md](../dev.md#run) and keep the UDP port at 40000.
2. Join one voice room from two browsers: a normal and a private window, or Chrome and Firefox, signed in as two accounts.
   Let one play speech (a podcast near the mic, or read a text) and listen on the other with headphones.
3. Note the numbers with no netem for 30 s, then apply a profile, wait 30 s and note them again. Then `off`.

## Read the numbers

- **Vesper, Settings > Diagnostics** (the call bar's "Call details"): `Loss (incoming)` is packets lost over packets
  expected, from the inbound streams' `packetsLost` and `packetsReceived` (`web/src/lib/diag.ts`). `Jitter` is the
  largest inbound `jitter`. `RTT` is the ICE pair's round trip. With `burst5` expect incoming loss near 5 % on the
  listener and about 0 % with netem off.
- **Vesper, voice list:** hover the connection icon next to the speaker's name, or open their person card. "N % of sound
  filled in" is concealed samples minus silent concealed samples over samples received, per poll window
  (`web/src/lib/ui/link-stats.ts`). This is the number to compare: it counts what the listener's jitter buffer had to make
  up, after FEC.
- **Chrome, `chrome://webrtc-internals`:** pick the listener's peer connection, then the `inbound-rtp` audio stats:
  `packetsLost`, `concealedSamples`, `silentConcealedSamples`, `concealmentEvents`, `totalSamplesReceived`, `jitter`,
  and `jitterBufferDelay` divided by `jitterBufferEmittedCount` for the average buffer delay. FEC recoveries show as
  `fecPacketsReceived`. Graphs are on the same page.
- **Firefox, `about:webrtc`:** the same inbound RTP numbers under the connection's RTP stats.

Write down the profile, the browser pair and those numbers. The test proves nothing about sound quality on its own:
listen too, and for a decision use a blind A/B ([checkpoint.md](checkpoint.md#4-blind-ab-protocol)).
