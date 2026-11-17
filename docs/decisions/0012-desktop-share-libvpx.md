# 0012. The desktop app encodes its screen with libvpx, linked statically

Status: accepted, 2026-11-16. Supersedes the encoder part of [ADR 0011](0011-desktop-screen-share-send.md); its capture, threads,
peer connection and quality options stand.

Context: ADR 0011 chose `vp8-encoder`, pure Rust and single threaded, and said libvpx was the upgrade if 1080p was too slow. Measured
with `share_bench` (`desktop/src/voice/share.rs`) on an i7-12700, a release build, the image viewer scene scrolling 8 px a frame:
`vp8-encoder` took 17 to 18 ms for a 1080p frame, and turning the captured picture into I420 (scalar Rust at the source size,
then scaled) took another 5 ms at 1080p, 21 ms from 1440p and 33 ms from 4K. A 1440p screen sent sharp topped out near 26 fps and
a 4K screen near 20. The wire stays VP8: the SFU and the web viewers must not change.

Decision:

- **Encoder: libvpx 1.17.0** through `vpx-rs` (BSD-2-Clause, a safe API; libvpx is BSD-3-Clause) in realtime mode: one pass, no lag,
  constant bitrate, cpu-used -6 (a fixed speed 6, WebRTC's VP8 setting), screen content mode, error resilient, up to 8 threads (half
  the cores), no automatic keyframes (a viewer's PLI or a new encoder makes one). Times are on the 90 kHz RTP clock, so a new frame
  rate needs nothing. A new bitrate starts a new encoder (vpx-rs cannot change it in place), which costs one keyframe when the
  share dialog's bitrate is changed.
- **Linked statically on both systems**, so the Linux tarball and AppImage do not depend on a distro's libvpx soname.
  `scripts/build-libvpx.sh` puts it in `desktop/.libvpx/` and `desktop/.cargo/config.toml` points the build there. Linux: built from
  the pinned v1.17.0 source tarball (sha256 checked) with nasm, runtime CPU detection on. Windows: vcpkg's libvpx, triplet
  `x64-windows-static-md` built for release only (a static library on the dynamic C runtime, as rustc links). CI caches the folder,
  keyed on the script.
- **Two crates patched in `desktop/vendor/`**: `vpx-rs` 0.2.1 passed cpu-used as a pointer, so libvpx clamped its bits to 16 or -16
  and ignored the speed asked for; `env-libvpx-sys` 5.1.3 uses bindgen 0.68, which wrote libvpx's config structs as opaque with
  libclang 23 (Arch). Each `PATCHED.txt` says what changed.
- **Conversion**: a bigger screen is scaled down in RGB first (`fast_image_resize`, box filter, as `vp8-encoder`'s scaler averaged the
  area), then turned into I420 with `yuv` (SIMD, BT.601 limited range, as before). The sent size is always even (I420 needs it) and an
  odd edge is cropped.
- `vp8-encoder` stays as a dev-dependency: the tests' fake sharers use it.

Why:

- `share_bench` before and after, per frame (convert+scale, then encode, at 8 Mbps, scrolling):

  | Screen, sent at | Before | After |
  |---|---|---|
  | 1080p, 1920x1080 | 4.7 + 16.9 ms | 0.5 + 2.4 ms |
  | 1440p, 1920x1080 | 20.6 + 18.2 ms | 3.8 + 2.2 ms |
  | 4K, 1920x1080 | 32.7 + 18.1 ms | 5.0 + 2.3 ms |
  | 1440p, 1280x720 | 14.3 + 7.7 ms | 1.9 + 1.3 ms |

- Quality is not worse for the bytes: luma PSNR after the viewers' decoder (`vp8-decoder`), frames 5 to 39, scrolling at 8 Mbps,
  1080p sent at 1080p: 55.3 dB at 20 KB a frame before, 56.8 dB at 10 KB after; 4K sent at 1080p: 54.2 dB at 17 KB before,
  55.6 dB at 9 KB after. At 2 Mbps the frames stay under the 7 KB budget (5 to 6 KB).
- cpu-used -8 and faster were no quicker on a scroll, sent about twice the bytes and lost 3 to 11 dB; -6 is also WebRTC's choice.

Consequences:

- Every desktop build needs `scripts/build-libvpx.sh` first ([dev.md](../dev.md)). On Linux that is a C compiler, make and nasm (or
  the pinned nasm it builds); on Windows, vcpkg, which the GitHub runners have. A cold CI cache adds the libvpx build (about 20 s
  on Linux, a few minutes through vcpkg on Windows).
- The Windows libvpx version is whatever the runner's vcpkg port ships (the script prints it); only the Linux build is pinned.
- libvpx is C, so `unsafe` code now runs in the encoder (inside `vpx-rs` and libvpx, not in our crate, where `unsafe` stays
  forbidden).
- `desktop/package/THIRD-PARTY.txt` carries libvpx's and vpx-rs's licences and says where `env-libvpx-sys`'s MPL source is.

Verified: on Linux (Arch, the dev machine): `share_bench` above; `colours_survive_the_conversion_and_the_codec` decodes libvpx's VP8
with `vp8-decoder` and checks the colours; `desktop/tests/share.rs` and the other desktop tests pass. CI builds, lints and tests the
desktop app on Linux and Windows, and the `desktop-package` run builds the four packages. Not verified: a real share from a real
Windows PC or from Toby's Framework 12 laptop (slower cores, so the timings above will be higher there), across a real network,
or the packages started on another distro.
