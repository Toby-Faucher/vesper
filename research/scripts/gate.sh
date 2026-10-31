#!/usr/bin/env bash
# gate.sh [--dry-run] <id>: preflight, build, unit tests, determinism, baseline and candidate eval on
# the gate split, paired SPRT and guards, then result.json and CATALOG.md. Controller only.
# One heavy step at a time, each under the controller's memory-capped scope.
# --dry-run: preflight, then print the audiolab eval commands; it builds, tests and records nothing.
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

dry=0
if [[ ${1:-} == --dry-run ]]; then dry=1; shift; fi
id=${1:-}
[[ $id =~ $ID_RE ]] || die "usage: gate.sh [--dry-run] <id> (invalid id: '$id')"
exp="$EXP_DIR/$id"
wt="$WORKTREES/$id"
[[ -f $exp/baseline_commit ]] || die "no such experiment: '$id' (run attempt.sh first)"
[[ -d $wt ]] || die "worktree $wt not found"
[[ -z $(git -C "$wt" status --porcelain) ]] || die "worktree has uncommitted changes; commit them first"

if (( ! dry )); then rm -f "$exp"/{result.json,build.log,test.log,eval.log}; fi

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
  if (( dry )); then echo "broken: $1" >&2; exit 1; fi
  python3 "$SCRIPTS/sprt.py" broken --reason "$1" --out "$exp/result.json" "${meta[@]}"
  finish; exit 0
}

# Eval options: an eval.toml in the experiment folder may set another model and an Opus stage.
etoml="$exp/eval.toml"
[[ -f $etoml ]] || etoml=""
opts=$(python3 "$SCRIPTS/cfg.py" evalopts ${etoml:+"$etoml"} 2>&1) || broken "$opts"
mapfile -t o <<<"$opts"
base_model=${o[0]} base_opus=${o[1]} cand_model=${o[2]} cand_opus=${o[3]} allow_bitrate=${o[4]}
meta+=(--meta "eval=${etoml:+eval.toml}" --meta "opus=$cand_opus")
if [[ $cand_model != "$base_model" ]]; then meta+=(--meta "model=$cand_model"); fi

# --- 1. Preflight: the attempt must descend from its base and touch only allowed paths. ---
[[ $baseline =~ ^[0-9a-f]{40}$ ]] || broken "baseline_commit is not a full commit sha"
git -C "$REPO_ROOT" merge-base --is-ancestor "$baseline" "$candidate" 2>/dev/null \
  || broken "ancestry: candidate does not descend from the base $b12"
bad=$(bad_paths "$id" "$baseline" "$candidate") || broken "could not diff $b12..$c12 for the path check"
[[ -z $bad ]] || broken "touches protected paths: $(tr '\n' ' ' <<<"$bad")"
weak=$(weakened_tests "$baseline" "$candidate") || broken "could not diff $b12..$c12 for the test check"
[[ -z $weak ]] || broken "weakens the unit tests: $(tr '\n' ' ' <<<"$weak")"
for m in "$base_model" "$cand_model"; do
  [[ -f $REPO_ROOT/$m ]] || broken "model not found: $m (see research/scripts/fetch-model*.sh)"
done

