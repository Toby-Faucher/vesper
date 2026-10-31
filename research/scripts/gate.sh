#!/usr/bin/env bash
# gate.sh <id>: preflight, build, unit tests, determinism, baseline and candidate eval on the gate
# split, paired SPRT and guards, then result.json and CATALOG.md. Controller only.
# One heavy step at a time, each under the controller's memory-capped scope.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

id=${1:-}
[[ $id =~ $ID_RE ]] || die "usage: gate.sh <id> (invalid id: '$id')"
exp="$EXP_DIR/$id"
wt="$WORKTREES/$id"
[[ -f $exp/baseline_commit ]] || die "no such experiment: '$id' (run attempt.sh first)"
[[ -d $wt ]] || die "worktree $wt not found"
[[ -z $(git -C "$wt" status --porcelain) ]] || die "worktree has uncommitted changes; commit them first"

rm -f "$exp"/{result.json,build.log,test.log,eval.log}

baseline=$(<"$exp/baseline_commit")
candidate=$(git -C "$wt" rev-parse HEAD)
b12=${baseline:0:12}
c12=${candidate:0:12}
cache="$REPO_ROOT/$(cfg paths.gate_cache)"
split=$(cfg eval.split)
threads=$(cfg machine.threads)
mkdir -p "$cache/bin"
export CARGO_TARGET_DIR="$REPO_ROOT/target"
export RAYON_NUM_THREADS=$threads CARGO_BUILD_JOBS=$threads

pconf="$exp/pipeline.toml"
[[ -f $pconf ]] || pconf=""
mapfile -t stack < <(python3 "$SCRIPTS/cfg.py" opt "stack.$id")
meta=(--meta "id=$id" --meta "branch=exp/$id" --meta "base_sha=$baseline" --meta "cand_sha=$candidate"
      --meta "config_hash=$(python3 "$SCRIPTS/cfg.py" hash)" --meta "pipeline=${pconf:+pipeline.toml}"
      --meta "split=$split" --meta "stack=${stack[*]}")

finish() {
  "$SCRIPTS/catalog.sh"
  echo "result: $exp/result.json"
}
broken() {
  python3 "$SCRIPTS/sprt.py" broken --reason "$1" --out "$exp/result.json" "${meta[@]}"
  finish; exit 0
}

# --- 1. Preflight: the attempt must descend from its base and touch only allowed paths. ---
[[ $baseline =~ ^[0-9a-f]{40}$ ]] || broken "baseline_commit is not a full commit sha"
git -C "$REPO_ROOT" merge-base --is-ancestor "$baseline" "$candidate" 2>/dev/null \
  || broken "ancestry: candidate does not descend from the base $b12"
bad=$(bad_paths "$id" "$baseline" "$candidate") || broken "could not diff $b12..$c12 for the path check"
[[ -z $bad ]] || broken "touches protected paths: $(tr '\n' ' ' <<<"$bad")"
weak=$(weakened_tests "$baseline" "$candidate") || broken "could not diff $b12..$c12 for the test check"
[[ -z $weak ]] || broken "weakens the unit tests: $(tr '\n' ' ' <<<"$weak")"

# build_bin DIR SHA: build audiolab in DIR and keep a copy named by commit.
build_bin() {
  local bin="$cache/bin/audiolab-${2:0:12}"
  if [[ ! -x $bin ]]; then
    (cd "$1" && heavy cargo build --release --locked -p vesper-audiolab) >>"$exp/build.log" 2>&1 || return 1
    cp "$CARGO_TARGET_DIR/release/audiolab" "$bin.tmp" && mv -f "$bin.tmp" "$bin"
  fi
  echo "$bin"
}
# UTMOS is scored when its model is there (reported, not gated). Part of the baseline cache key.
utmos="$REPO_ROOT/$(python3 "$SCRIPTS/cfg.py" opt paths.second_mos)"
utmos_args=()
# VESPER_GATE_NO_UTMOS=1 skips the (slow, report-only) second judge for quick experiment runs.
if [[ -f $utmos && -z ${VESPER_GATE_NO_UTMOS:-} ]]; then utmos_args=(--second-mos "$utmos"); fi
# run_eval BIN OUT ARGS...: score on the gate split; ARGS holds one or more --config FILE.
run_eval() {
  local bin=$1 out=$2
  shift 2
  (cd "$REPO_ROOT" && heavy "$bin" eval --split "$split" --out "$out" --jobs "$threads" \
     --model "$REPO_ROOT/$(cfg paths.model)" --dnsmos "$REPO_ROOT/$(cfg paths.dnsmos)" \
     --corpus "$REPO_ROOT/$(cfg paths.corpus)" "${utmos_args[@]}" "$@") >>"$exp/eval.log" 2>&1
}

