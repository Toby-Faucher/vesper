#!/usr/bin/env python3
"""Vesper's benchmark suite: load, forwarding throughput, join and resume, per participant cost, and
the denoisers. Writes docs/benchmarks/<name>.md and .json and compares against the last minor
release's report. Method, limits and thresholds: docs/benchmarks/README.md. Run it with `just bench`.

  scripts/bench.py [--name vX.Y.Z] [--repeats 3] [--quick] [--server-bin PATH] [--baseline FILE|none]
  scripts/bench.py --render RESULTS.json [--baseline FILE|none] [--out FILE]   (no runs; for tests)

Every server run gets a fresh data dir and free loopback ports. Server, load test and denoiser
bench run at nice 10, and every child has a timeout. --server-bin measures a vesper-server built
from another tree; the report names that tree's commit.
Exit 0: every limit met and no regression. 1: a limit failed or a regression was flagged. 2: setup error.
"""

import argparse
import datetime
import glob
import json
import os
import platform
import resource
import re
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BENCH_DIR = os.path.join(ROOT, "docs", "benchmarks")
NICE = "10"
SECONDS = 10
# The server's fixed room cap (`max_room_size` in crates/vesper-server/src/config.rs).
MAX_ROOM = 12

# Load cases: (id, speakers, sharers). Each share is 10 Mbps. 4 sharers is the room's share
# budget (MAX_ROOM_SHARE_KBPS, 120 000 kbps counted per viewer).
LOAD = [("load-4", 4, 0), ("load-8", 8, 0), ("load-12", 12, 0), ("share-2", 4, 2), ("share-4", 4, 4)]
QUICK_LOAD = {"load-4", "load-12", "share-2"}
# Throughput: 12 speakers, packet gap stepped down from 20 ms. 5 s a step.
THROUGHPUT_SPEAKERS = 12
THROUGHPUT_STEPS_MS = [10, 5, 2]
THROUGHPUT_SECONDS = 5
RECONNECTS = 4
# Other work on the machine, in cores, above which a report's verdict is marked as not to be trusted.
# A nice 19 build on the dev machine (about 12 cores busy) raised the same binary's CPU per packet
# by 40% and its jitter threefold; at under 1.5 cores of other work, two runs of one binary agree.
BUSY_CORES = 1.5
CLK_TCK = os.sysconf("SC_CLK_TCK") if hasattr(os, "sysconf") else 100

# Absolute limits on the median of the repeats, for every load case. Why: docs/benchmarks/README.md.
LIMITS = {
    "worst_pair": ("min", 99.0, "Audio delivered, worst pair", "%"),
    "latency_p99": ("max", 15.0, "Forwarding latency p99", "ms"),
    "jitter_max": ("max", 5.0, "Jitter (RFC 3550), worst pair", "ms"),
    "cpu": ("max", 50.0, "Server CPU, share of one core", "%"),
    "rss_peak": ("max", 128.0, "Server memory, peak RSS", "MiB"),
}

# Regression rule per metric suffix: (lower or higher is better, relative change, absolute floor).
# Flagged only when the median moved by more than both AND the two runs' ranges do not overlap.
REGRESSION = {
    "worst_pair": ("higher", 0.0, 0.5),
    "latency_p50": ("lower", 0.25, 0.5),
    "latency_p99": ("lower", 0.25, 1.0),
    "jitter_max": ("lower", 0.50, 0.5),
    "cpu": ("lower", 0.25, 2.0),
    "rss_peak": ("lower", 0.10, 2.0),
    "join_ms": ("lower", 0.25, 5.0),
    "media_ms": ("lower", 0.25, 2.0),
    "resume_ms": ("lower", 0.25, 5.0),
    "rss_per_participant": ("lower", 0.25, 0.1),
    "cpu_per_participant": ("lower", 0.25, 0.2),
    "us_per_packet": ("lower", 0.20, 0.5),
    "forwarded_per_s": ("higher", 0.15, 1000.0),
    "ms_per_frame": ("lower", 0.15, 0.01),
}

