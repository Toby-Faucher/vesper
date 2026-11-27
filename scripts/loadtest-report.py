#!/usr/bin/env python3
"""Release load test: runs vesper-loadtest at a few room sizes and writes a Markdown report.

Each scenario gets its own throwaway server (fresh data dir, free loopback ports), started with
`nice`, and the load test runs for a capped time. The thresholds below decide PASS; why each one
is what it is: docs/testing/loadtest.md. `just loadtest-report` builds and runs this.

  scripts/loadtest-report.py [--sizes 4,8,12] [--seconds 10] [--no-video]
      [--server-bin PATH] [--loadtest-bin PATH] [--name vX.Y.Z | --out FILE]
  scripts/loadtest-report.py --render RESULTS.json [--out FILE]   (no server; for tests)

--server-bin can point at a vesper-server built in another worktree, so this branch's load test
measures any branch. The report names the commit of the tree that binary was built in.
Exit 0 if every scenario passes, 1 if one fails, 2 on a setup error.
"""

import argparse
import datetime
import json
import os
import platform
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MAX_SECONDS = 60
# The server's fixed room cap (`max_room_size` in crates/vesper-server/src/config.rs).
MAX_ROOM = 12
# Video scenarios: (speakers, sharers), each share at 10 Mbps. 2 sharers is the usual load; 4 is the
# room's share budget (MAX_ROOM_SHARE_KBPS, 120 000 kbps counted per viewer).
VIDEO = [(4, 2), (4, 4)]
NICE = "10"

# Applied to every scenario. docs/testing/loadtest.md says why each number is what it is.
THRESHOLDS = {
    "worst_pair_percent": ("min", 99.0, "Audio delivered, worst receiver-sender pair", "%"),
    "latency_p99": ("max", 15.0, "Forwarding latency p99", "ms"),
    "jitter_max": ("max", 5.0, "Jitter (RFC 3550), worst pair", "ms"),
    "cpu": ("max", 50.0, "Server CPU, share of one core", "%"),
    "rss_peak": ("max", 128.0, "Server memory, peak RSS", "MiB"),
}


def metrics(r):
    """The numbers THRESHOLDS reads, from one vesper-loadtest --json summary."""
    return {
        "worst_pair_percent": r["worst_pair_percent"],
        "latency_p99": r["latency_ms"]["p99"],
        "jitter_max": r["jitter_ms"]["max"],
        "cpu": r["server_cpu_percent"],
        "rss_peak": r["server_rss_mib"]["peak"],
    }


def verdict(r):
    """(pass, list of failed check names). A missing number fails its check."""
    failed = [] if r.get("pass_95") else ["vesper-loadtest 95% rule"]
    for key, value in metrics(r).items():
        kind, limit, name, _ = THRESHOLDS[key]
        if value is None or (value < limit if kind == "min" else value > limit):
            failed.append(name)
    return (not failed, failed)


def fmt(v, digits=1):
    return "n/a" if v is None else f"{v:.{digits}f}"


def scenario_name(r):
    if r["sharers"]:
        return f"{r['speakers']} speakers, {r['sharers']} sharers x {r['share_mbps']} Mbps"
    return f"{r['speakers']} speakers"


