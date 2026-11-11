# 0009. The SFU forwards one VP8 screen per person

Status: accepted, 2026-11-09. Design: [the screen share spec](../superpowers/specs/2026-11-09-screen-share-design.md).

Context: The SFU was audio only. A join offer had to carry exactly one audio m-line, and anything else was refused. Screen share
needs video to go through the same SFU, to the people in the same call, without hurting voice. Every room's media runs on the one SFU
thread. The SFU drives renegotiation, and it makes no new offer while one is unanswered.

Decision:

- **One audio m-line and at most one video m-line.** A join offer with anything else (two video m-lines, video without audio, a data
  channel) is still `BadOffer`. A client whose offer had no video m-line (an older build) is never offered video.
- **VP8 only.** The SFU's codecs are Opus and VP8 (`room.rs`, `new_client`).
- **Video m-lines are fixed at join.** A client's own screen m-line is in its join offer. The SFU offers a client one video m-line for
  each other client with video, when either joins, whether anyone shares or not. Starting or stopping a share needs no renegotiation.
- **Forwarding is gated, not negotiated.** The server calls `SfuHandle::set_sharing(user, on)` and `SfuHandle::watch(viewer, sharer, on)`.
  A video packet goes from a sender to a receiver only while the sender is sharing and the receiver watches them. Both are per session:
  a new join starts not sharing and watching nobody. A force-mute drops only audio.
- **One layer, no simulcast.** Every viewer gets the same stream.

Why:

- One video m-line per client caps what one client can make the SFU hold, as the one audio m-line did.
- Firefox and Chromium both send VP8 (from docs). Zen sent about 59 fps of 1728x1080 VP8 in software on Toby's Linux machine
  (measured, see the spec).
- Renegotiating on every share start, stop or watch change would queue behind any unanswered offer and cost a round trip each time.
  Gating packets on the SFU thread takes effect on the next packet.
- A second, smaller copy would cost the sharer more encoding, which is what hurts Linux sharers. The group is four people on fast links.

Consequences:

- Each client with video holds a video m-line for every other client with video, even when nobody shares. It carries nothing until a
  share is on and watched.
- A viewer with a bad link gets the same stream as everyone else. Its keyframe requests cost every viewer a little.
- A share and its watchers must be set again after a rejoin, because the SFU forgets them with the session.
- Video shares the voice thread and its socket. The [video load test](../testing/latency.md#video-load) measured it: the usual load
  (2 x 10 Mbps) and 4 x 10 Mbps pass; 4 x 20 Mbps drops packets at the server's socket buffer, so the server caps a room's shares.

Verified: `crates/vesper-sfu/tests/forwarding.rs` with `FakeClient` over loopback (`video_reaches_only_watchers`, `video_needs_a_share`,
`late_viewer_gets_video`, `audio_only_client_gets_no_video`, `force_mute_keeps_video`, `hostile_offers_are_rejected`), and
`video_only_for_a_client_with_video` in `client.rs`. Not verified: a real browser sending or receiving video through the SFU.
