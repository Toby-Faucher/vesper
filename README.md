<div align="center">

<img src="assets/logo.png" alt="Vesper" width="360">

### Voice chat for your friends, on your own server. The mic is the product.

![Rust](https://img.shields.io/badge/Rust-2024-fbbf24?style=flat-square&logo=rust&logoColor=1c1405&labelColor=121419)
![Svelte](https://img.shields.io/badge/UI-Svelte%205-fbbf24?style=flat-square&labelColor=121419)
![WebRTC](https://img.shields.io/badge/audio-WebRTC%20%2B%20Opus-fbbf24?style=flat-square&labelColor=121419)
![WASM](https://img.shields.io/badge/denoiser-DeepFilterNet%20in%20WASM-fbbf24?style=flat-square&labelColor=121419)
![Status](https://img.shields.io/badge/status-personal%20project-71717a?style=flat-square&labelColor=121419)

</div>

---

Vesper is a small, self-hosted voice and text chat for one friend group. It runs in a desktop browser (Chrome or Firefox) and the server is a single Rust binary.

It exists because the other Discord replacements sound bad. Discord's edge is its noise suppression, so Vesper puts that first: a DeepFilterNet model runs in the browser as WASM, and your voice is cleaned before it is encoded and sent. Everything else is kept small on purpose. No games, no bots, no watch-together.

## What you get

| | |
|---|---|
| **Clean mic** | DeepFilterNet3 in an AudioWorklet and a Worker. If it fails to load or falls behind, the call keeps going on the browser's own suppression and the UI says so. |
| **Your bitrate** | Pick 32 to 510 kbps per person. Opus at 48 kHz, in-band FEC on, optional DTX. |
| **Voice controls** | Mute, deafen, per-person volume up to 400 %, a speaking ring, push to talk, a mic test with a level meter. |
| **Screen share** | In group rooms. 30 or 60 fps at a bitrate you pick, with an upload test. A panel with Grid, Focus and Dock layouts. A share only sends while someone watches it. |
| **Text chat** | Rooms with markdown, replies, reactions, pins, file uploads, search and mentions. Web Push for mentions and for voice starting. |
| **Direct messages** | A locked two-person chat. Only its two members can see it, and a voice join rings the other person. |
| **A Rust SFU** | One UDP port carries all audio and video. The server forwards each person's Opus stream and each VP8 share and never decodes them. |
| **Diagnostics** | Live round trip, jitter and loss, mic and denoiser levels, plus a "Test connection" button that tells you if UDP is blocked. |
| **Looks** | Dark, Light or System, an accent colour per account, and four more styles: Persona, Cozy, BBS and Aurora. |
| **Yours** | Self-hosted, no telemetry, accounts by invite only, sign in with a password or a passkey. |

## How it fits together

```
 browser (Chrome / Firefox)                         your server
┌─────────────────────────────┐  https + websocket  ┌──────────────────────────────┐
│ Svelte UI                   │◄───────────────────►│ Caddy ──► vesper-server      │
│ mic ─► DeepFilterNet (WASM) │                     │            web app, /ws,     │
│ Opus over WebRTC            │◄═ audio, UDP 40000 ═►│            presence, SFU     │
└─────────────────────────────┘                     └──────────────────────────────┘
```

Signaling and presence go over one WebSocket. Audio never goes through Caddy.

## Quickstart

You need Rust, Node and the `wasm32-unknown-unknown` target. From the repo root:

```bash
scripts/build-dsp.sh
npm --prefix web install && npm --prefix web run build
cargo run -p vesper-server -- create-owner   # prints a one-use link for the first account
VESPER_HTTP_BIND=127.0.0.1:8080 VESPER_UDP_BIND=127.0.0.1:40000 VESPER_WEB_DIR=web/dist cargo run -p vesper-server --release
```

Open the printed link to create the owner account, then sign in at http://127.0.0.1:8080. More detail, tests and the hot-reload setup are in [docs/dev.md](docs/dev.md).
For friends on other machines, see [docs/deploy.md](docs/deploy.md): Docker or systemd, Caddy for HTTPS, and forwarding UDP 40000 on your router.

## Status

Last updated 2026-11-13. This is a personal project. The latest release is v1.7.1. [docs/versions.md](docs/versions.md) lists each release and what was verified in it.

- Built and covered by tests: the SFU, the server, the web client, text chat, accounts and passkeys, screen share, the load test (12 speakers for 30 s, no loss on a local run), the deploy files. The end-to-end suite runs headless Chromium, with a canvas in place of a real screen.
- **Deployed:** v1.0 on 2026-11-01. A test build of v1.7.1 has run on the live server since 2026-11-11.
- **Not verified:** screen share in Firefox or Zen in a real call, Windows browsers, several sharers at once on a real network, passkeys with a real authenticator, and a call rejoining without a click in a real browser. Docker, Caddy and the router forward are not verified on the dev machine.
- **Not measured yet:** the delay DeepFilterNet adds, and how the audio sounds next to Discord. [docs/testing/checkpoint.md](docs/testing/checkpoint.md) is the listening test.
- **Known issue:** heavy video can still drop voice at the server's UDP receive buffer. See [docs/known-issues.md](docs/known-issues.md).
- **Next:** open work is in the GitHub issues.

## Layout

| Path | What it is |
|---|---|
| `crates/vesper-protocol` | Wire types shared by the server and the client (TypeScript is generated from them). |
| `crates/vesper-sfu` | The audio SFU, built on [str0m](https://github.com/algesten/str0m). |
| `crates/vesper-server` | axum server: signaling, presence, the web app, `/health`. |
| `crates/vesper-dsp` | DeepFilterNet wrapper, native for tests and WASM for the browser. |
| `crates/vesper-store` | SQLite storage through sqlx. The only crate that imports it ([ADR 0005](docs/decisions/0005-sqlite-sqlx.md)). |
| `crates/vesper-loadtest` | Synthetic speakers and screen shares for load and loss checks. |
| `crates/vesper-audiolab` | Audio lab: a seeded corpus of noisy mixtures, metrics and the `audiolab` CLI that scores the capture path. |
| `web/` | The Svelte client. |
| `deploy/` | Dockerfile, compose, Caddyfile example, systemd unit. |

## Docs

- [docs/dev.md](docs/dev.md): build, run, test, the checks.
- [docs/env.md](docs/env.md): every environment variable.
- [docs/architecture.md](docs/architecture.md): how the parts fit.
- [docs/deploy.md](docs/deploy.md): Docker, Caddy, ports.
- [docs/runbook.md](docs/runbook.md): running it and fixing it.
- [docs/decisions/](docs/decisions/README.md): why it is built this way.
- [docs/known-issues.md](docs/known-issues.md): small things left on purpose.
- [docs/testing/checkpoint.md](docs/testing/checkpoint.md): the quality checkpoint, step by step.
- [docs/testing/latency.md](docs/testing/latency.md): measuring delay, loss and server load.
- [docs/rust-style.md](docs/rust-style.md) and [docs/ui-design.md](docs/ui-design.md): code rules and UI principles.
- [docs/superpowers/specs](docs/superpowers/specs): the design. [docs/superpowers/plans](docs/superpowers/plans): the build plan.
- [SECURITY.md](SECURITY.md): reporting a problem.

## Credits

[DeepFilterNet](https://github.com/Rikorose/DeepFilterNet) does the noise suppression. [str0m](https://github.com/algesten/str0m) does WebRTC on the server.

## License

MIT, see [LICENSE](LICENSE).

The DeepFilterNet3 model is not in this repository: `scripts/fetch-model.sh` downloads it from the DeepFilterNet repository, which licenses it under MIT or Apache-2.0. The research corpus (CSTR VCTK, CC BY 4.0) is downloaded by `research/scripts/` and is not in the repository either. Dependencies keep their own licenses; `cargo deny check licenses` lists the allowed ones in `deny.toml`.