# build_bin DIR SHA: build audiolab in DIR and keep a copy named by commit.
build_bin() {
  local bin="$cache/bin/audiolab-${2:0:12}"
  if (( dry )); then
    [[ -x $bin ]] || echo "# would build $bin from $1" >&2
  elif [[ ! -x $bin ]]; then
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
# run_eval BIN OUT MODEL OPUS ARGS...: score on the gate split with MODEL (repo-relative), through
# Opus when OPUS is set; ARGS holds one or more --config FILE. --dry-run prints the command instead.
run_eval() {
  local bin=$1 out=$2 model=$3 opus=$4
  shift 4
  local cmd=("$bin" eval --split "$split" --out "$out" --jobs "$threads"
             --model "$REPO_ROOT/$model" --dnsmos "$REPO_ROOT/$(cfg paths.dnsmos)"
             --corpus "$REPO_ROOT/$(cfg paths.corpus)" "${utmos_args[@]}" ${opus:+--opus "$opus"} "$@")
  if (( dry )); then printf '%q ' "${cmd[@]}"; echo; return 0; fi
  (cd "$REPO_ROOT" && heavy "${cmd[@]}") >>"$exp/eval.log" 2>&1
}

# --- 2. Build and unit tests (candidate). The candidate binary is always rebuilt from its tree. ---
if (( ! dry )); then rm -f "$cache/bin/audiolab-$c12"; fi
cand_bin=$(build_bin "$wt" "$candidate") || broken "build failed (see build.log)"
if (( ! dry )); then
  (cd "$wt" && heavy cargo test --locked --release -p vesper-dsp --features tract) >"$exp/test.log" 2>&1 \
    || broken "cargo test -p vesper-dsp failed (see test.log)"
fi

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

# --- 4. Baseline eval, cached per base commit, default config and eval options. ---
corpus_tag=$(sha256sum "$REPO_ROOT/$(cfg paths.corpus)/$split/manifest.json" | cut -c1-8)
# The shipped model without Opus keeps the old cache name, so existing baselines stay valid.
opt_tag=""
if [[ -n $base_opus || $base_model != "$(cfg paths.model)" ]]; then
  opt_tag=-$({ sha256sum "$REPO_ROOT/$base_model" | cut -c1-64; echo "$base_opus"; } | sha256sum | cut -c1-8)
fi
base_json="$cache/base-$b12-$dh-$corpus_tag$opt_tag${utmos_args:+-utmos}.json"
if (( dry )); then
  if [[ -f $base_json ]]; then echo "# baseline cached, not run:"; fi
  run_eval "$cache/bin/audiolab-$b12" "$base_json.tmp" "$base_model" "$base_opus" --config "$default_conf"
elif [[ ! -f $base_json ]]; then
  bwt="$WORKTREES/.base-$b12"
  [[ -d $bwt ]] || git -C "$REPO_ROOT" worktree add -q --detach "$bwt" "$baseline"
  base_bin=$(build_bin "$bwt" "$baseline") || broken "baseline build failed (see build.log)"
  git -C "$REPO_ROOT" worktree remove --force "$bwt"
  run_eval "$base_bin" "$base_json.tmp" "$base_model" "$base_opus" --config "$default_conf" || broken "baseline eval failed (see eval.log)"
  mv -f "$base_json.tmp" "$base_json"
fi

# --- 5. Candidate eval on the whole split, then a short second run for the determinism check. ---
cand_json="$cache/cand-$id-$c12.json"
det_json="$cache/det-$id-$c12.json"
run_eval "$cand_bin" "$cand_json" "$cand_model" "$cand_opus" "${cand_confs[@]}" \
  || broken "candidate eval failed (see eval.log)"
run_eval "$cand_bin" "$det_json" "$cand_model" "$cand_opus" "${cand_confs[@]}" --limit "$(cfg eval.determinism_items)" \
  || broken "determinism eval failed (see eval.log)"

if (( dry )); then exit 0; fi

# --- 6. Paired SPRT (replayed in manifest order) and guards. ---
allow=()
if [[ $allow_bitrate == 1 ]]; then allow=(--allow-more-bitrate); fi
meta+=(--meta "base_eval=${base_json#"$REPO_ROOT"/}" --meta "cand_eval=${cand_json#"$REPO_ROOT"/}")
python3 "$SCRIPTS/sprt.py" gate --manifest "$REPO_ROOT/$(cfg paths.corpus)/$split/manifest.json" \
  --base "$base_json" --cand "$cand_json" --det "$det_json" --hypothesis "$exp/hypothesis.md" \
  --out "$exp/result.json" "${allow[@]}" "${meta[@]}" || broken "sprt.py failed"
finish
