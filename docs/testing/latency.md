# Latency and load testing

## 1. Load test (packet loss and server CPU)

`vesper-loadtest` joins N synthetic speakers to one room over the real WebSocket protocol, each sends 20 ms Opus-sized packets for S seconds, and every speaker counts what it received from each other speaker. It exits 1 if any speaker received under 95% of its peers' packets (2 on setup errors). `--pid` adds the server's CPU over the run; that reads `/proc/<pid>/stat`, so it is Linux only (and assumes 100 clock ticks per second).

Against a local server on loopback with a throwaway data dir (ports are arbitrary high ones). Each speaker signs in with one
session token from `loadtest-accounts` and sends `Origin` on the upgrade. Room 1 is `general` on a fresh database.

```sh
cargo build --release -p vesper-server -p vesper-loadtest

DATA=$(mktemp -d)
VESPER_DATA_DIR=$DATA target/release/vesper-server loadtest-accounts --count 12 > $DATA/tokens
VESPER_DATA_DIR=$DATA VESPER_ORIGIN=http://127.0.0.1:47123 VESPER_HTTP_BIND=127.0.0.1:47123 \
  VESPER_UDP_BIND=127.0.0.1:47124 target/release/vesper-server &
SERVER=$!

target/release/vesper-loadtest --url ws://127.0.0.1:47123/ws --origin http://127.0.0.1:47123 \
  --tokens $DATA/tokens --room-id 1 --speakers 12 --seconds 30 --pid $SERVER
echo "exit=$?"

kill $SERVER; rm -rf $DATA
```

Options: `--speakers` and `--seconds` must be at least 1; one speaker is a trivial pass (no peers). Pass requires every speaker to get 95% of its peers' packets in aggregate AND from each individual peer (a dead peer fails at any room size). Each speaker sends about 48 packets/s in practice (not exactly 50), and the tool warns if one sent over 5% fewer than seconds*50. `--pid` is checked at start.

Recorded result (debug build, 12 speakers x 30 s on loopback): loss 0.00% for every speaker, server CPU 54.9% of one core. A debug build is pessimistic; use `--release` as above for real numbers.
With sessions (release build, 12 speakers on loopback, 2026-11-01, the commands above): 10 s run loss 0.00%, server CPU 16.7% of one core; 30 s run loss 0.00% for every speaker, server CPU 17.5% of one core. Both PASS.

The server's `no advertised addresses` warning is expected on loopback; the fake clients use host candidates. Run the load test on the same machine as the server: client and server CPU are then both on that box, so read the CPU figure as an upper bound.

## Video load

Voice and video share one SFU thread, so a screen share must never make voice break up. `vesper-loadtest` can add sharers: `--sharers N` (default 0) makes the first N speakers share a screen, and `--share-mbps M` (default 10, 1 to 30) sets the rate. Every speaker watches every sharer. Sharers send 60 frames a second of `M * 1_000_000 / 8 / 60` bytes each. The readiness gate counts audio tracks only. PASS is still the audio rule (95% in aggregate and from each peer); video delivery is printed per viewer and sharer and does not decide PASS. The "video" figure is frames received out of frames queued (`seconds * 60`); the fake sender paces at about 57 frames a second in practice, so a pair that lost nothing still shows about 93 to 95%.

Run it as in section 1, with the server and the load test pinned to separate cores (`taskset -c SERVERCPUS target/release/vesper-server`, `taskset -c 4-11 target/release/vesper-loadtest ...`) and extra flags, for example `--speakers 4 --sharers 2 --share-mbps 10 --seconds 30 --pid $SERVER`.

Measured 2026-11-10, release build, loopback, 30 s, 4 speakers. CPU: Intel Core i7-12700. "E-cores" is `taskset -c 16,17` (the closer proxy for the 2 vCPU Ryzen 7 3700X VM); "P-cores" is `taskset -c 0,2` (two physical cores). Server CPU is percent of one core.

