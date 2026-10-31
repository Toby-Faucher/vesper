#!/usr/bin/env bash
# attempt.sh <id> [--base <ref>]: create .worktrees/<id> on branch exp/<id> and the experiment
# folder, then print the path of the filled-in agent prompt.
# Default base: the integration branch, created from the source branch tip if it does not exist.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

id=${1:-}
[[ $id =~ $ID_RE ]] || die "usage: attempt.sh <id> [--base <ref>]  (id like E1-input-gain or E-null)"
shift
base_ref=""
if [[ ${1:-} == --base ]]; then base_ref=${2:?--base needs a ref}; shift 2; fi
[[ $# -eq 0 ]] || die "unexpected argument: $1"
exp="$EXP_DIR/$id"
wt="$WORKTREES/$id"
[[ ! -e $exp/baseline_commit ]] || die "experiment $id already exists"
git -C "$REPO_ROOT" rev-parse -q --verify "refs/heads/exp/$id" >/dev/null && die "branch exp/$id already exists"

if [[ -z $base_ref ]]; then
  base_ref=$(cfg repo.integration_branch)
  if ! git -C "$REPO_ROOT" rev-parse -q --verify "refs/heads/$base_ref" >/dev/null; then
    src=$(git -C "$REPO_ROOT" rev-parse "$(cfg repo.source_branch)")
    git -C "$REPO_ROOT" branch "$base_ref" "$src"
    # Record the original baseline once, for the final holdout check.
    sed -i "s|^origin_sha = \"\"|origin_sha = \"$src\"|" "$RESEARCH_DIR/config.toml"
    echo "created $base_ref at $src (recorded as repo.origin_sha)" >&2
  fi
fi
baseline=$(git -C "$REPO_ROOT" rev-parse --verify "$base_ref^{commit}") || die "no such ref: $base_ref"

mkdir -p "$exp" "$WORKTREES"
git -C "$REPO_ROOT" worktree add -q -b "exp/$id" "$wt" "$baseline"
# The model is git-ignored; vesper-dsp's tests read it from the crate folder.
model=$(cfg paths.model)
mkdir -p "$wt/$(dirname "$model")"
cp "$REPO_ROOT/$model" "$wt/$model"
echo "$baseline" > "$exp/baseline_commit"
[[ -e $exp/hypothesis.md ]] || cat > "$exp/hypothesis.md" <<'TPL'
# <one-line hypothesis>

## Change
What you will change: a PipelineConfig field, a new stage, or only pipeline.toml.

## Mechanism
Why it should raise DNSMOS OVRL without hurting SIG or SI-SDR.

## Expected effect
Which items gain (noise type, SNR, level) and roughly by how much.

## Risk
What could get worse.

Added latency (samples): 0
TPL

"$SCRIPTS/catalog.sh" >/dev/null

prompt="$exp/prompt.md"
sed -e "s|{{ID}}|$id|g" \
    -e "s|{{WORKTREE}}|$wt|g" \
    -e "s|{{EXPERIMENT}}|$exp|g" \
    -e "s|{{REPO}}|$REPO_ROOT|g" \
    -e "s|{{RESEARCH}}|$RESEARCH_DIR|g" \
    -e "s|{{CAP}}|$(cfg machine.memory_cap_mib)|g" \
    -e "s|{{THREADS}}|$(cfg machine.threads)|g" \
    "$RESEARCH_DIR/agent-prompt.md" > "$prompt"
echo "$prompt"
