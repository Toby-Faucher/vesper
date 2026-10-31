#!/usr/bin/env bash
# The check list, in order; stops at the first failure. CI jobs call subsets of these steps, so
# local and CI run the same commands. Cargo and npm run under the memory cap (scripts/capped.sh);
# VESPER_NO_CAP=1 runs them plain (CI runners).
set -euo pipefail

all=(fmt clippy test bindings deny secrets scripts shell web)
usage() {
  cat <<EOF
Usage: scripts/check.sh [step]...
Runs the named steps in the order given. No step runs them all: ${all[*]}.
Steps:
  fmt       cargo fmt --check
  clippy    clippy -D warnings
  test      nextest, ci profile (the SFU ICE-timeout test alone takes about 20 s)
  bindings  fail if the committed ts-rs bindings in web/src/lib/gen differ from the Rust types
  deny      cargo deny check (advisories need network)
  secrets   scripts/check-secrets.sh
  scripts   every scripts/test-*.sh
  shell     shellcheck scripts/*.sh (skipped with a note if shellcheck is missing)
  web       npm ci, svelte-check, vitest, vite build in web/
  dsp       not in the full run: fetches the model, runs the denoiser tests and builds the wasm
            (scripts/build-dsp.sh; needs the wasm-bindgen CLI at the Cargo.lock version)
  fast      fmt clippy test
EOF
}

steps=()
for a in "$@"; do
  case $a in
    -h | --help) usage; exit 0 ;;
    fast) steps+=(fmt clippy test) ;;
    fmt | clippy | test | bindings | deny | secrets | scripts | shell | web | dsp) steps+=("$a") ;;
    *) echo "unknown step: $a" >&2; usage >&2; exit 2 ;;
  esac
done
[ ${#steps[@]} -gt 0 ] || steps=("${all[@]}")

cd "$(dirname "$0")/.."
export PATH=$HOME/.cargo/bin:$PATH
# shellcheck source=scripts/capped.sh
. scripts/capped.sh
cap_check || exit 1
step() { echo; echo "== $*"; }
gen=web/src/lib/gen

run_step() {
  case $1 in
    fmt) step "cargo fmt --check"; capped cargo fmt --all --check ;;
    clippy) step "clippy"; capped cargo clippy --locked --workspace --all-targets -- -D warnings ;;
    # No doc tests exist yet; nextest skips them, so add `cargo test --doc --workspace` with the first.
    test) step "nextest"; capped cargo nextest run --locked --workspace --profile ci ;;
    bindings)
      step "ts-rs bindings up to date"
      # The vesper-protocol export tests rewrite web/src/lib/gen (TS_RS_EXPORT_DIR in .cargo/config.toml).
      capped cargo nextest run --locked -p vesper-protocol --profile ci
      git diff --exit-code -- "$gen"
      [ -z "$(git status --porcelain -- "$gen")" ] || { git status --short -- "$gen"; echo "untracked bindings above: commit them" >&2; exit 1; }
      ;;
    deny) step "cargo deny"; capped cargo deny --locked check ;;
    secrets) step "check-secrets"; scripts/check-secrets.sh ;;
    scripts)
      for t in scripts/test-*.sh; do
        step "$t"
        # The tests stub systemd-run and expect the capped commands, so they never see VESPER_NO_CAP.
        env -u VESPER_NO_CAP timeout 300 "$t"
      done
      ;;
    shell)
      step "shellcheck"
      if command -v shellcheck >/dev/null; then
        shellcheck -x -S warning scripts/*.sh
      else
        echo "skipped: shellcheck is not installed (CI runs it)"
      fi
      ;;
    web)
      step "web: npm ci, check, test, build"
      capped npm --prefix web ci --no-audit --no-fund
      capped npm --prefix web run check
      capped npm --prefix web test
      capped npm --prefix web run build
      ;;
    dsp)
      step "denoiser: native tests, then the wasm build"
      [ -f crates/vesper-dsp/model/DeepFilterNet3_onnx.tar.gz ] || scripts/fetch-model.sh
      capped cargo nextest run --locked -p vesper-dsp --features tract --profile ci
      capped scripts/build-dsp.sh
      ;;
  esac
}

for s in "${steps[@]}"; do run_step "$s"; done
echo
echo "ALL CHECKS PASSED"
