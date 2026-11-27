# Benchmarks

Every minor and major release (x.y.0 and x.0.0) gets a benchmark report: the same cases, on the same machine, with the same
rules, compared against the last minor release. The idea comes from rvoip, which publishes benchmark results with each release
in the style of RFC 7502 (a fixed method, a described test bed, numbers per release). Only the idea is borrowed; no code.

This folder holds the reports: `<version>.md` to read and `<version>.json` with every number, raw runs included. Only
`vX.Y.0` files count as baselines. Other names, such as `v2.3.2-33-g61e6cfb` for `main` between releases, are kept for
reference.

## Run it

```sh
just bench --name v2.4.0     # the full suite, about 5 minutes: docs/benchmarks/v2.4.0.md and .json
just bench --quick           # smoke: 3 load cases once, 5 s each, about 30 s, written to target/bench/
just bench                   # named after `git describe`
```

The recipe builds `vesper-server`, `vesper-loadtest` and the denoiser bench (`crates/vesper-dsp/examples/dsp_bench.rs`, with
the model from `scripts/fetch-model.sh`) under `scripts/capped.sh`, then runs `scripts/bench.py`. Each server run gets a fresh
data dir and free loopback ports. Server, load test and denoiser bench run at `nice 10`, and every child process has a timeout.
It exits 0 when every limit is met and nothing regressed, 1 when a limit is missed or a regression is flagged, and 2 on a setup
error. `scripts/bench.py --help` lists the flags: `--repeats` (1 to 5, default 3), `--baseline FILE` or `--baseline none`,
`--no-dsp`, `--out FILE`.

### Against another branch or release

The tools do not need to come from the tree under test. Build its server there and pass it:

```sh
git worktree add --detach /tmp/vesper-v2.3.0 v2.3.0
(cd /tmp/vesper-v2.3.0 && scripts/capped.sh cargo build --release --locked -p vesper-server)
just bench --name v2.3.0 --baseline none --server-bin /tmp/vesper-v2.3.0/target/release/vesper-server
```

The report names the commit of the tree that holds the binary, and the commit of the bench tools separately. Everything but the
denoiser rows goes through the server binary, so any release with `loadtest-accounts` can be measured (v1.0 on). The denoiser
rows always measure the bench tools' tree; the report says whether `crates/vesper-dsp` there is the same as in the server's
commit. For the UDP receive path work: build that branch's server and run `just bench --server-bin <it> --out /tmp/udp.md`,
which compares against the last minor release.

## Cases

| Case | What it does |
|---|---|
| Load: 4, 8 and 12 speakers | Audio only, everyone talks at once, 10 s. 12 is the server's room cap (`max_room_size`, fixed), so a room of 16 or 32 cannot be tested. |
| Load: 4 speakers with 2 and with 4 sharers x 10 Mbps | The usual load with screen share, and a room's whole share budget (`MAX_ROOM_SHARE_KBPS`, 120 000 kbps counted per viewer). Voice must still hold. |
| Join and resume | Every speaker of the audio cases joins one at a time: time from opening the WebSocket to `joined`, and from `joined` to ICE and DTLS up. After each run 4 speakers lose their TCP connection with no close frame, as a dropped network does, and send `resume` on a new socket; the time is from the new connect to `resumed`. Servers before v2.3.2 have no resume, so that cell is n/a. |
| Per participant | From the audio cases: the slope of server RSS over room size (MiB per extra person), and at 12 people the server CPU divided by 12 and by the packets it forwarded (microseconds of CPU per forwarded packet). Forwarding grows with the square of the room, so CPU per person grows with the room too; the per packet figure is the steady one. |
| Forwarding throughput | 12 speakers sending a packet every 10, 5 and 2 ms instead of 20, 5 s a step: forwarded packets a second, loss, CPU and CPU per packet. |
| Denoisers | DeepFilterNet3 and RNNoise on 5 s of a fixed test signal, natively: milliseconds of CPU per 10 ms frame. |

Every speaker sends a 160 byte Opus-shaped packet every 20 ms (about 64 kbps) unless a case says otherwise. Sharers send 60
VP8-shaped frames a second. Every speaker watches every sharer.

### What is measured

