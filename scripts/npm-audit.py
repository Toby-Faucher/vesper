#!/usr/bin/env python3
"""Usage: scripts/npm-audit.py [REPORT.json]

The CI gate on npm advisories in web/. Runs `npm audit --json` in web/ (or reads REPORT.json) and exits 1
on any high or critical advisory not listed in ALLOWED. Moderate and low are reported, not blocking.
An ALLOWED entry that no longer appears also fails, so the exception is removed once the fix lands.
Exit 2 when the report cannot be read.
"""
import json
import subprocess
import sys
from pathlib import Path

# Advisory id: why it is allowed, and when to look again.
ALLOWED = {}
BLOCKING = {"high", "critical"}


def load(argv):
    if len(argv) > 1:
        return json.loads(Path(argv[1]).read_text())
    web = Path(__file__).resolve().parent.parent / "web"
    # npm audit exits non-zero whenever it finds anything; the JSON is what matters.
    out = subprocess.run(["npm", "audit", "--json"], cwd=web, capture_output=True, text=True).stdout
    return json.loads(out)


def main(argv):
    try:
        report = load(argv)
        found = {}
        for vuln in report.get("vulnerabilities", {}).values():
            for via in vuln["via"]:
                if isinstance(via, dict):
                    found[via["url"].rsplit("/", 1)[-1]] = (via["severity"], via.get("title", ""))
    except (OSError, ValueError, KeyError, TypeError, AttributeError) as e:
        print(f"npm-audit: cannot read the npm audit report: {e}", file=sys.stderr)
        return 2
    bad = 0
    for gid, (sev, title) in sorted(found.items()):
        if gid in ALLOWED:
            print(f"allowed  {gid} ({sev}): {ALLOWED[gid]}")
        elif sev in BLOCKING:
            print(f"BLOCKING {gid} ({sev}): {title}")
            bad += 1
        else:
            print(f"reported {gid} ({sev}): {title}")
    for gid in sorted(set(ALLOWED) - set(found)):
        print(f"STALE    {gid} no longer appears: remove it from ALLOWED in scripts/npm-audit.py")
        bad += 1
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
