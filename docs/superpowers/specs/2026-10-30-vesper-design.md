# Vesper: design

Self-hosted voice and text chat for one friend group, written in Rust, built around voice that sounds good.

Status: draft for Toby's review. Items marked **[Toby]** were decided by Toby in conversation. Items marked **[call]** are the author's calls and are open to veto. Anything unmarked is description.

## Intent and why

Toby dislikes how intrusive Discord is and how much personal data it holds. The other self-hosted alternatives he tried sound bad, because Discord's strength is its background noise suppression. He wants a small, simple replacement for his friend group that does two things well: chat, and voice with excellent mic quality. Other alternatives try to do everything; Vesper deliberately does little.

Success **[Toby]**: his friends use it as their daily chat and it covers their needs. Quality bar **[call]**: in a blind listen with a noisy room, friends prefer Vesper's voice to Discord's.

Rust is the means, not a learning goal **[Toby]**. Latency and steady performance are expected benefits; mouth-to-ear delay is dominated by audio buffers, codec framing and the jitter buffer, so it is measured rather than assumed (see Testing).

## Goals

- Voice rooms: join with one click, mute, deafen, per-user volume, push to talk or voice activity, speaking indicators, stays connected when the tab is in the background **[Toby]**.
- Noise suppression close to Discord's, with echo cancellation always on and noise suppression and gain as user toggles. This is the most refined part of the app **[Toby]**.
- Configurable Opus bitrate per user, from speech grade up to music grade **[Toby]**.
- Text chat with full, searchable history kept forever **[Toby]**.
- Self-hosted, private, no telemetry **[Toby]**.
- Built-in diagnostics and repeatable latency and quality tests **[Toby: wants to run latency tests]**.

## Non-goals (v1)

- Screen share **[Toby: v2]**, monitoring and status page **[Toby: v2]**, phones and mobile layouts **[Toby]**, light theme **[Toby]**, accessibility work beyond keyboard basics **[Toby]**, games, watch-together, bots, plugins, federation, video, E2E encryption, call recording **[call]**.
- Multiple unrelated groups on one server **[Toby: one friend group]**.
- Open sourcing was ruled out at the start **[Toby]**; superseded: the repository is public under the MIT license **[Toby, 2026-11-01]**.

## Anti-goals

- Becoming a Discord clone. Features beyond chat and voice need a reason.
- A noise suppressor that makes people sound robotic or clips quiet speech. Clean-but-natural beats aggressive.
- Voice quality that depends on the user's setup being perfect.

## Constraints

- Desktop Chrome and Firefox only **[Toby]**.
- One host: a Proxmox box, in a container or in the Dockge VM, behind Caddy on a subdomain of tfaucher.com **[Toby]**. Proxmox already provides backups **[Toby]**. Starting size 1 vCPU, 1 GB RAM, 20 GB disk, scalable **[Toby]**.
- Toby's connection is 1 Gbps fiber and the friends are local, so bandwidth is not a constraint **[Toby]**.
- Friends are not behind strict NAT **[Toby]**, so no TURN server in v1 **[call]**. The design allows adding one later.
- Code lives on GitHub, public under the MIT license **[Toby, 2026-11-01]** (it was private at first).

## Decisions

### Stack

- **Server**: Rust, tokio, axum. **[call]**
- **Voice**: a Rust SFU on `str0m` (sans-IO WebRTC), one UDP port for all media. It forwards each speaker's Opus stream and does no decoding, mixing or transcoding. **[call, proven in spike]** Chosen over peer-to-peer mesh because every user would otherwise upload one copy per listener, and it keeps screen share (v2) feasible.
- **Database**: SQLite in WAL mode with `sqlx` and plain SQL; full-text search through SQLite FTS5. **[call; Toby asked to discuss, recommendation given, not yet confirmed]** Postgres would also work but adds a second service for about ten users.
- **Frontend**: Svelte with Tailwind, anime.js for motion, Linear-style dark theme with user-selectable accent colors **[Toby]**. Base UI is React-only, so the headless component layer is **Bits UI** **[call]**.
- **Client audio**: Rust compiled to WASM for the noise suppressor **[Toby: "a ton of WASM"]**. The rest of the client logic is TypeScript/Svelte; Rust is used on the client only where it earns its place **[call]**.
- **Protocol types**: a shared Rust crate for server messages; the client gets generated TypeScript types **[call]**.
- **Realtime channel**: one WebSocket per client carries chat events and WebRTC signaling, JSON messages **[call]**.
- **Layout**: a Cargo workspace with crates `vesper-server`, `vesper-sfu`, `vesper-protocol`, `vesper-dsp` (WASM), plus the web app **[call]**. Crate names carry the `vesper-` prefix because `vesper` on crates.io is taken.