| Number | How |
|---|---|
| Loss and worst pair | Packets each speaker got from each other speaker, out of what that one sent. "Worst pair" is the lowest receiver-sender pair. |
| Forwarding latency | For every audio packet: when it reached the receiver's socket minus when the sender wrote it. All fake clients run in one process on one clock. p50 and p99 over all packets. |
| Jitter | RFC 3550 interarrival jitter (section 6.4.1) for each receiver-sender pair, from the same times. The worst pair. |
| Server CPU | utime plus stime of the server process over the run, from `/proc`, as a share of one core. |
| Server memory | `VmRSS` at the end of the run and `VmHWM` (peak), from `/proc`. |

The latency includes the fake clients' own work: str0m on both ends and the client threads. On an idle loopback the median is
about 2 ms, most of it outside the SFU. So the numbers track changes; they are not the SFU's cost alone.

### Why not micro-benchmarks of the SFU's functions

The SFU's hot path is private (`room.rs` and `client.rs`) and most of its time is str0m's: decrypting, routing and encrypting
each packet. A criterion bench of our functions would need them made public, and it could not run against an older release, so
it could not give a history. The suite measures the hot path from outside instead: CPU per forwarded packet and the throughput
steps go through the real server binary, so every release is measured the same way.

### Repeats, spread and the machine

Each case runs 3 times. The repeats go round all the cases, so a busy minute on the machine spreads over every case instead of
spoiling one. A cell is the median of the repeats with the lowest and highest in brackets. The denoiser bench runs its signal
5 times after a warm-up pass.

The report records the machine, CPU model, threads, memory, kernel, the CPU governor and turbo state, the average clock at
start, the load average at start and end, rustc, the commits, and the settings. The CPU clock is not pinned: that needs root,
and the dev machine runs the `powersave` governor with turbo on. So compare reports from one machine only, and read a small
move as noise.

Other work on the machine matters more than the clock. During one test run a `nice 19` build took about 12 cores, and the
same server binary then showed 40% more CPU per forwarded packet and three times the jitter, enough to flag 8 regressions
against itself. So the suite measures other work: before it starts it waits (up to 10 minutes, `--wait-quiet`) until the
machine uses under 1.5 cores, and during each run it counts every busy CPU tick on the machine minus the server's and the
load test's. The report shows that figure, and when its median over the runs passed 1.5 cores, in this report or in its baseline, the result reads
"FAIL, machine busy" instead of "FAIL": run it again when the machine is quiet.

The clock moves the other way too. With `powersave`, a quiet machine runs its cores slower, so the same work shows a higher CPU
share: the 12-speaker case took 18% of a core on a quiet machine and 6% while a build kept the clocks up. CPU figures are only
comparable between reports taken quiet, which is what the wait at the start is for.

## Limits

Each load case must meet all of these, on the median of its repeats. They sit in `LIMITS` in `scripts/bench.py`.

| Check | Limit | Why |
|---|---|---|
| vesper-loadtest's own rule | 95% in aggregate and from each peer | The existing bar of [latency.md](../testing/latency.md). |
| Audio, worst pair | at least 99% | Loopback has no network loss, so any loss is the server's. Opus FEC and loss concealment hide a percent or so; a real network takes its own share on top. Every case loses 0.00% on the dev machine. |
| Forwarding latency, p99 | at most 15 ms | Under one 20 ms Opus frame: a packet held longer than a frame makes the browser's jitter buffer grow. The 12-speaker p99 is 5 to 7 ms, so 15 ms is about twice that. |
| Jitter, worst pair | at most 5 ms | A quarter of a frame. Measured at about 2 ms; well over 5 means the SFU sends in bursts. |
| Server CPU | at most 50% of one core | The SFU is one thread, so one core is its ceiling. The live server is a 2 vCPU VM with a slower core than the dev machine; half a core here leaves room for that. The heaviest load case takes 13 to 30%. |
| Server memory, peak | at most 128 MiB | At most 38 MiB measured. Four times that would mean a leak or a buffer that grows with traffic. |

## Regressions

Each new report compares every number with the same number in the baseline: the newest `vX.Y.0.json` in this folder older than
the report's name, or `--baseline FILE`. A number is a **regression** when all three hold:

1. Its median moved the wrong way by more than the relative threshold below.
2. It also moved by more than the absolute floor, so a tiny number cannot flag on noise (0.3 ms to 0.5 ms is +66%).
3. The two ranges do not overlap: the new lowest is worse than the old highest.

| Number | Relative | Floor | Why |
|---|---|---|---|
| Latency p50, p99 | 25% | 0.5 ms, 1 ms | Moved up to 2% (p50) and 22% (p99, at the 2 ms throughput step) between two runs of one binary. |
| Jitter, worst pair | 50% | 0.5 ms | Moved up to 19% between two runs of one binary, and up to 50% within a run's repeats. |
| Server CPU | 25% | 2 points | Moved up to 10% between two runs of one binary, and up to 45% within a run's repeats. |
| Peak RSS | 10% | 2 MiB | Moved under 2%. Memory is steady, so a small rise is real. |
| Audio, worst pair | any drop | 0.5 points | It sits at 100%; half a point is about 2 700 packets at 12 speakers. |
| Join, media up, resume | 25% | 5 ms, 2 ms, 5 ms | Moved up to 9%, 2% and 0.3%. |
| RSS and CPU per participant | 25% | 0.1 MiB, 0.2 points | Moved up to 4%. Derived from the load cases, so as noisy as they are. |
| CPU per forwarded packet | 20% | 0.5 us | Moved up to 10%. |
| Forwarded packets a second | 15% lower | 1 000 /s | Did not move: the load generator sets the rate (see the notes in a report). |
| Denoiser per frame | 15% | 0.01 ms | Moved up to 4%, and up to 16% within a run. |

The thresholds rest on the backfill (2026-11-26, dev machine, quiet): the same server, `main` at `61e6cfb`, measured twice,
gave no flag, and the "moved" figures above are the largest changes of a median between those two runs. Each threshold sits
above that move, and the ranges rule keeps a noisy number from flagging alone. `main` against v2.3.0 moved less than 10% on
every number, which fits: the SFU changed little between them. A regression makes `just bench` exit 1, and it blocks the release until someone
explains it: a real cost of a new feature is fine when it is written down in the release notes.

## Each release

| Release | What runs | Who, where | How long |
|---|---|---|---|
| Minor or major (x.y.0, x.0.0) | `just bench --name vX.Y.Z`, the full suite | The agent or Toby preparing the release pull request, on the dev machine | About 5 minutes, plus the build |
| Patch (x.y.z) | `just bench --quick` when the patch touches `crates/vesper-sfu`, the server's voice or WebSocket code, or `crates/vesper-dsp`; otherwise nothing | The same | About 30 s |

A patch release is bug fixes; the minor's report stays its baseline, and a full run per patch would add noise to the history
without a decision to make. The quick run checks the limits and compares with the last minor; its file stays in `target/bench/`.

For a minor or major release:

1. On the dev machine, on the release branch, with the machine quiet: `just bench --name vX.Y.Z`.
2. Read the Verdict. A missed limit or a regression blocks the release until it is understood. Run it once more first if the load average at start was high.
3. Commit `docs/benchmarks/vX.Y.Z.md` and `.json` in the release pull request. They do not cost the light CI pipeline.
4. After the tag, attach the Markdown to the GitHub release: `gh release upload vX.Y.Z docs/benchmarks/vX.Y.Z.md`.

It is a checklist step and not a CI job on purpose. GitHub runners are shared and change hardware from run to run, so their
numbers could not be held against the last release, and the CPU and latency limits would fail at random. Not verified: the
suite was not run on a GitHub runner.

## Limits of the method

- Loopback only. No real network, no `tc netem`, and server and load generator share the machine's cores. The throughput steps
  stop where the fake clients stop keeping up, not where the SFU does; the report says so when the server stayed under 90% of a core.
- One room at a time, at most 12 people. Several busy rooms at once on the one SFU thread are not tested.
- Audio is fixed-size packets at a steady rate: no DTX silence and no talk spurts. Video frames are fixed-size too.
- The denoisers run natively, not as WASM in a browser. `node scripts/bench-wasm.mjs chromium` measures the browser, by hand.
- Linux only (`/proc`). CPU assumes 100 clock ticks a second.
