# 0001. Use str0m for the SFU

Status: accepted, 2026-10-30

Context: Voice needs a server that forwards each speaker's Opus stream to up to 12 users. A peer-to-peer mesh would make every client upload to every other client.

Decision: A Rust SFU on str0m, a sans-IO WebRTC library. One UDP port carries all media. The SFU forwards packets and never decodes, mixes or transcodes.

Why: A spike relayed audio on the LAN and from a phone on cellular through a forwarded UDP port. Sans-IO means the SFU owns its own thread and clock, which keeps tests deterministic (a fake client in `vesper_sfu::testing`).

Consequences: str0m limits MSID characters and length, so user ids are lowercase hex, 64 characters at most. Panics in str0m kill the SFU thread, so the server watches it, `/health` returns 503 and the process exits so the restart policy recovers it. No TURN in v1, so users behind strict NAT cannot join.
