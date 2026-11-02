#!/usr/bin/env bash
# Bad network for a local two-browser call: tc netem on the media packets going INTO the SFU only.
# Prints the tc commands. Only --apply runs them, through sudoq (root, Toby approves). Recipe: docs/testing/bad-network.md.
#   scripts/netem-call.sh <profile> [--dev lo] [--port 40000] [--apply]
#   scripts/netem-call.sh off [--apply]          remove it again
# Profiles: loss5 loss10 burst5 burst10 jitter delay wifi off.
set -euo pipefail

usage() { sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//'; }

# Gilbert-Elliott for netem's `loss gemodel p r 1-h 1-k`: always lost in the bad state, never in the good one.
# Mean burst B packets means r = 1/B; a long-run loss L means p = L*r/(1-L). Same model as the audio lab's loss_pattern.
ge() { awk -v l="$1" -v b="$2" 'BEGIN { r = 1 / b; printf "loss gemodel %.3f%% %.3f%% 100%% 0%%", 100 * l * r / (1 - l), 100 * r }'; }

netem_args() {
  case "$1" in
    loss5) echo "loss 5%" ;;
    loss10) echo "loss 10%" ;;
    burst5) ge 0.05 3 ;;  # 5 % loss in bursts of 3 packets on average
    burst10) ge 0.10 5 ;; # 10 % loss in bursts of 5
    # netem reorders packets when the jitter is larger than the gap between them (20 ms): that is part of the test.
    jitter) echo "delay 40ms 20ms" ;;
    delay) echo "delay 150ms" ;;
    wifi) echo "delay 30ms 15ms $(ge 0.05 3)" ;;
    *) return 1 ;;
  esac
}

profile="" dev=lo port=40000 apply=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dev) dev=${2:?--dev needs a device}; shift ;;
    --port) port=${2:?--port needs a number}; shift ;;
    --apply) apply=1 ;;
    -h | --help) usage; exit 0 ;;
    *) [ -z "$profile" ] || { usage >&2; exit 2; }; profile=$1 ;;
  esac
  shift
done
[ -n "$profile" ] || { usage >&2; exit 2; }
[[ $port =~ ^[0-9]{1,5}$ ]] && [ "$port" -ge 1 ] && [ "$port" -le 65535 ] || { echo "bad port: $port" >&2; exit 2; }
# Only loopback or a test veth: never the interface the machine is reached through (bond0 on bawn).
[[ $dev = lo || $dev =~ ^veth[A-Za-z0-9._-]*$ ]] || { echo "refused device: $dev (lo or veth* only)" >&2; exit 2; }

cmds=()
if [ "$profile" = off ]; then
  cmds+=("tc qdisc del dev $dev root")
else
  args=$(netem_args "$profile") || { echo "unknown profile: $profile" >&2; usage >&2; exit 2; }
  # A prio qdisc with a 4th band that only the filter fills: everything else on $dev (SSH, the WebSocket) is untouched.
  cmds+=(
    "tc qdisc add dev $dev root handle 1: prio bands 4"
    "tc qdisc add dev $dev parent 1:4 handle 40: netem $args"
    "tc filter add dev $dev parent 1: protocol ip prio 1 u32 match ip protocol 17 0xff match ip dport $port 0xffff flowid 1:4"
  )
fi

if [ "$apply" = 0 ]; then
  printf '%s\n' "${cmds[@]}"
  echo "# printed only; add --apply to run them through sudoq. Check with: tc -s qdisc show dev $dev" >&2
  exit 0
fi
command -v sudoq >/dev/null || { echo "sudoq not found: run the printed commands as root yourself" >&2; exit 1; }
script=$(printf '%s && ' "${cmds[@]}")
sudoq run --sh --why "netem-call.sh $profile on $dev (UDP $port into the SFU), for a bad network call test" -- "${script% && }"
tc -s qdisc show dev "$dev"
