# Latency and load testing

## 1. Load test (packet loss and server CPU)

`vesper-loadtest` joins N synthetic speakers to one room over the real WebSocket protocol, each sends 20 ms Opus-sized packets for S seconds, and every speaker counts what it received from each other speaker. It exits 1 if any speaker received under 95% of its peers' packets (2 on setup errors). `--pid` adds the server's CPU over the run; that reads `/proc/<pid>/stat`, so it is Linux only (and assumes 100 clock ticks per second).

Against a local server on loopback with a throwaway key (ports are arbitrary high ones):

```sh
cargo build --release -p vesper-server -p vesper-loadtest

VESPER_DEV_KEY=throwaway-lt VESPER_HTTP_BIND=127.0.0.1:47123 VESPER_UDP_BIND=127.0.0.1:47124 \
  target/release/vesper-server &
SERVER=$!

target/release/vesper-loadtest --url ws://127.0.0.1:47123/ws --key throwaway-lt \
  --room lt --speakers 12 --seconds 30 --pid $SERVER
echo "exit=$?"

kill $SERVER
```

The server's `no advertised addresses` warning is expected on loopback; the fake clients use host candidates. Run the load test on the same machine as the server: client and server CPU are then both on that box, so read the CPU figure as an upper bound.

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

**These commands need root (shown with `sudo`). Do not run them on a machine reached over the network except on `lo`, and even that affects everything using loopback, including SSH tunnels to localhost.** Prefer a dummy interface so nothing else is touched; but media must then be routed over it, which is awkward. The simplest safe recipe is `lo`, with server and load test both on 127.0.0.1 as in section 1.

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

Note netem on `lo` applies to both directions of every loopback packet (including the WebSocket signaling), so expect roughly double the configured delay end to end and loss on both legs. With 5% loss each way, the load test is expected to FAIL its 95% bar; that is the point of the exercise (it proves the harness detects loss). For a dummy interface instead: `sudo ip link add dum0 type dummy && sudo ip addr add 10.99.0.1/24 dev dum0 && sudo ip link set dum0 up`, bind the server to `10.99.0.1`, apply netem to `dum0`, and remove with `sudo ip link del dum0`.