# --- 2. Build and unit tests (candidate). The candidate binary is always rebuilt from its tree. ---
rm -f "$cache/bin/audiolab-$c12"
cand_bin=$(build_bin "$wt" "$candidate") || broken "build failed (see build.log)"
(cd "$wt" && heavy cargo test --locked --release -p vesper-dsp --features tract) >"$exp/test.log" 2>&1 \
  || broken "cargo test -p vesper-dsp failed (see test.log)"

# --- 3. Configs. The default comes from the BASE commit, so stacked config changes move with it. ---
default_conf="$cache/default-$b12.toml"
git -C "$REPO_ROOT" show "$baseline:$(cfg paths.default_config)" >"$default_conf" \
  || broken "no $(cfg paths.default_config) at base $b12"
# A stack ([stack] in config.toml) merges the listed experiments' pipeline.toml, then this one's.
cand_confs=()
for s in "${stack[@]}"; do
  [[ $s =~ $ID_RE ]] || broken "stack entry '$s' is not an experiment id"
  f="$cache/stack-$id-$s.toml"
  # The controller checkout first (config-only results are committed there), else the exp branch.
  if [[ -f $EXP_DIR/$s/pipeline.toml ]]; then
    cp "$EXP_DIR/$s/pipeline.toml" "$f"
  else
    git -C "$REPO_ROOT" show "exp/${s}:research/experiments/${s}/pipeline.toml" >"$f" 2>/dev/null \
      || broken "stack: no pipeline.toml for $s (checkout or exp/$s)"
  fi
  cand_confs+=(--config "$f")
done
if [[ -n $pconf ]]; then cand_confs+=(--config "$pconf"); fi
((${#cand_confs[@]})) || cand_confs=(--config "$default_conf")
dh=$(sha256sum "$default_conf" | cut -c1-12)

# --- 4. Baseline eval, cached per base commit and default config. ---
corpus_tag=$(sha256sum "$REPO_ROOT/$(cfg paths.corpus)/$split/manifest.json" | cut -c1-8)
base_json="$cache/base-$b12-$dh-$corpus_tag${utmos_args:+-utmos}.json"
if [[ ! -f $base_json ]]; then
  bwt="$WORKTREES/.base-$b12"
  [[ -d $bwt ]] || git -C "$REPO_ROOT" worktree add -q --detach "$bwt" "$baseline"
  base_bin=$(build_bin "$bwt" "$baseline") || broken "baseline build failed (see build.log)"
  git -C "$REPO_ROOT" worktree remove --force "$bwt"
  run_eval "$base_bin" "$base_json.tmp" --config "$default_conf" || broken "baseline eval failed (see eval.log)"
  mv -f "$base_json.tmp" "$base_json"
fi

# --- 5. Candidate eval on the whole split, then a short second run for the determinism check. ---
cand_json="$cache/cand-$id-$c12.json"
det_json="$cache/det-$id-$c12.json"
run_eval "$cand_bin" "$cand_json" "${cand_confs[@]}" || broken "candidate eval failed (see eval.log)"
run_eval "$cand_bin" "$det_json" "${cand_confs[@]}" --limit "$(cfg eval.determinism_items)" \
  || broken "determinism eval failed (see eval.log)"

# --- 6. Paired SPRT (replayed in manifest order) and guards. ---
meta+=(--meta "base_eval=${base_json#"$REPO_ROOT"/}" --meta "cand_eval=${cand_json#"$REPO_ROOT"/}")
python3 "$SCRIPTS/sprt.py" gate --manifest "$REPO_ROOT/$(cfg paths.corpus)/$split/manifest.json" \
  --base "$base_json" --cand "$cand_json" --det "$det_json" --hypothesis "$exp/hypothesis.md" \
  --out "$exp/result.json" "${meta[@]}" || broken "sprt.py failed"
finish
