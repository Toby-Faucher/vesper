# Known issues

Things found in review that were left on purpose, because they are small, rare or wait on a bigger piece of work.
Last updated 2026-10-30. Nothing here was run in a real browser or a real deploy. Fix an item, then delete its line in the same commit.

## Security and abuse (all need the join key, except where noted)

- **Anyone with the key can kick anyone.** A join under an existing name closes that name's earlier session before the new join is checked. This is the shared-key shortcut ([decision 0003](decisions/0003-shared-join-key.md)). Accounts remove it.
- **Names are not normalised.** Lookalike names (Unicode forms, zero-width characters) count as different people.
- **No Origin check on `/ws`.** Low risk while there are no cookies. It must land with accounts.
- **No Content-Security-Policy.** See [decision 0004](decisions/0004-no-csp-yet.md).
- **Failed joins are limited per socket, not per address.** After 5 bad messages a socket is closed, but reconnecting resets the count. Guessing the key is not throttled by address. Use a long key.
- **Plain HTTP connections before the upgrade are not limited.** A slow client can hold a connection on `/ws` or `/health`. The cap of 256 sockets counts upgraded sockets only.
- **A client that stops reading can grow memory.** Each socket has an unbounded outbound queue, and there is no send timeout.
- **Pipelined messages after the 5th bad one may still be handled.** A correct join in that window can leave a ghost member for about 19 s until the session times out.
- **A very large `VESPER_MAX_SOCKETS` panics at start.** Keep it in the hundreds.

## Server behaviour

- **One lock serialises joins.** It is held while the SFU joins and while an old session closes (up to 2 s). Fine for 12 people. A key holder can slow others by rejoining in a loop.
- **A changed public IP needs a rejoin.** The name is re-resolved every 60 s, but only new sessions use the new address. A failed lookup at start advertises nothing until the next refresh, with a warning every 60 s.
- **Without `VESPER_PUBLIC_HOST`, `VESPER_UDP_PORT_PUBLIC` is ignored**, and a `0.0.0.0` bind is advertised as is, with a warning.
- **The ping interval (25 s) and the pre-join error cap (5) have no environment variable.**
- **Static files:** a missing extensionless path under `/assets/` returns `index.html` with the `immutable` cache header. Replies 206 and 304 under `/assets/` get `no-cache`. Base images in `deploy/Dockerfile` are not pinned by digest, and assets are not precompressed.
- **The SFU wakes every 10 ms when idle.** Cheap, but it is not zero.
- **Tests:** the rejoin race test is a regression guard, not a proof (the lock order is the proof). A 300 ms deadline test could fail on a very slow machine.

## Web client

- **Not tried in real Chrome or Firefox.** Firefox may refuse a mic and AudioContext rate mismatch, which forces the browser fallback every time.
- **The VAD threshold in Settings is display-only.** Nothing gates on it. DTX still drops silence. The UI says so. See [decision 0006](decisions/0006-quality-checkpoint-gate.md).
- **DTX applies at the next join.** The client cannot renegotiate it mid-call.
- **A mic that ends mid-call has no "Retry microphone" button.** The status line says what happened, but you leave and rejoin to recover.
- **Stale mode reason on unmute.** The capture watcher can replay an old fallback reason after an unmute.
- **After a failed join, the page's guard against messages from an old socket stays set for a short window.** It clears when the next join starts.
- **No fade on underrun or drops.** A starved output can click.
- **Boost is lost when the mode falls back to the browser**, and the status line says so.
- **A track swap can leave the new track out of sync with mute or push to talk for one tick.** Room re-applies the state after the mode change.
- **Firefox may fire a button click on Space key-up** after push to talk captured the key.
- **Stored per-person volumes are never pruned.** `navigator.locks` missing only logs a warning.
- **Unhandled edge cases:** a teardown that races a joining call can leave a poll timer running, and an `AudioContext` constructor that throws is not caught. Both are rare.
- **The connection test reports only the local candidate type.**

## Denoiser

- **The wasm is about 15 MB** because the `wasm` feature of `deep_filter` embeds a second copy of the model. A build on `deep_filter/tract` plus `getrandom` with the `js` feature should halve it. Not tried.
- **No panic hook in the wasm**, so a Rust panic shows as an opaque error.
- **Added delay is not measured.** The algorithm's lookahead plus the 30 ms output queue may exceed 30 ms. The [checkpoint](testing/checkpoint.md) must measure it.
- **`ndarray` and the `tract-core` pin in `vesper-dsp` are not optional**, although only the `tract` feature needs them. A featureless build pulls them in for nothing.
- **Thresholds are untuned:** the fallback trips at 10 late frames out of 100, with a 10 ms budget. The numbers are a first guess.

## Build and deploy

- **`scripts/build-dsp.sh` writes straight into `web/public/dsp`.** If `wasm-bindgen` fails half way, the folder is partial.
- **The load test assumes 100 clock ticks per second** when it computes server CPU, and reads `/proc`, so it is Linux only. The docs say so.
- **Not verified:** the Docker build, Dockge, the router forward, Caddy, and the systemd hardening options. See [deploy.md](deploy.md#not-verified).
