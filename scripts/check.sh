#!/usr/bin/env bash
# The check list, in order; stops at the first failure. CI jobs call subsets of these steps, so
# local and CI run the same commands. Cargo and npm run under the memory cap (scripts/capped.sh);
# VESPER_NO_CAP=1 runs them plain (CI runners).
set -euo pipefail

all=(fmt clippy test bindings deny secrets wording scripts shell web)
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
  secrets   scripts/check-secrets.sh and its test, scripts/test-check-secrets.sh
  wording   scripts/check-wording.sh (stock phrases and em dashes in docs and web strings)
  scripts   every other scripts/test-*.sh
  shell     shellcheck scripts/*.sh (skipped with a note if shellcheck is missing)
  web       npm ci, then \`npm run ci\` in web/ (svelte-check, vitest, vite build, gzip size budget)
  dsp       not in the full run: denoiser audiolab wasm
  denoiser  vesper-dsp native tests (fetches the model)
  audiolab  vesper-audiolab tests (fetches the DNSMOS model)
  wasm      scripts/build-dsp.sh (needs the wasm-bindgen CLI at the Cargo.lock version)
  fast      fmt clippy test
EOF
}

steps=()
for a in "$@"; do
  case $a in
    -h | --help) usage; exit 0 ;;
    fast) steps+=(fmt clippy test) ;;
    dsp) steps+=(denoiser audiolab wasm) ;;
    fmt | clippy | test | bindings | deny | secrets | wording | scripts | shell | web | denoiser | audiolab | wasm) steps+=("$a") ;;
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
    # vesper-audiolab needs the models and turns on vesper-dsp's tract feature, so it runs in `dsp`.
    test) step "nextest"; capped cargo nextest run --locked --workspace --exclude vesper-audiolab --profile ci ;;
    bindings)
      step "ts-rs bindings up to date"
      # The vesper-protocol export tests rewrite web/src/lib/gen (TS_RS_EXPORT_DIR in .cargo/config.toml).
      capped cargo nextest run --locked -p vesper-protocol --profile ci
      git diff --exit-code -- "$gen"
      [ -z "$(git status --porcelain -- "$gen")" ] || { git status --short -- "$gen"; echo "untracked bindings above: commit them" >&2; exit 1; }
      ;;
    deny) step "cargo deny"; capped cargo deny --locked check ;;
    secrets) step "check-secrets"; scripts/check-secrets.sh && scripts/test-check-secrets.sh ;;
    wording) step "check-wording"; scripts/check-wording.sh ;;
    scripts)
      for t in scripts/test-*.sh; do
        [ "$t" = scripts/test-check-secrets.sh ] && continue # run by the secrets step
        step "$t"
        # The tests stub systemd-run and expect the default cap, so they never see VESPER_NO_CAP or VESPER_CAP_MIB.
        env -u VESPER_NO_CAP -u VESPER_CAP_MIB timeout 300 "$t"
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
      step "web: npm ci, then npm run ci"
      capped npm --prefix web ci --no-audit --no-fund
      capped npm --prefix web run ci
      ;;
    denoiser)
      step "denoiser: native tests"
      [ -f crates/vesper-dsp/model/DeepFilterNet3_onnx.tar.gz ] || scripts/fetch-model.sh
      capped cargo nextest run --locked -p vesper-dsp --features tract --profile ci
      ;;
    audiolab)
      step "audio lab tests"
      [ -f research/models/sig_bak_ovr.onnx ] || research/scripts/fetch-dnsmos.sh
      capped cargo nextest run --locked -p vesper-audiolab --profile ci
      ;;
    wasm) step "denoiser wasm build"; capped scripts/build-dsp.sh ;;
  esac
}

for s in "${steps[@]}"; do run_step "$s"; done
echo
echo "ALL CHECKS PASSED"
