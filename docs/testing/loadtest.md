# Release load test

Each release gets a load test report: the same scenarios, on the same machine, with the same pass rules, so one release can be
held against the last. The idea comes from rvoip, which publishes benchmark results with each release in the style of RFC 7502
(a fixed method, a described test bed, numbers per release). Only the idea is borrowed; no code.

Reports live in [loadtest/](loadtest/), one Markdown file and one JSON file per release. The JSON holds every number, for
comparing releases by script.

## Run it

```sh
just loadtest-report                  # names the report after `git describe`
just loadtest-report --name v2.4.0    # for a release: docs/testing/loadtest/v2.4.0.md
```

The recipe builds `vesper-server` and `vesper-loadtest` (release, under `scripts/capped.sh`), then runs
`scripts/loadtest-report.py`. Each scenario starts its own server on a throwaway data dir and free loopback ports, makes the
accounts with `loadtest-accounts`, and runs [`vesper-loadtest`](latency.md#1-load-test-packet-loss-and-server-cpu) against it.
Server and load test run at `nice 10`. A scenario sends 10 s of audio (`--seconds`, at most 60), and the whole run takes about
a minute. It exits 0 if every scenario passes, 1 if one fails, 2 on a setup error. A setup error is a FAIL row in the report.

Other flags: `--sizes 4,8,12` (room sizes, 2 to 12), `--no-video` (skip the share scenarios), `--out FILE`. `scripts/loadtest-report.py --help` lists them.

### Against another branch

The load test does not need to come from the branch under test. Build the server in that branch's worktree and pass it:

```sh
(cd ../vesper-other && scripts/capped.sh cargo build --release --locked -p vesper-server)
just loadtest-report --server-bin ../vesper-other/target/release/vesper-server --out /tmp/other.md
```

The report names the commit and branch of the tree that holds the binary, and the commit of the load test separately. Run the
current branch the same way for a before and after.

## Scenarios

| Scenario | Why |
|---|---|
| 4, 8 and 12 speakers | Audio only, everyone talks at once. 12 is the server's room cap (`max_room_size`, fixed), so a room of 16 or 32 cannot be tested. |
| 4 speakers, 2 sharers x 10 Mbps | The usual load with screen share. |
| 4 speakers, 4 sharers x 10 Mbps | A room's whole share budget (`MAX_ROOM_SHARE_KBPS`, 120 000 kbps counted per viewer). Voice must still hold. |

Every speaker sends a 160 byte Opus-shaped packet every 20 ms (about 64 kbps). Sharers send 60 VP8-shaped frames a second.
Every speaker watches every sharer.

## What is measured

| Number | How |
|---|---|
| Loss, worst pair | Packets each speaker got from each other speaker, out of what that one sent. "Worst pair" is the lowest receiver-sender pair. |
| Forwarding latency | For every audio packet that arrived: the time it reached the receiver's socket minus the time the sender wrote it. All fake clients run in one process and share one clock. Reported as p50, p95, p99 and max over all packets. |
| Jitter | RFC 3550 interarrival jitter (section 6.4.1) for each receiver-sender pair, from the same send and arrival times. Mean and worst pair. |
| Server CPU | utime plus stime of the server process over the run, from `/proc`, as a share of one core. |
| Server memory | `VmRSS` before the run and at its end, and `VmHWM` (peak), from `/proc`. |
| Video worst pair | Frames a viewer got from a sharer, out of those queued. Shown, not part of PASS: the fake sender paces at a little under 60 a second. |

The latency includes the fake clients' own work: str0m on both ends and the client threads. On an idle loopback the median is
about 2 ms, most of it outside the SFU. So the numbers track changes; they are not the SFU's cost alone.

## Pass rules

A scenario passes when all of these hold. They sit in `THRESHOLDS` in `scripts/loadtest-report.py`.

| Check | Limit | Why |
|---|---|---|
| vesper-loadtest's own rule | 95% in aggregate and from each peer | The existing bar of [latency.md](latency.md). |
| Audio, worst pair | at least 99% | Loopback has no network loss, so any loss is the server's. Opus FEC and loss concealment hide a percent or so; a real network takes its own share on top. On the dev machine every scenario loses 0.00%. |
| Forwarding latency, p99 | at most 15 ms | Under one 20 ms Opus frame: a packet held longer than a frame makes the browser's jitter buffer grow. The 12-speaker p99 is about 7 ms, so 15 ms is about twice that. |
| Jitter, worst pair | at most 5 ms | A quarter of a frame. Measured at about 2 ms; well over 5 means the SFU sends in bursts. |
| Server CPU | at most 50% of one core | The SFU is one thread, so one core is its ceiling. The live server is a 2 vCPU VM with a slower core than the dev machine. Half a core here leaves room for that. The heaviest scenario is about 20 to 30%. |
| Server memory, peak | at most 128 MiB | About 40 MiB at the heaviest scenario. Four times that would mean a leak or a buffer that grows with traffic. |

The limits were picked on 2026-11-26 from two runs on the dev machine (Core i7-12700, 20 threads). They hold for that machine.
A slower machine can fail on CPU or latency with no regression in the code; compare reports from one machine only.

## Each release

The release pull request carries the report (see "How a release is cut" in [versions.md](../versions.md#how-a-release-is-cut)):

1. On the dev machine, on the release branch: `just loadtest-report --name vX.Y.Z`.
2. Read it. A FAIL blocks the release until it is understood. Run it once more first if the machine was busy (the report shows the load average at start).
3. Commit `docs/testing/loadtest/vX.Y.Z.md` and `.json` in the release pull request. They do not cost the light CI pipeline.
4. After the tag, attach the Markdown to the GitHub release: `gh release upload vX.Y.Z docs/testing/loadtest/vX.Y.Z.md`.

It is a checklist step and not a CI job on purpose. GitHub runners are shared and change hardware from run to run, so their
numbers could not be held against the last release, and the CPU and latency limits would fail at random. Not verified: the
load test was not run on a GitHub runner.

## Limits

- Loopback only. No real network, no `tc netem`, and server and load test share the machine's cores.
- One room at a time, at most 12 people. Several busy rooms at once on the one SFU thread are not tested.
- Audio is fixed-size packets at a steady rate: no DTX silence and no talk spurts. Video frames are fixed-size too.
- Linux only (`/proc`). CPU assumes 100 clock ticks a second.