LABELS = {
    "worst_pair": ("Audio, worst pair", "%"),
    "loss": ("Audio loss", "%"),
    "latency_p50": ("Latency p50", "ms"),
    "latency_p99": ("Latency p99", "ms"),
    "jitter_max": ("Jitter, worst pair", "ms"),
    "cpu": ("Server CPU", "%"),
    "rss_peak": ("Peak RSS", "MiB"),
    "join_ms": ("Join to `joined`", "ms"),
    "media_ms": ("`joined` to media up", "ms"),
    "resume_ms": ("Drop to `resumed`", "ms"),
    "rss_per_participant": ("RSS per participant", "MiB"),
    "cpu_per_participant": ("CPU per participant, 12 in a room", "%"),
    "us_per_packet": ("CPU per forwarded packet", "us"),
    "forwarded_per_s": ("Forwarded packets", "/s"),
    "ms_per_frame": ("CPU per 10 ms frame", "ms"),
    "video_worst_pair": ("Video, worst pair", "%"),
}


# ---------------------------------------------------------------- summary and comparison


def summarize(values):
    """median, min, max, n of the numbers that are not None; None when there are none."""
    v = [x for x in values if x is not None]
    if not v:
        return None
    return {"median": statistics.median(v), "min": min(v), "max": max(v), "n": len(v)}


def suffix(metric_id):
    return metric_id.rsplit(".", 1)[-1]


def limit_failures(summary):
    """[(metric id, label)] for load-case metrics whose median misses its limit."""
    failed = []
    for mid, s in summary.items():
        if not mid.startswith(("load-", "share-")) or suffix(mid) not in LIMITS:
            continue
        kind, limit, name, _ = LIMITS[suffix(mid)]
        if s is None or (s["median"] < limit if kind == "min" else s["median"] > limit):
            failed.append((mid, name))
    return failed


def compare(summary, baseline):
    """{metric id: (verdict, change)} against a baseline summary. verdict: regression, better, same, new."""
    out = {}
    for mid, s in summary.items():
        rule = REGRESSION.get(suffix(mid))
        b = (baseline or {}).get(mid)
        if rule is None or s is None:
            continue
        if b is None:
            out[mid] = ("new", None)
            continue
        better, rel, floor = rule
        delta = s["median"] - b["median"]
        worse = -delta if better == "higher" else delta
        moved = abs(delta) > floor and abs(delta) > rel * abs(b["median"])
        apart = s["min"] > b["max"] or s["max"] < b["min"]
        if moved and apart:
            out[mid] = ("regression" if worse > 0 else "better", delta)
        else:
            out[mid] = ("same", delta)
    return out


def version_key(name):
    m = re.fullmatch(r"v(\d+)\.(\d+)\.(\d+)", name)
    return tuple(int(x) for x in m.groups()) if m else None


def find_baseline(name, here):
    """The newest docs/benchmarks/vX.Y.0.json older than `name` (or the newest at all if `name`
    is not a version), skipping the report being written."""
    mine = version_key(name)
    found = []
    for f in glob.glob(os.path.join(BENCH_DIR, "v*.json")):
        stem = os.path.splitext(os.path.basename(f))[0]
        k = version_key(stem)
        if k is None or k[2] != 0 or os.path.abspath(f) == os.path.abspath(here):
            continue
        if mine is None or k < mine:
            found.append((k, f))
    return max(found)[1] if found else None


# ---------------------------------------------------------------- rendering


def fmt(v, unit=""):
    if v is None:
        return "n/a"
    if unit == "/s":
        return f"{v:,.0f}".replace(",", " ")
    digits = 3 if unit == "ms" and abs(v) < 1 else (2 if abs(v) < 10 else 1)
    return f"{v:.{digits}f}"


def cell(s, unit):
    if s is None:
        return "n/a"
    if s["n"] == 1:
        return fmt(s["median"], unit)
    return f"{fmt(s['median'], unit)} ({fmt(s['min'], unit)} to {fmt(s['max'], unit)})"


