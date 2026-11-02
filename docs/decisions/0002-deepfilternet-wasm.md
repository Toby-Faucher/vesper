# 0002. Run DeepFilterNet in the browser as WASM

Status: accepted, 2026-10-30

Context: Friends use ordinary mics in noisy rooms. Server-side denoising would need decoding and re-encoding, which the SFU must not do.

Decision: DeepFilterNet3 (`deep_filter` crate, upstream `wasm` feature) runs in an AudioWorklet and worker on the client. Capture order: mic, browser echo cancellation, DeepFilterNet, gain, Opus through WebRTC.

Why: The spike measured 0.8 to 2 ms per 10 ms frame in Firefox, real time with margin. It cuts quiet speech by default, so attenuation, post-filter and input boost are user settings.

Consequences: The WASM bundle is about 15 MB because the `wasm` feature embeds a second copy of the model. (Correction, v1.4: it never did, the linker drops it. The 15 MB was tract's code plus a 4.4 MB name section. The build now drops that section, uses `deep_filter/tract`, and the server sends a 2.2 MB gzip copy; see known-issues.md.) Dev builds pin tract 0.21.4 and turn off its debug assertions (upstream bug). Each user picks DeepFilterNet, browser suppression or off, and the client falls back by itself when processing falls behind. Added delay is not yet measured end to end; the quality checkpoint does that.