| Load | Server pinning | Server CPU | Audio, worst pair | Video, min pair | Result |
|---|---|---|---|---|---|
| 12 speakers, no share | E-cores | 26.8% | 95% or more (not recorded) | n/a | PASS |
| 2 sharers x 10 Mbps | E-cores | 27.4% | 99.7% | 93.5% | PASS |
| 2 sharers x 10 Mbps | P-cores | 17.1% | 95% or more (not recorded) | 93.9% | PASS |
| 4 sharers x 10 Mbps | E-cores | 40.2% | 99.6% | 95.1% | PASS |
| 4 sharers x 10 Mbps | P-cores | 28.4% | 99.9% | 94.7% | PASS |
| 4 sharers x 20 Mbps | E-cores | 48.6% to 49.0% | 87.9% (loss 11.8%), 2 runs | 75.2% to 76.3% | FAIL |
| 4 sharers x 20 Mbps | P-cores | 44.9% to 45.1% | 93.2% (loss 5.6% to 6.6%), 2 runs | 66.7% to 72.7% | FAIL |
| 4 sharers x 20 Mbps, cap raised locally, drop counters read | E-cores | 48.9% | 86.9% (loss 12.70% to 12.89%), 1 run | 75.7% | FAIL |

The 4 x 20 Mbps rows were taken before the cap existed; on this build the third 20 Mbps share is refused, and the load test then exits 2 with the server's error. Only 4-person rooms were measured, and the cap scales with viewers, so a larger room with one share can pass the cap untested. The usual load (2 sharers x 10 Mbps) and 4 x 10 Mbps (120 000 kbps counted per viewer) pass. The worst load (4 x 20 Mbps, 240 000 kbps counted per viewer) fails on both pinnings, so the server caps a room's shares in total at 120 000 kbps (`MAX_ROOM_SHARE_KBPS` in `voice.rs`; see [known issues](../known-issues.md)). The cap is the highest tested load that passed; loads between 120 000 and 240 000 were not tested.

The failing runs did not show a saturated core (server CPU stayed under 50% of one core). The drop counters below show where the loss is. The load test was not run on the Dockge VM; that is Toby's call. The i7-12700 E-cores are estimated, not measured, to match a 3700X thread.

**Where the 4 x 20 Mbps loss is** (measured 2026-11-10, the last row above: one run, E-cores, with `MAX_ROOM_SHARE_KBPS` raised locally for the run and then put back). The UDP drop counters were read before and after the run, and once a second during it, since the load test's sockets close when it exits: the `drops` column of `/proc/net/udp` per socket (the `d` field of `ss -uapm`), and `nstat -az UdpRcvbufErrors UdpSndbufErrors UdpInErrors` for the whole machine.

- The server's media socket dropped 50 146 packets, about 1 600 a second from the second second of the run on. Its receive buffer was the kernel default, 212 992 bytes.
- The load test's four sockets dropped none while the run lasted. Two of them dropped 1 400 and 187 in the last sample, after the load test had begun to close its sockets.
- `UdpRcvbufErrors` rose by 51 733, the sum of the drops above. `UdpSndbufErrors` stayed at 0.

So the voice loss is at the server's receive buffer. The SFU thread reads its one socket too slowly for this load, and voice packets that arrive behind the video are dropped with it. The CPU figure is an average over the run, so it does not rule out bursts. Likely, not verified: the fake senders write each 41 kB frame at once, so the four sharers' frames arrive in bursts that the buffer cannot hold. A bigger buffer was not tried; see [known issues](../known-issues.md).

## 2. Mouth-to-ear latency: acoustic loopback

This measures the whole path including capture, DeepFilterNet, Opus, network, jitter buffer and playback, which WebRTC stats cannot see.

1. Put machine A and machine B in the same room, both in the same voice room, B on speakers (not headphones) so a third device can hear it. A's microphone must hear A's own speaker (laptop speakers are fine) so a click played on A is picked up and sent.
2. Place a recorder (phone voice memo, or a third machine) where it hears both A's speaker and B's speaker about equally.
3. Generate a click: `python3 docs/testing/click_offset.py --make-click click.wav`, then play it on A (`ffplay -nodisp -autoexit click.wav`).
4. A's mic sends it; B plays it. The recording contains two clicks: first from A's speaker, then B's playback. The gap is the mouth-to-ear latency (plus about 3 ms per metre of speaker distance difference).
5. Measure the gap in Audacity (select between the two click onsets; the selection length is shown at the bottom), or run `python3 docs/testing/click_offset.py recording.wav` (16-bit WAV, stdlib only; convert other formats with `ffmpeg -i memo.m4a -ac 1 recording.wav`). Pass a second argument (0..1) to change the detection threshold if it finds the wrong clicks. Repeat 5 to 10 times and report the median.