def verdict_text(c, unit):
    if c is None:
        return ""
    v, delta = c
    if v == "new":
        return "new"
    sign = "+" if delta >= 0 else ""
    word = {"regression": "**REGRESSION** ", "better": "better ", "same": ""}[v]
    return f"{word}{sign}{fmt(delta, unit)}"


def render(doc, baseline_doc=None, baseline_name=None):
    m, summary = doc["meta"], doc["summary"]
    cmp = compare(summary, baseline_doc["summary"] if baseline_doc else None)
    fails = limit_failures(summary)
    regressions = [k for k, (v, _) in cmp.items() if v == "regression"]
    ok = not fails and not regressions and not doc.get("errors")
    busy = [n for n, d in ((m["name"], doc), (baseline_name, baseline_doc)) if d and (d["summary"].get("machine.other_cores") or {}).get("median", 0) > BUSY_CORES]
    head = "PASS" if ok else ("FAIL, machine busy" if busy else "FAIL")
    out = [
        f"# Benchmarks: {m['name']}",
        "",
        f"Result: **{head}**. {len(fails)} limits missed, {len(regressions)} regressions"
        + (f" against {baseline_name}" if baseline_name else ", no baseline")
        + ". Made by `just bench` ([method](README.md)).",
        "",
        "| | |",
        "|---|---|",
        f"| Date (UTC) | {m['date']} |",
        f"| Server | `{m['commit']}` ({m['describe']}){' with uncommitted changes' if m['dirty'] else ''}, branch `{m['branch']}` |",
        f"| Bench tools | `{m['bench_commit']}`. Denoiser code {m['dsp_note']} |",
        f"| Machine | {m['host']}: {m['cpu']}, {m['threads']} threads, {m['mem_gib']} GiB, Linux {m['kernel']} |",
        f"| CPU frequency | governor `{m['governor']}`, turbo {m['turbo']}, {m['mhz']} MHz average at start. Not pinned (needs root). |",
        f"| Load average | {m['loadavg_start']} at start, {m['loadavg_end']} at end |",
        f"| Other work during the runs | {cell(summary.get('machine.other_cores'), 'cores')} cores (a median over {BUSY_CORES} marks the verdict as not trusted) |",
        f"| Toolchain | {m['rustc']} |",
        f"| Settings | {m['settings']} |",
        "",
    ]
    if baseline_name:
        out += [f"Changes are against [{baseline_name}]({baseline_name}.md). Each cell is the median of the repeats, with the lowest and highest in brackets.", ""]
    else:
        out += ["Each cell is the median of the repeats, with the lowest and highest in brackets.", ""]

    def table(title, rows, cols):
        nonlocal out
        out += [f"## {title}", "", "| " + " | ".join(["Case"] + [f"{LABELS[c][0]} ({LABELS[c][1]})" for c in cols]) + " |"]
        out.append("|" + "---|" * (len(cols) + 1))
        for case, label in rows:
            cells = [label]
            for c in cols:
                mid = f"{case}.{c}"
                s = summary.get(mid)
                text = cell(s, LABELS[c][1])
                vt = verdict_text(cmp.get(mid), LABELS[c][1]) if baseline_name else ""
                cells.append(text + (f", {vt}" if vt else ""))
            out.append("| " + " | ".join(cells) + " |")
        out.append("")

    load_rows = [(cid, case_label(sp, sh)) for cid, sp, sh in LOAD if any(k.startswith(cid + ".") for k in summary)]
    table("Load", load_rows, ["loss", "worst_pair", "latency_p50", "latency_p99", "jitter_max", "cpu", "rss_peak"])
    share_rows = [r for r in load_rows if r[0].startswith("share-")]
    if share_rows:
        table("Video in the share cases", share_rows, ["video_worst_pair"])
    table("Join and resume", [("calls", "All load cases without shares")], ["join_ms", "media_ms", "resume_ms"])
    table("Per participant", [("scale", "Audio, 4 to 12 speakers")], ["rss_per_participant", "cpu_per_participant", "us_per_packet"])
    tp = [(f"throughput-{ms}ms", f"{THROUGHPUT_SPEAKERS} speakers, a packet every {ms} ms") for ms in THROUGHPUT_STEPS_MS]
    tp = [r for r in tp if any(k.startswith(r[0] + ".") for k in summary)]
    if tp:
        table("Forwarding throughput", tp, ["forwarded_per_s", "worst_pair", "cpu", "us_per_packet", "latency_p99"])
    dsp = [(e, n) for e, n in (("deepfilternet", "DeepFilterNet3 (native, not WASM)"), ("rnnoise", "RNNoise")) if f"{e}.ms_per_frame" in summary]
    if dsp:
        table("Denoisers", dsp, ["ms_per_frame"])

    out += ["## Verdict", ""]
    if fails:
        out += [f"- Limit missed: {mid} ({name})" for mid, name in fails]
    if regressions:
        out += [f"- Regression: {mid}" for mid in regressions]
    for e in doc.get("errors", []):
        out.append(f"- Setup error in {e['case']}: {e['error']}")
    for n in busy:
        mx = (doc if n == m["name"] else baseline_doc)["summary"]["machine.other_cores"]["median"]
        out.append(f"- The machine was busy during {n}: {mx:.1f} cores of other work in a typical run. Its numbers may be noise; run again on a quiet machine before trusting a FAIL.")
    if ok:
        out.append("- Every limit met" + (f", no regression against {baseline_name}." if baseline_name else ". No baseline to compare against."))
    out += [""]
    if doc.get("notes"):
        out += ["## Notes", ""] + [f"- {n}" for n in doc["notes"]] + [""]
    return "\n".join(out), ok