def render(doc):
    m, results = doc["meta"], doc["results"]
    all_pass = all(verdict(r)[0] for r in results) and not doc.get("errors")
    out = [
        f"# Load test report: {m['describe']}",
        "",
        f"Result: **{'PASS' if all_pass else 'FAIL'}**. Made by `just loadtest-report` "
        "([how to read it](../loadtest.md)).",
        "",
        "| | |",
        "|---|---|",
        f"| Date (UTC) | {m['date']} |",
        f"| Server commit | `{m['commit']}`{' (uncommitted changes)' if m['dirty'] else ''}, branch `{m['branch']}` |",
        f"| Load test commit | `{m['loadtest_commit']}` |",
        f"| Machine | {m['host']}: {m['cpu']}, {m['threads']} threads, {m['mem_gib']} GiB, Linux {m['kernel']} |",
        f"| Load average at start | {m['loadavg']} |",
        f"| Each scenario | {m['seconds']} s of audio on loopback, server and load test at nice {NICE} |",
        "",
        "## Results",
        "",
        "| Scenario | Loss | Worst pair | Latency p50 / p95 / p99 / max | Jitter mean / max "
        "| Server CPU | RSS idle / load / peak | Video worst pair | Result |",
        "|---|---|---|---|---|---|---|---|---|",
    ]
    for r in results:
        ok, failed = verdict(r)
        lat, jit, rss = r["latency_ms"], r["jitter_ms"], r["server_rss_mib"]
        video = (
            "n/a"
            if r.get("video_worst_pair_percent") is None
            else f"{r['video_worst_pair_percent']:.1f}%"
        )
        out.append(
            f"| {scenario_name(r)} | {r['loss_percent']:.2f}% | {r['worst_pair_percent']:.1f}% "
            f"| {lat['p50']:.2f} / {lat['p95']:.2f} / {lat['p99']:.2f} / {lat['max']:.2f} ms "
            f"| {jit['mean']:.2f} / {jit['max']:.2f} ms | {fmt(r['server_cpu_percent'])}% "
            f"| {fmt(rss['idle'])} / {fmt(rss['load'])} / {fmt(rss['peak'])} MiB | {video} "
            f"| {'PASS' if ok else 'FAIL: ' + ', '.join(failed)} |"
        )
    for e in doc.get("errors", []):
        out.append(f"| {e['scenario']} | | | | | | | | FAIL: {e['error']} |")
    out += ["", "## Thresholds", "", "Every scenario must meet all of these.", ""]
    out.append("| Check | Limit |")
    out.append("|---|---|")
    out.append("| vesper-loadtest's own rule | 95% of packets in aggregate and from each peer |")
    for kind, limit, name, unit in THRESHOLDS.values():
        out.append(f"| {name} | {'at least' if kind == 'min' else 'at most'} {limit:g} {unit} |")
    out += [
        "",
        "Video delivery is shown but does not decide the result (see the doc). Server and load test run on "
        "one machine, so the CPU figure is an upper bound and the latency includes the load test's own "
        "scheduling. Numbers from different machines are not comparable.",
        "",
    ]
    return "\n".join(out)