### Voice and audio pipeline

Capture order **[call]**: microphone, then the browser's echo cancellation (always on), then DeepFilterNet in the client, then gain, then Opus encode through WebRTC to the SFU.

- The browser's own noise suppression and auto gain are off by default for the ML path, since stacking them degrades quality; they remain as user toggles **[Toby: toggles]**.
- **Noise suppressor**: DeepFilterNet3 (Rust crate `deep_filter`, upstream `wasm` feature), running off the main thread. The spike measured it at 0.8 to 2 ms per 10 ms frame in Firefox (real-time factor about 0.1 to 0.2), with isolated single frames up to 11 to 12 ms **[spike]**. A small buffer between the audio thread and the denoiser absorbs those spikes, trading a few tens of milliseconds for glitch-free audio **[call]**. The repo also ships a low-latency model variant; its latency and quality are untested.
- **Tuning**: attenuation limit, post-filter strength and input boost are exposed as settings, because the spike showed DeepFilterNet cuts quiet speech by default **[spike]**. Defaults are chosen by ear before launch **[call]**.
- **Bitrate**: user setting from about 32 kbps to 128 kbps and above. Default 64 kbps mono **[call]**. FEC on. DTX on at speech bitrates **[call]**.
- Frame size 20 ms **[call]**, 48 kHz.
- Opus, the browser's jitter buffer and congestion control handle loss; the SFU forwards, answers NACKs and relays RTCP **[call]**.
- Voice activity uses an adjustable threshold and a live level meter. Push to talk works while the tab has focus only; browsers cannot capture global hotkeys **[call]**.

Known limit: Bluetooth headsets drop to a narrow call profile and often do their own noise processing, so they cap achievable quality. The app detects and warns where it can **[call]**.

### Text chat

Rooms **[Toby]** with edit, delete, reply, reactions, mentions, basic Markdown, image and file upload (25 MB, stored on local disk), typing indicators, unread badges, pinned messages, jump-to-reply, infinite scroll back **[call: all]**. Search over all history. No threads, custom emoji, GIF search or link previews in v1 **[call]**.

### Rooms and voice presence

Every room has text and a voice call; the sidebar shows who is in each voice room live **[call]**. Admins create and reorder rooms; rooms can be restricted to a member list **[call]**. No automatic disconnect or AFK room **[Toby]**. Soft cap of 12 in a voice room **[call]**.

### Accounts and access

Invite links created by the owner **[call]**. Username with passkey (WebAuthn) and password fallback, argon2id for passwords **[call]**. Three fixed roles: owner, moderator, member **[call]**. Long-lived sessions (90 days) with a device list **[call]**. Display name, avatar, accent color per user **[call]**. No email in v1 **[call]**.

### Security

Strict CSP, HttpOnly SameSite cookies, CSRF protection, rate limits on login, invites, messages and uploads, safe serving of uploads **[call]**. Voice is encrypted hop by hop (DTLS-SRTP) and the server can hear it; text is stored in the clear on Toby's server. No end-to-end encryption in v1 **[call]**. No analytics or telemetry **[Toby]**.

### Deployment

- One static binary plus Caddy for TLS, run from a container with a systemd unit or Docker **[call]**.
- HTTPS for the web app and WebSocket goes through Caddy. WebRTC media does not: one UDP port is forwarded from the router to the container, and the SFU advertises the public IP as an ICE candidate **[spike: worked from a phone on cellular]**.
- Toby's own client on the LAN needs the same hostname to resolve to the LAN address (split DNS) or router hairpin NAT **[call]**.
- Data lives in one directory (SQLite file and uploads), so Proxmox backups cover it. SQLite is kept crash-consistent under snapshots by WAL mode, and a periodic `.backup` copy is a cheap extra **[call]**.