def case_label(speakers, sharers):
    return f"{speakers} speakers" + (f", {sharers} sharers x 10 Mbps" if sharers else "")


# ---------------------------------------------------------------- running


def git(*args, cwd=ROOT):
    try:
        return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, check=True).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def read(path, default="unknown"):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return default


def machine():
    cpu, mhz = "unknown", []
    for line in read("/proc/cpuinfo", "").splitlines():
        if line.startswith("model name") and cpu == "unknown":
            cpu = line.split(":", 1)[1].strip()
        if line.startswith("cpu MHz"):
            mhz.append(float(line.split(":", 1)[1]))
    mem = next((int(l.split()[1]) for l in read("/proc/meminfo", "").splitlines() if l.startswith("MemTotal:")), 0)
    no_turbo = read("/sys/devices/system/cpu/intel_pstate/no_turbo", "")
    boost = read("/sys/devices/system/cpu/cpufreq/boost", "")
    turbo = "off" if no_turbo == "1" or boost == "0" else ("on" if no_turbo == "0" or boost == "1" else "unknown")
    try:
        rustc = subprocess.run(["rustc", "-V"], cwd=ROOT, capture_output=True, text=True).stdout.strip() or "unknown"
    except OSError:
        rustc = "unknown"
    return {
        "host": platform.node() or "unknown",
        "cpu": cpu,
        "threads": os.cpu_count(),
        "mem_gib": round(mem / 1024 / 1024),
        "kernel": platform.release(),
        "governor": read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        "turbo": turbo,
        "mhz": round(statistics.mean(mhz)) if mhz else "unknown",
        "loadavg_start": " ".join(read("/proc/loadavg").split()[:3]),
        "rustc": rustc,
    }


def cpu_jiffies():
    """(busy, total) jiffies of the whole machine from /proc/stat; (0, 0) when unreadable."""
    try:
        with open("/proc/stat") as f:
            v = [int(x) for x in f.readline().split()[1:9]]
    except (OSError, ValueError):
        return (0, 0)
    idle = v[3] + v[4]
    return (sum(v) - idle, sum(v))