def git(*args, cwd=ROOT):
    try:
        return subprocess.run(
            ["git", *args], cwd=cwd, capture_output=True, text=True, check=True
        ).stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def machine():
    cpu = "unknown"
    try:
        for line in open("/proc/cpuinfo"):
            if line.startswith("model name"):
                cpu = line.split(":", 1)[1].strip()
                break
        mem_kb = next(
            int(l.split()[1]) for l in open("/proc/meminfo") if l.startswith("MemTotal:")
        )
        loadavg = " ".join(open("/proc/loadavg").read().split()[:3])
    except OSError:
        mem_kb, loadavg = 0, "unknown"
    return {
        "host": platform.node() or "unknown",
        "cpu": cpu,
        "threads": os.cpu_count(),
        "mem_gib": round(mem_kb / 1024 / 1024),
        "kernel": platform.release(),
        "loadavg": loadavg,
    }


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def free_udp_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def run_scenario(args, speakers, sharers):
    """Start a fresh server, run the load test once, return its JSON summary. Raises on setup errors."""
    with tempfile.TemporaryDirectory(prefix="vesper-loadtest-") as data:
        env = {**os.environ, "VESPER_DATA_DIR": data, "RUST_LOG": "warn"}
        tokens = os.path.join(data, "tokens")
        with open(tokens, "w") as f:
            subprocess.run(
                [args.server_bin, "loadtest-accounts", "--count", str(speakers)],
                env=env, stdout=f, check=True,
            )
        http, udp = free_port(), free_udp_port()
        origin = f"http://127.0.0.1:{http}"
        env.update(
            VESPER_ORIGIN=origin,
            VESPER_HTTP_BIND=f"127.0.0.1:{http}",
            VESPER_UDP_BIND=f"127.0.0.1:{udp}",
        )
        log = open(os.path.join(data, "server.log"), "w")
        server = subprocess.Popen(
            ["nice", "-n", NICE, args.server_bin], env=env, stdout=log, stderr=log
        )
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
            cmd = [
                "nice", "-n", NICE, args.loadtest_bin,
                "--url", f"ws://127.0.0.1:{http}/ws", "--origin", origin,
                "--tokens", tokens, "--room-id", "1",
                "--speakers", str(speakers), "--seconds", str(args.seconds),
                "--pid", str(server.pid), "--json", out,
            ]
            if sharers:
                cmd += ["--sharers", str(sharers), "--share-mbps", "10"]
            # Setup can take up to 40 s (two 20 s readiness gates), then the run and 2 s to drain.
            p = subprocess.run(cmd, capture_output=True, text=True, timeout=args.seconds + 60)
            sys.stderr.write(p.stdout)
            if p.returncode == 2 or not os.path.exists(out):
                raise RuntimeError(f"vesper-loadtest exited {p.returncode}: {p.stderr.strip()}")
            with open(out) as f:
                return json.load(f)
        finally:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()
            log.close()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--sizes", default="4,8,12")
    ap.add_argument("--seconds", type=int, default=10)
    ap.add_argument("--no-video", action="store_true", help="skip the screen share scenarios")
    ap.add_argument("--server-bin", default=os.path.join(ROOT, "target/release/vesper-server"))
    ap.add_argument("--loadtest-bin", default=os.path.join(ROOT, "target/release/vesper-loadtest"))
    ap.add_argument("--name", help="report name, e.g. v2.4.0: writes docs/testing/loadtest/<name>.md")
    ap.add_argument("--out", help="default: docs/testing/loadtest/<name, or git describe of the server tree>.md")
    ap.add_argument("--render", metavar="RESULTS.json", help="render a saved results file, run nothing")
    args = ap.parse_args()

    if args.render:
        with open(args.render) as f:
            doc = json.load(f)
    else:
        if not 1 <= args.seconds <= MAX_SECONDS:
            sys.exit(f"--seconds must be 1 to {MAX_SECONDS}")
        try:
            sizes = [int(s) for s in args.sizes.split(",") if s]
        except ValueError:
            sys.exit("--sizes takes numbers like 4,8,16,32")
        if not sizes or min(sizes) < 2 or max(sizes) > MAX_ROOM:
            sys.exit(f"--sizes must be 2 to {MAX_ROOM} each (the server's room cap)")
        for b in (args.server_bin, args.loadtest_bin):
            if not os.access(b, os.X_OK):
                sys.exit(f"{b}: not found; build with `just loadtest-report` or cargo build --release")
        args.server_bin = os.path.abspath(args.server_bin)
        tree = os.path.dirname(args.server_bin)
        status = git("status", "--porcelain", "--untracked-files=no", cwd=tree)
        doc = {
            "meta": {
                **machine(),
                "date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M"),
                "describe": args.name or git("describe", "--tags", "--always", cwd=tree),
                "commit": git("rev-parse", "--short=12", "HEAD", cwd=tree),
                "branch": git("rev-parse", "--abbrev-ref", "HEAD", cwd=tree).replace("HEAD", "(detached)"),
                "dirty": status not in ("", "unknown"),
                "loadtest_commit": git("rev-parse", "--short=12", "HEAD"),
                "seconds": args.seconds,
            },
            "results": [],
            "errors": [],
        }
        plan = [(n, 0) for n in sizes] + ([] if args.no_video else VIDEO)
        for speakers, sharers in plan:
            label = f"{speakers} speakers" + (f", {sharers} sharers" if sharers else "")
            print(f"== {label}", file=sys.stderr)
            try:
                doc["results"].append(run_scenario(args, speakers, sharers))
            except (RuntimeError, subprocess.SubprocessError, OSError) as e:
                print(f"   setup error: {e}", file=sys.stderr)
                doc["errors"].append({"scenario": label, "error": str(e)})

    md = render(doc)
    out = args.out or os.path.join(ROOT, "docs/testing/loadtest", doc["meta"]["describe"] + ".md")
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "w") as f:
        f.write(md)
    if not args.render:
        with open(os.path.splitext(out)[0] + ".json", "w") as f:
            json.dump(doc, f, indent=2)
            f.write("\n")
    print(md)
    print(f"wrote {out}", file=sys.stderr)
    if doc.get("errors"):
        sys.exit(2)
    sys.exit(0 if all(verdict(r)[0] for r in doc["results"]) else 1)


if __name__ == "__main__":
    main()