Caveats: browser echo cancellation may suppress a very short click on B, so use a few repeats or a 50 ms tone burst if clicks vanish. The script was checked here only against a synthetic recording (two clicks 187 ms apart, reported 187.0 ms), not against a real call.

## 3. WebRTC stats instead

When you only need network behaviour (RTT, jitter, packet loss) and not the full path, open the Diagnostics page in the client during a call. It shows RTT, jitter and loss from `getStats()`. Use it for before/after comparisons of network changes (section 4), for remote users you cannot stand next to, and for long soak runs. Use the acoustic method for anything involving the audio pipeline (denoiser mode, Opus settings, jitter buffer).

## 4. Simulating loss and jitter with `tc netem`

For a two-browser call use [bad-network.md](bad-network.md) and `scripts/netem-call.sh` instead: it shapes only the media into the SFU and runs root steps through `sudoq`. The recipe below shapes all of `lo`, for the load test.

**These commands need root (shown with `sudo`). Do not run this on a machine you reach over the network: `lo` is shared by everything on the box, and it affects everything using loopback, including SSH tunnels to localhost. The recipe is unverified (never run by the author).** The recipe below uses `lo`, with server and load test both on 127.0.0.1 as in section 1.

Add 5% loss and 20 ms delay with 10 ms jitter on loopback:

```sh
sudo tc qdisc add dev lo root netem loss 5% delay 20ms 10ms
tc qdisc show dev lo          # confirm
# ... run vesper-loadtest as in section 1 ...
```

Change values in place with `sudo tc qdisc change dev lo root netem loss 10% delay 40ms 20ms`.

**Remove it afterwards (always):**

```sh
sudo tc qdisc del dev lo root
tc qdisc show dev lo          # should show no netem
```

Note netem on `lo` applies to both directions of every loopback packet (including the WebSocket signaling), so expect roughly double the configured delay end to end and loss on both legs. With 5% loss each way, the load test is expected to FAIL its 95% bar; that is the point of the exercise (it proves the harness detects loss).

A dummy interface does NOT work as a substitute: traffic between two local addresses is routed over `lo`, so netem on the dummy device does nothing. Shaping a non-loopback path needs two network namespaces joined by a veth pair (unverified, not described here; not run).

## Prefetch on intent

`npm --prefix web run bench:intent` runs the real `project` rule from `web/src/lib/ui/intent.ts` and the 80 ms hover rule on
simulated mouse moves. Each move is a minimum-jerk path with a slight arc and 1 px of jitter, timed by Fitts's law, sampled
at 60 Hz, then a 120 ms rest before the click. The layout is the sidebar's: 15 rows of 28 px. 400 moves per case, seeded.
"Lead" is how long before the click the target room's fetch starts. "Wrong" is other rooms fetched per move.

Measured 2026-11-14 on the dev machine:

| Case | Hover lead (median) | Hover + velocity lead (median, p10) | Wrong, hover | Wrong, hover + velocity |
| --- | --- | --- | --- | --- |
| Main pane to a row | 160 ms | 401 ms, 234 ms | 0.47 | 2.84 |
| Row to another row | 157 ms | 375 ms, 321 ms | 1.66 | 5.95 |
| Within the main pane | none | none | 0 | 0 |

Variants tried and dropped: checking three points along the projection, nearest first (lead 297 ms, wrong 2.15), the same
row on 3 samples in a row (257 ms, 1.17), and only while slowing down (293 ms, 2.02). A wrong room costs one history GET,
once per session, since a loaded room is cached. The test fails if the lead gain drops under 150 ms or wrong passes 3.5.

Not verified: real mouse paths. The model has no overshoot, no corrections and no pauses, so real numbers will be lower.
The ms saved in a browser depend on how long the history GET takes, which was not measured.