def busy_cores(seconds):
    """Cores in use by everything on the machine over the next `seconds`."""
    b0, _ = cpu_jiffies()
    time.sleep(seconds)
    b1, _ = cpu_jiffies()
    return (b1 - b0) / CLK_TCK / seconds


def wait_quiet(limit_s):
    """Wait until under BUSY_CORES of other work, up to limit_s; returns the last sample."""
    deadline = time.monotonic() + limit_s
    while True:
        c = busy_cores(5)
        if c < BUSY_CORES or time.monotonic() > deadline:
            return c
        print(f"   machine busy ({c:.1f} cores in use), waiting for it to settle", file=sys.stderr)
        time.sleep(10)


def free_port(kind):
    with socket.socket(socket.AF_INET, kind) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def load_run(args, speakers, sharers, seconds, packet_ms=20, reconnects=0):
    """One fresh server and one vesper-loadtest run; its JSON summary. Raises RuntimeError on setup errors."""
    with tempfile.TemporaryDirectory(prefix="vesper-bench-") as data:
        env = {**os.environ, "VESPER_DATA_DIR": data, "RUST_LOG": "warn"}
        tokens = os.path.join(data, "tokens")
        with open(tokens, "w") as f:
            subprocess.run([args.server_bin, "loadtest-accounts", "--count", str(speakers)], env=env, stdout=f, check=True, timeout=30)
        http, udp = free_port(socket.SOCK_STREAM), free_port(socket.SOCK_DGRAM)
        origin = f"http://127.0.0.1:{http}"
        env.update(VESPER_ORIGIN=origin, VESPER_HTTP_BIND=f"127.0.0.1:{http}", VESPER_UDP_BIND=f"127.0.0.1:{udp}")
        with open(os.path.join(data, "server.log"), "w") as log:
            server = subprocess.Popen(["nice", "-n", NICE, args.server_bin], env=env, stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 15
                while True:
                    try:
                        urllib.request.urlopen(f"{origin}/health", timeout=1).read()
                        break
                    except OSError:
                        if server.poll() is not None or time.monotonic() > deadline:
                            raise RuntimeError("server did not answer /health within 15 s")
                        time.sleep(0.2)
                out = os.path.join(data, "result.json")
                cmd = ["nice", "-n", NICE, args.loadtest_bin, "--url", f"ws://127.0.0.1:{http}/ws", "--origin", origin,
                       "--tokens", tokens, "--room-id", "1", "--speakers", str(speakers), "--seconds", str(seconds),
                       "--packet-ms", str(packet_ms), "--reconnects", str(reconnects), "--pid", str(server.pid), "--json", out]
                if sharers:
                    cmd += ["--sharers", str(sharers), "--share-mbps", "10"]
                # Setup can take 40 s (two 20 s gates), then the run, 2 s to drain and the resumes.
                ru0 = resource.getrusage(resource.RUSAGE_CHILDREN)
                b0, t_start = cpu_jiffies()[0], time.monotonic()
                p = subprocess.run(cmd, capture_output=True, text=True, timeout=seconds + 90)
                b1, wall = cpu_jiffies()[0], time.monotonic() - t_start
                ru1 = resource.getrusage(resource.RUSAGE_CHILDREN)
                if p.returncode == 2 or not os.path.exists(out):
                    raise RuntimeError(f"vesper-loadtest exited {p.returncode}: {p.stderr.strip()[-300:]}")
                with open(out) as f:
                    r = json.load(f)
                # Other work: everything busy on the machine minus the load test and the server.
                ours = (ru1.ru_utime + ru1.ru_stime) - (ru0.ru_utime + ru0.ru_stime)
                served = server_seconds(server.pid)
                r["other_cores"] = max(0.0, ((b1 - b0) / CLK_TCK - ours - served) / wall) if wall > 0 else None
                return r
            finally:
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()


def server_seconds(pid):
    """CPU seconds the server has used so far, from /proc/<pid>/stat."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            rest = f.read().rsplit(")", 1)[1].split()
        return (int(rest[11]) + int(rest[12])) / CLK_TCK
    except (OSError, IndexError, ValueError):
        return 0.0


def load_metrics(r):
    """Flat numbers from one load test summary."""
    cpu = r["server_cpu_percent"]
    fwd = r["forwarded_per_second"]
    return {
        "loss": r["loss_percent"],
        "worst_pair": r["worst_pair_percent"],
        "latency_p50": r["latency_ms"]["p50"],
        "latency_p99": r["latency_ms"]["p99"],
        "jitter_max": r["jitter_ms"]["max"],
        "cpu": cpu,
        "rss_peak": r["server_rss_mib"]["peak"],
        "rss_load": r["server_rss_mib"]["load"],
        "video_worst_pair": r.get("video_worst_pair_percent"),
        "forwarded_per_s": fwd,
        "us_per_packet": None if cpu is None or not fwd else cpu / 100 * 1e6 / fwd,
    }


def slope(xs, ys):
    """Least-squares slope of ys over xs; None with fewer than two points."""
    pts = [(x, y) for x, y in zip(xs, ys) if y is not None]
    if len(pts) < 2:
        return None
    mx = statistics.mean(p[0] for p in pts)
    my = statistics.mean(p[1] for p in pts)
    den = sum((p[0] - mx) ** 2 for p in pts)
    return sum((p[0] - mx) * (p[1] - my) for p in pts) / den if den else None


def build_summary(raw):
    """raw: {"load": {case: [run...]}, "throughput": {step: [run...]}, "dsp": {...} or None} to {metric id: summary}."""
    s = {}
    for case, runs in raw["load"].items():
        ms = [load_metrics(r) for r in runs]
        for key in ("loss", "worst_pair", "latency_p50", "latency_p99", "jitter_max", "cpu", "rss_peak"):
            s[f"{case}.{key}"] = summarize([m[key] for m in ms])
        if case.startswith("share-"):
            s[f"{case}.video_worst_pair"] = summarize([m["video_worst_pair"] for m in ms])
    # Join and resume: per repeat, the median over every speaker of the audio-only cases.
    audio = [c for c in raw["load"] if c.startswith("load-")]
    repeats = max((len(raw["load"][c]) for c in audio), default=0)
    for key in ("join_ms", "media_ms", "resume_ms"):
        per = []
        for i in range(repeats):
            vals = [v for c in audio if i < len(raw["load"][c]) for v in raw["load"][c][i].get(key, [])]
            per.append(statistics.median(vals) if vals else None)
        s[f"calls.{key}"] = summarize(per)
    # Per participant: RSS slope over room size, CPU per person and per forwarded packet at 12.
    rss, cpu12, us12 = [], [], []
    for i in range(repeats):
        pts = [(int(c.split("-")[1]), load_metrics(raw["load"][c][i])) for c in audio if i < len(raw["load"][c])]
        rss.append(slope([p[0] for p in pts], [p[1]["rss_load"] for p in pts]))
        at12 = [p[1] for p in pts if p[0] == MAX_ROOM]
        if at12:
            cpu12.append(None if at12[0]["cpu"] is None else at12[0]["cpu"] / MAX_ROOM)
            us12.append(at12[0]["us_per_packet"])
    s["scale.rss_per_participant"] = summarize(rss)
    s["scale.cpu_per_participant"] = summarize(cpu12)
    s["scale.us_per_packet"] = summarize(us12)
    for step, runs in raw.get("throughput", {}).items():
        ms = [load_metrics(r) for r in runs]
        for key in ("forwarded_per_s", "worst_pair", "cpu", "us_per_packet", "latency_p99"):
            s[f"throughput-{step}ms.{key}"] = summarize([m[key] for m in ms])
    others = [r.get("other_cores") for runs in list(raw["load"].values()) + list(raw.get("throughput", {}).values()) for r in runs]
    busy = summarize(others)
    if busy:
        s["machine.other_cores"] = busy
    dsp = raw.get("dsp")
    if dsp:
        s["deepfilternet.ms_per_frame"] = summarize(dsp["deepfilternet_ms_per_frame"])
        s["rnnoise.ms_per_frame"] = summarize(dsp["rnnoise_ms_per_frame"])
    return {k: v for k, v in s.items() if v is not None}


def run_dsp(args):
    model = os.path.join(ROOT, "crates/vesper-dsp/model/DeepFilterNet3_onnx.tar.gz")
    p = subprocess.run(["nice", "-n", NICE, args.dsp_bin, model, "5", str(max(args.repeats, 3))],
                       capture_output=True, text=True, timeout=300)
    if p.returncode != 0:
        raise RuntimeError(f"dsp_bench exited {p.returncode}: {p.stderr.strip()[-300:]}")
    return json.loads(p.stdout)


def run_all(args):
    raw = {"load": {}, "throughput": {}, "dsp": None}
    errors = []
    cases = [c for c in LOAD if not args.quick or c[0] in QUICK_LOAD]
    # Repeats go round the cases, so a slow minute on the machine spreads over all of them.
    for rep in range(args.repeats):
        for cid, speakers, sharers in cases:
            print(f"== {cid}, repeat {rep + 1}/{args.repeats}", file=sys.stderr)
            try:
                r = load_run(args, speakers, sharers, args.seconds, reconnects=0 if sharers else RECONNECTS)
                raw["load"].setdefault(cid, []).append(r)
            except (RuntimeError, subprocess.SubprocessError, OSError) as e:
                errors.append({"case": cid, "error": str(e)})
        if not args.quick:
            for ms in THROUGHPUT_STEPS_MS:
                print(f"== throughput {ms} ms, repeat {rep + 1}/{args.repeats}", file=sys.stderr)
                try:
                    r = load_run(args, THROUGHPUT_SPEAKERS, 0, THROUGHPUT_SECONDS, packet_ms=ms)
                    raw["throughput"].setdefault(str(ms), []).append(r)
                except (RuntimeError, subprocess.SubprocessError, OSError) as e:
                    errors.append({"case": f"throughput-{ms}ms", "error": str(e)})
    if not args.quick and not args.no_dsp:
        print("== denoisers", file=sys.stderr)
        try:
            raw["dsp"] = run_dsp(args)
        except (RuntimeError, subprocess.SubprocessError, OSError, ValueError) as e:
            errors.append({"case": "denoisers", "error": str(e)})
    return raw, errors


def notes(raw):
    n = []
    resumes = [r for runs in raw["load"].values() for r in runs]
    if resumes and not any(r.get("resume_supported") for r in resumes):
        n.append("This server has no `resume` (before v2.3.2), so drop to `resumed` is n/a.")
    errs = sorted({e for r in resumes for e in r.get("resume_errors", [])})
    if errs:
        n.append("Resume errors: " + "; ".join(errs[:5]))
    tp = raw.get("throughput", {})
    if tp:
        cpus = [r["server_cpu_percent"] or 0 for runs in tp.values() for r in runs]
        if cpus and max(cpus) < 90:
            n.append(f"Throughput: the server never passed {max(cpus):.0f}% of one core, so the load generator, not the SFU, set the ceiling. The rate tracks the tools; CPU per packet tracks the SFU.")
    return n


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--name", help="report name, e.g. v2.4.0 (default: git describe of the server tree)")
    ap.add_argument("--repeats", type=int, default=3, help="runs of each case, 1 to 5 (default 3)")
    ap.add_argument("--quick", action="store_true", help="patch smoke: 1 repeat of 3 load cases, no throughput or denoisers, not saved in docs/")
    ap.add_argument("--no-dsp", action="store_true", help="skip the denoiser bench")
    ap.add_argument("--server-bin", default=os.path.join(ROOT, "target/release/vesper-server"))
    ap.add_argument("--loadtest-bin", default=os.path.join(ROOT, "target/release/vesper-loadtest"))
    ap.add_argument("--dsp-bin", default=os.path.join(ROOT, "target/release/examples/dsp_bench"))
    ap.add_argument("--wait-quiet", type=int, default=600, metavar="S", help=f"wait up to S seconds for under {BUSY_CORES} cores of other work first (default 600)")
    ap.add_argument("--baseline", help="results JSON to compare against, or none (default: the last vX.Y.0 before --name)")
    ap.add_argument("--out", help="default: docs/benchmarks/<name>.md (target/bench/ with --quick)")
    ap.add_argument("--render", metavar="RESULTS.json", help="render saved results, run nothing")
    args = ap.parse_args()

    if args.render:
        with open(args.render) as f:
            doc = json.load(f)
    else:
        if args.quick:
            args.repeats = 1
        if not 1 <= args.repeats <= 5:
            sys.exit("--repeats must be 1 to 5")
        args.seconds = 5 if args.quick else SECONDS
        bins = [args.server_bin, args.loadtest_bin] + ([] if args.quick or args.no_dsp else [args.dsp_bin])
        for b in bins:
            if not os.access(b, os.X_OK):
                sys.exit(f"{b}: not found; `just bench` builds it")
        args.server_bin = os.path.abspath(args.server_bin)
        tree = os.path.dirname(args.server_bin)
        commit = git("rev-parse", "HEAD", cwd=tree)
        dsp_same = subprocess.run(["git", "diff", "--quiet", commit, "HEAD", "--", "crates/vesper-dsp"], cwd=ROOT).returncode == 0
        meta = {
            **machine(),
            "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M"),
            "describe": git("describe", "--tags", "--always", cwd=tree),
            "commit": commit[:12],
            "branch": git("rev-parse", "--abbrev-ref", "HEAD", cwd=tree).replace("HEAD", "(detached)"),
            "dirty": git("status", "--porcelain", "--untracked-files=no", cwd=tree) not in ("", "unknown"),
            "bench_commit": git("rev-parse", "--short=12", "HEAD"),
            "dsp_note": "is the same as the server's commit." if dsp_same else "differs from the server's commit: the denoiser rows measure the bench tools' copy.",
            "settings": f"{args.repeats} repeat{"s" if args.repeats > 1 else ""}; load cases {args.seconds} s, throughput steps {THROUGHPUT_SECONDS} s; nice {NICE}; loopback; quick {'yes' if args.quick else 'no'}",
        }
        meta["name"] = args.name or meta["describe"]
        print("== checking the machine is quiet", file=sys.stderr)
        meta["busy_at_start"] = round(wait_quiet(args.wait_quiet), 2)
        t0 = time.monotonic()
        raw, errors = run_all(args)
        meta["loadavg_end"] = " ".join(read("/proc/loadavg").split()[:3])
        meta["wall_s"] = round(time.monotonic() - t0)
        meta["settings"] += f"; took {meta['wall_s']} s"
        doc = {"meta": meta, "summary": build_summary(raw), "errors": errors, "notes": notes(raw), "raw": raw}

    name = doc["meta"]["name"]
    default_dir = os.path.join(ROOT, "target", "bench") if (not args.render and args.quick) else BENCH_DIR
    out = args.out or os.path.join(default_dir, name + ".md")
    if args.baseline == "none":
        base_path = None
    else:
        base_path = args.baseline or find_baseline(name, os.path.splitext(out)[0] + ".json")
    base_doc, base_name = None, None
    if base_path:
        with open(base_path) as f:
            base_doc = json.load(f)
        base_name = base_doc["meta"]["name"]
    md, ok = render(doc, base_doc, base_name)
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "w") as f:
        f.write(md)
    if not args.render:
        with open(os.path.splitext(out)[0] + ".json", "w") as f:
            json.dump(doc, f, indent=1)
            f.write("\n")
    print(md)
    print(f"wrote {out}", file=sys.stderr)
    if doc.get("errors"):
        sys.exit(2)
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