## Build order

1. Voice skeleton: signaling, an SFU room with several browsers, device selection, mute and deafen, per-user volume.
2. Client audio: DeepFilterNet in the capture path, bitrate and tuning settings, mic test screen, diagnostics page.
3. Accounts and invites, rooms, voice presence.
4. Text chat, history, search, uploads.
5. Polish: Linear-style UI, motion, notifications, settings.
6. v2 candidates: screen share, monitoring.

Voice goes first because it is the risky part. The spike already covered UDP reachability and denoiser speed.

## Testing

- Rust unit and integration tests; a headless-browser call test; a synthetic client that joins N speakers to load the SFU **[call]**.
- Packet loss and jitter tests with `tc netem` **[call]**.
- Latency: WebRTC stats for round trip and jitter, plus an acoustic loopback test (play a click, record it back through a call) for true mouth-to-ear delay **[Toby wants to test latency; method is a call]**.
- Quality: record the same noisy clip through Vesper and Discord and compare blind **[call]**.

## Spike results

- str0m's example SFU relayed audio on the LAN and from a phone on cellular via forwarded UDP 40000. Result: UDP path and str0m work.
- DeepFilterNet3 built to WASM from upstream with `wasm-bindgen` 0.2.129 and SIMD. Result: real-time in Firefox, good perceived quality; quiet speech partly clipped by default; settings can tune it.
- The test mic was a Bluetooth headset that probably denoised on its own, so the raw-versus-denoised comparison was not clean.
- Not yet tested: Chrome, a weaker laptop, the denoiser running live next to an SFU call, a wired or USB mic.

## Calls added after builder review

All marked **[call]**; open to veto.

- **Denoiser fallback.** Failure of DeepFilterNet is not a project blocker. Each user picks a mode: DeepFilterNet, browser noise suppression, or off. The client falls back automatically (DeepFilterNet to browser suppression) when processing falls behind real time, and says so in the UI. The Proxmox host is x86-64 (Proxmox VE).
- **Connectivity.** Friends' networks are an assumption, not a measurement, so v1 ships a connectivity check on the diagnostics page that reports whether UDP media reaches the server. If a friend fails it, TURN or a TCP fallback is added then; nothing is built for it up front. The SFU advertises a public address resolved from a configured hostname and re-resolved periodically, so a changed home IP is picked up without a restart.
- **Quality gate.** After build step 2 there is a tuning checkpoint: a recorded set of three noisy scenarios (fan, keyboard, TV) and at least three listeners comparing Vesper with Discord. Toby signs off before the text and polish work is treated as the main line. It is a checkpoint, not a blocker for building the non-voice parts.
- **Hostname and passkeys.** One fixed hostname is the passkey relying-party ID, `vesper.tfaucher.com`, used by every device, including Toby's own on the LAN through split DNS. The first owner is created by a server CLI command that prints a one-time sign-in link; invites work the same way afterward.
- **Packaging.** Primary target is a Docker image plus a compose file for Dockge. A static binary with a systemd unit stays supported for an LXC container.
- **Moderators** can delete others' messages, kick someone from a room, and force-mute or disconnect someone in voice. Bans and role changes are owner-only.
- **Disk.** History is never auto-deleted. The admin page shows disk use and warns at 80%; uploads have a per-file limit (25 MB) and a total quota.

## Open items

- Confirmed by Toby on 2026-10-30: SQLite with `sqlx`; invite links with passkeys and a password fallback; no TURN in v1.
- Trademark and clash check for the name beyond crates.io, npm and GitHub (a quick search found no voice chat app called Vesper).
- Logo and visual identity.

## Left to the builder

Choice of WASM bindings and build tooling, the exact worker and buffering layout for the denoiser, database schema, API shape, project scaffolding, Svelte structure, and test harness details.
