//! Sending my screen share. A capture thread (`vesper-capture`) takes pictures of the screen (`capture.rs`) and keeps
//! only the newest. An encode thread (`vesper-encode`) scales it down and turns it into I420 (`to_i420`, SIMD), then
//! into VP8 (libvpx in realtime mode through `vpx-rs`, linked statically, ADR 0011) at the share's frame rate and
//! bitrate, and hands each frame to the call engine, which sends it on its video m-line. Neither runs on the UI or the
//! call's thread, so a slow encode costs frames, never voice. `share_bench` (ignored) measures each step.
//!
//! The encoder runs only while someone watches (`ShareSetup::active`, from the server's watcher count), as the web
//! turns its encoding off. A still screen gives no new pictures on Wayland and Windows, so the newest one is kept
//! and encoded again when a viewer asks for a keyframe.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// One captured picture: `height` rows of `stride` bytes, each pixel `bpp` bytes with red, green and blue at `rgb`.
pub struct Shot {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub bpp: usize,
    pub rgb: [usize; 3],
    pub data: Vec<u8>,
}

/// A screen being captured. Dropping it stops the capture.
pub trait Screen: Send {
    /// Waits a moment for the next picture: none if the screen stayed still, so a stop is not missed. An error ends
    /// the share; its text is for the user.
    fn next(&mut self) -> Result<Option<Shot>, String>;
}

/// Opens a screen, on the capture thread: on Wayland this waits for the system's screen picker.
pub type OpenScreen = Box<dyn FnOnce(u32) -> Result<Box<dyn Screen>, String> + Send>;

/// How my share is sent, from the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShareSetup {
    /// 30 or 60.
    pub fps: u32,
    pub kbps: u32,
    /// Keep text sharp (up to 1080p) rather than motion smooth (up to 720p, cheaper to encode).
    pub sharp: bool,
    /// Someone watches: encode and send. Otherwise the encoder is off.
    pub active: bool,
    /// What to share, chosen in the share dialog; none for the system's picker or the primary monitor.
    pub source: Option<super::capture::Source>,
}

/// The most pixels sent: 1080p for sharp text, 720p for smooth motion. A bigger screen is scaled down to fit.
/// share_bench on an i7-12700: a 1080p frame from a 4K screen takes about 8 ms in all, so 60 fps has room.
pub const SHARP_PIXELS: u32 = 1920 * 1080;
pub const SMOOTH_PIXELS: u32 = 1280 * 720;

/// From the share's threads to the engine.
#[derive(Debug, PartialEq)]
pub enum Out {
    /// The screen is open (the picker, if any, was answered): the share can start.
    Open,
    /// One VP8 frame, and its time on the 90 kHz RTP clock.
    Frame { data: Vec<u8>, rtp: u64 },
    /// Every 2 s while encoding: what was sent, as the web's share card shows it.
    Figures {
        width: u32,
        height: u32,
        fps: f32,
        kbps: f32,
    },
    /// The share ended here: the picker was cancelled (empty text) or the capture failed.
    Ended(String),
}

/// The newest picture, waiting for the encoder.
#[derive(Default)]
struct Slot {
    shot: Mutex<Option<Shot>>,
    came: Condvar,
}

impl Slot {
    fn put(&self, s: Shot) {
        *self.shot.lock().unwrap() = Some(s);
        self.came.notify_one();
    }

    /// The newest picture, waiting until `until` for one.
    fn take(&self, until: Instant) -> Option<Shot> {
        let mut g = self.shot.lock().unwrap();
        while g.is_none() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            g = self.came.wait_timeout(g, left).unwrap().0;
        }
        g.take()
    }
}

struct Shared {
    setup: Mutex<ShareSetup>,
    keyframe: AtomicBool,
    stop: AtomicBool,
    slot: Slot,
}

/// The engine's end of a running share. Dropping it ends both threads.
pub struct Sending {
    shared: Arc<Shared>,
    pub out: Receiver<Out>,
}

impl Sending {
    pub fn start(open: OpenScreen, setup: ShareSetup) -> Sending {
        let shared = Arc::new(Shared {
            setup: Mutex::new(setup),
            keyframe: AtomicBool::new(false),
            stop: AtomicBool::new(false),
            slot: Slot::default(),
        });
        let (tx, out) = channel();
        let (s, t) = (shared.clone(), tx.clone());
        std::thread::Builder::new()
            .name("vesper-capture".into())
            .spawn(move || capture(open, &s, &t))
            .expect("the capture thread starts");
        let s = shared.clone();
        std::thread::Builder::new()
            .name("vesper-encode".into())
            .spawn(move || encode(&s, &tx))
            .expect("the encode thread starts");
        Sending { shared, out }
    }

    pub fn set(&self, setup: ShareSetup) {
        *self.shared.setup.lock().unwrap() = setup;
    }

    /// A viewer asked for a keyframe (a PLI through the SFU).
    pub fn keyframe(&self) {
        self.shared.keyframe.store(true, Ordering::Relaxed);
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.slot.came.notify_one();
    }
}

/// A capture crate that panics (Windows' does when the capture cannot start) ends the share, not the app.
fn capture(open: OpenScreen, shared: &Shared, out: &Sender<Out>) {
    let run = std::panic::AssertUnwindSafe(|| run_capture(open, shared, out));
    if std::panic::catch_unwind(run).is_err() {
        shared.stop.store(true, Ordering::Relaxed);
        let _ = out.send(Out::Ended(super::capture::FAILED.into()));
    }
}

fn run_capture(open: OpenScreen, shared: &Shared, out: &Sender<Out>) {
    let fps = shared.setup.lock().unwrap().fps;
    let mut screen = match open(fps) {
        Ok(s) => s,
        Err(e) => return drop(out.send(Out::Ended(e))),
    };
    if shared.stop.load(Ordering::Relaxed) {
        return; // stopped while the picker was open
    }
    let _ = out.send(Out::Open);
    while !shared.stop.load(Ordering::Relaxed) {
        match screen.next() {
            Ok(Some(s)) => shared.slot.put(s),
            Ok(None) => {}
            Err(e) => {
                shared.stop.store(true, Ordering::Relaxed);
                let _ = out.send(Out::Ended(e));
                return;
            }
        }
    }
}

fn encode(shared: &Shared, out: &Sender<Out>) {
    let start = Instant::now();
    let mut enc: Option<Vp8> = None;
    let mut held: Option<Shot> = None;
    let mut fresh = false;
    let mut due = start;
    let (mut since, mut frames, mut bytes) = (start, 0u32, 0usize);
    while !shared.stop.load(Ordering::Relaxed) {
        if let Some(s) = shared
            .slot
            .take(due.max(Instant::now() + Duration::from_millis(1)))
        {
            (held, fresh) = (Some(s), true);
            if Instant::now() < due {
                continue;
            }
        }
        let setup = *shared.setup.lock().unwrap();
        if !setup.active {
            // Off until watched; the next start begins with a keyframe.
            enc = None;
            due = Instant::now() + Duration::from_millis(100);
            continue;
        }
        let key = shared.keyframe.load(Ordering::Relaxed);
        let Some(shot) = held.as_ref().filter(|_| fresh || key || enc.is_none()) else {
            due = Instant::now() + Duration::from_millis(10);
            continue;
        };
        let now = Instant::now();
        due = now + Duration::from_secs(1) / setup.fps.max(1);
        fresh = false;
        let (w, h) = fit(shot.width, shot.height, setup.sharp);
        let i420 = to_i420(shot, w, h);
        // A new size or bitrate starts a new encoder (libvpx through vpx-rs cannot change its bitrate), so the
        // share dialog's change costs one keyframe.
        if !enc
            .as_ref()
            .is_some_and(|e| (e.w, e.h, e.kbps) == (w, h, setup.kbps))
        {
            enc = Vp8::new(w, h, setup.kbps);
        }
        let Some(e) = enc.as_mut() else {
            continue;
        };
        shared.keyframe.store(false, Ordering::Relaxed);
        let rtp = (now - start).as_micros() as u64 * 90 / 1000;
        let Some(data) = e.encode(&i420, rtp, 90_000 / u64::from(setup.fps.max(1)), key) else {
            continue;
        };
        (frames, bytes) = (frames + 1, bytes + data.len());
        if out.send(Out::Frame { data, rtp }).is_err() {
            return;
        }
        let secs = since.elapsed().as_secs_f32();
        if secs >= 2. {
            let _ = out.send(Out::Figures {
                width: w,
                height: h,
                fps: frames as f32 / secs,
                kbps: bytes as f32 * 8. / secs / 1000.,
            });
            (since, frames, bytes) = (Instant::now(), 0, 0);
        }
    }
}

/// libvpx's VP8 speed in realtime: -n is a fixed speed n (higher is faster), +n lets libvpx pick by the time left.
/// share_bench: -6 (WebRTC's) scrolls 1080p text in 2.4 ms at 10 KB a frame; -8 and faster send twice the bytes on
/// a scroll and lose 3 to 11 dB of PSNR, and are no quicker.
const CPU_USED: i32 = -6;

/// libvpx's VP8 encoder, set up for a screen as WebRTC does: realtime, no lag, constant bitrate, screen content,
/// error resilient, keyframes only when asked (a viewer's PLI) or when it starts.
struct Vp8 {
    enc: vpx_rs::Encoder<u8>,
    w: u32,
    h: u32,
    kbps: u32,
}

impl Vp8 {
    fn new(w: u32, h: u32, kbps: u32) -> Option<Vp8> {
        use vpx_rs::enc::ctrl::{EncoderControlSet as Set, Vp8ScreenContentMode};
        use vpx_rs::enc::{CodecId, ErrorResilient, KeyFrameMode, RateControl, Timebase};
        // Times are on the 90 kHz RTP clock, so a new frame rate needs no new encoder.
        let clock = Timebase {
            num: std::num::NonZero::<u32>::MIN,
            den: std::num::NonZero::new(90_000)?,
        };
        let rate = RateControl::ConstantBitRate(kbps);
        let mut c = vpx_rs::EncoderConfig::<u8>::new(CodecId::VP8, w, h, clock, rate).ok()?;
        // Half the cores, up to 8 (WebRTC's most for 1080p): the call and the app keep the rest.
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get() as u32);
        c.threads = (cores / 2).clamp(1, 8);
        c.error_resilient = ErrorResilient::DEFAULT;
        c.lag_in_frames = 0;
        c.kf_mode = KeyFrameMode::Disabled;
        c.rc_resize_allowed = None;
        let mut enc = vpx_rs::Encoder::new(c).ok()?;
        enc.codec_control_set(Set::CpuUsed(CPU_USED)).ok()?;
        enc.codec_control_set(Set::Vp8ScreenContentMode(Vp8ScreenContentMode::On))
            .ok()?;
        Some(Vp8 { enc, w, h, kbps })
    }

    /// One frame of I420 at `rtp` (90 kHz) that lasts `ticks`; none if libvpx failed or dropped it.
    fn encode(&mut self, i420: &[u8], rtp: u64, ticks: u64, key: bool) -> Option<Vec<u8>> {
        use vpx_rs::enc::{EncoderFrameFlags, EncodingDeadline, Packet};
        let (w, h) = (self.w as usize, self.h as usize);
        let pic =
            vpx_rs::YUVImageData::from_raw_data(vpx_rs::ImageFormat::I420, w, h, i420).ok()?;
        let flags = if key {
            EncoderFrameFlags::FORCE_KF
        } else {
            EncoderFrameFlags::empty()
        };
        let mut packets = self
            .enc
            .encode(rtp as i64, ticks, pic, EncodingDeadline::Realtime, flags)
            .ok()?;
        packets.find_map(|p| match p {
            Packet::CompressedFrame(f) => Some(f.data),
            _ => None,
        })
    }
}

/// The size to send `w` x `h` at: as it is up to the preference's pixels, else scaled down to fit. Always even, as
/// I420 wants: an odd edge is cropped.
pub fn fit(w: u32, h: u32, sharp: bool) -> (u32, u32) {
    let most = if sharp { SHARP_PIXELS } else { SMOOTH_PIXELS };
    let px = u64::from(w) * u64::from(h);
    let k = (f64::from(most) / px.max(1) as f64).sqrt().min(1.);
    let even = |n: u32| ((f64::from(n) * k) as u32 & !1).max(2);
    (even(w), even(h))
}

/// The shot as I420 of `w` x `h`, BT.601 limited range (what the viewers' VP8 decoders turn back into RGB). A
/// smaller size is scaled down in RGB first, so only the pixels sent are converted; both steps use SIMD.
pub fn to_i420(s: &Shot, w: u32, h: u32) -> Vec<u8> {
    use yuv::{BufferStoreMut, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};
    let (sw, sh) = (s.width as usize, s.height as usize);
    let whole = s.stride >= sw * s.bpp
        && s.stride.checked_rem(s.bpp) == Some(0)
        && s.data.len() >= s.stride * sh;
    let convert = match (s.bpp, s.rgb) {
        (4, [2, 1, 0]) if whole => yuv::bgra_to_yuv420, // BGRx too: alpha is not read
        (4, [0, 1, 2]) if whole => yuv::rgba_to_yuv420,
        (3, [2, 1, 0]) if whole => yuv::bgr_to_yuv420,
        (3, [0, 1, 2]) if whole => yuv::rgb_to_yuv420,
        // shortcut: other layouts (xRGB from some PipeWire streams) are repacked as RGBA, a slower scalar pass.
        _ => return to_i420(&rgba(s), w, h),
    };
    let scaled;
    let (mut data, mut stride) = (&s.data[..], s.stride);
    // An odd edge is cropped by converting w x h of the shot as it is.
    if (w, h) != (s.width & !1, s.height & !1) {
        use fast_image_resize as fr;
        let kind = if s.bpp == 4 {
            fr::PixelType::U8x4
        } else {
            fr::PixelType::U8x3
        };
        // The rows' padding is read as pixels on the right, then cut off (the filter never reads past the cut).
        let Ok(padded) =
            fr::images::ImageRef::new((s.stride / s.bpp) as u32, s.height, &s.data, kind)
        else {
            return Vec::new();
        };
        let Ok(src) = fr::images::CroppedImage::new(&padded, 0, 0, s.width, s.height) else {
            return Vec::new();
        };
        let mut dst = fr::images::Image::new(w, h, kind);
        let how = fr::ResizeOptions::new()
            .use_alpha(false)
            .resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Box)); // the source area under each pixel
        // Kept between frames (the encode thread's): setting up its buffers costs about 1 ms a frame.
        thread_local!(static R: std::cell::RefCell<fr::Resizer> = std::cell::RefCell::new(fr::Resizer::new()));
        if R.with_borrow_mut(|r| r.resize(&src, &mut dst, &how))
            .is_err()
        {
            return Vec::new();
        }
        scaled = dst.into_vec();
        (data, stride) = (&scaled[..], w as usize * s.bpp);
    }
    let (wu, hu) = (w as usize, h as usize);
    let (cw, ch) = (wu.div_ceil(2), hu.div_ceil(2));
    let mut out = vec![0u8; wu * hu + 2 * cw * ch];
    let (y, uv) = out.split_at_mut(wu * hu);
    let (u, v) = uv.split_at_mut(cw * ch);
    let mut planes = YuvPlanarImageMut {
        y_plane: BufferStoreMut::Borrowed(y),
        y_stride: w,
        u_plane: BufferStoreMut::Borrowed(u),
        u_stride: cw as u32,
        v_plane: BufferStoreMut::Borrowed(v),
        v_stride: cw as u32,
        width: w,
        height: h,
    };
    let range = (YuvRange::Limited, YuvStandardMatrix::Bt601);
    match convert(
        &mut planes,
        data,
        stride as u32,
        range.0,
        range.1,
        YuvConversionMode::Balanced,
    ) {
        Ok(()) => out,
        Err(_) => Vec::new(),
    }
}

/// The shot repacked as tightly packed RGBA, for a layout the SIMD conversion does not take.
fn rgba(s: &Shot) -> Shot {
    let [r, g, b] = s.rgb;
    let mut data = Vec::with_capacity(s.width as usize * s.height as usize * 4);
    for y in 0..s.height as usize {
        for x in 0..s.width as usize {
            let i = y * s.stride + x * s.bpp;
            let p = s
                .data
                .get(i..i + s.bpp)
                .map_or([0; 4], |p| [p[r], p[g], p[b], 255]);
            data.extend_from_slice(&p);
        }
    }
    Shot {
        width: s.width,
        height: s.height,
        stride: s.width as usize * 4,
        bpp: 4,
        rgb: [0, 1, 2],
        data,
    }
}

/// Moving colour bars for tests and the parity harness (`VESPER_DEV_SHARE`): red, green, blue and white, sliding a
/// pixel a frame, paced at the frame rate.
pub struct Pattern {
    pub width: u32,
    pub height: u32,
    fps: u32,
    start: Instant,
    made: u64,
}

impl Pattern {
    pub fn new(width: u32, height: u32, fps: u32) -> Pattern {
        let start = Instant::now();
        Pattern {
            width,
            height,
            fps: fps.max(1),
            start,
            made: 0,
        }
    }
}

impl Screen for Pattern {
    fn next(&mut self) -> Result<Option<Shot>, String> {
        let at = self.start + Duration::from_secs(1) * self.made as u32 / self.fps;
        std::thread::sleep(at.saturating_duration_since(Instant::now()));
        self.made += 1;
        let (w, h) = (self.width as usize, self.height as usize);
        let bars: [[u8; 4]; 4] = [
            [0, 0, 255, 255],
            [0, 255, 0, 255],
            [255, 0, 0, 255],
            [255; 4],
        ];
        let mut data = Vec::with_capacity(w * h * 4);
        for _ in 0..h {
            for x in 0..w {
                data.extend_from_slice(&bars[(x + self.made as usize) * 4 / w.max(1) % 4]);
            }
        }
        Ok(Some(Shot {
            width: self.width,
            height: self.height,
            stride: w * 4,
            bpp: 4,
            rgb: [2, 1, 0],
            data,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_screens_are_scaled_to_the_preference() {
        assert_eq!(fit(1920, 1080, true), (1920, 1080));
        assert_eq!(fit(1280, 720, false), (1280, 720));
        let (w, h) = fit(2560, 1440, true);
        assert!(w * h <= SHARP_PIXELS && w % 2 == 0 && h % 2 == 0, "{w}x{h}");
        assert!((f64::from(w) / f64::from(h) - 16. / 9.).abs() < 0.01);
        assert_eq!(fit(1920, 1080, false), (1280, 720));
    }

    #[test]
    fn colours_survive_the_conversion_and_the_codec() {
        let mut p = Pattern::new(64, 16, 1000);
        let shot = p.next().unwrap().unwrap();
        let i420 = to_i420(&shot, 64, 16);
        let mut enc = Vp8::new(64, 16, 2_000).unwrap();
        let f = enc.encode(&i420, 0, 3000, true).unwrap();
        // libvpx's VP8, read by the viewers' decoder.
        let mut dec = vp8_decoder::Decoder::new();
        let pic = dec.decode(&f).unwrap().unwrap();
        let bgra = super::super::video::bgra(&pic, pic.width, pic.height);
        // The middle of each bar (the first frame is shifted one pixel): red, green, blue, white.
        for (x, want) in [
            (8, [0, 0, 255]),
            (24, [0, 255, 0]),
            (40, [255, 0, 0]),
            (56, [255; 3]),
        ] {
            let i = (8 * 64 + x) * 4;
            for c in 0..3 {
                assert!(
                    bgra[i + c].abs_diff(want[c]) < 24,
                    "x {x}: {:?}",
                    &bgra[i..i + 3]
                );
            }
        }
        // Scaled down, the size is the one asked for.
        assert_eq!(to_i420(&shot, 32, 8).len(), 32 * 8 + 2 * 16 * 4);
    }

    #[test]
    fn odd_sizes_padded_rows_and_other_layouts_convert() {
        // Red, 33 x 17 with 8 bytes of padding a row, as BGRx and as xRGB (the slower repacked path).
        for (rgb, px) in [([2, 1, 0], [0, 0, 255, 0]), ([1, 2, 3], [0, 255, 0, 0])] {
            let (w, h, stride) = (33u32, 17u32, 33 * 4 + 8);
            let mut data = vec![7u8; stride * h as usize];
            for row in data.chunks_exact_mut(stride) {
                row[..w as usize * 4].as_chunks_mut::<4>().0.fill(px);
            }
            let shot = Shot {
                width: w,
                height: h,
                stride,
                bpp: 4,
                rgb,
                data,
            };
            assert_eq!(fit(w, h, true), (32, 16), "an odd edge is cropped");
            for (tw, th) in [(32, 16), (16, 8)] {
                let i420 = to_i420(&shot, tw, th);
                let (n, c) = ((tw * th) as usize, (tw * th / 4) as usize);
                assert_eq!(i420.len(), n + 2 * c, "{rgb:?} {tw}x{th}");
                // BT.601 limited range red: Y 82, U 90, V 240.
                let near = |p: &[u8], want: u8| p.iter().all(|&v| v.abs_diff(want) <= 2);
                assert!(near(&i420[..n], 82), "{rgb:?} {tw}x{th} Y {:?}", &i420[..n]);
                assert!(near(&i420[n..n + c], 90), "{rgb:?} U {:?}", &i420[n..n + 4]);
                assert!(
                    near(&i420[n + c..], 240),
                    "{rgb:?} V {:?}",
                    &i420[n + c..n + c + 4]
                );
            }
        }
    }

    #[test]
    fn frames_flow_only_while_watched_and_a_keyframe_comes_on_request() {
        let open: OpenScreen =
            Box::new(|fps| Ok(Box::new(Pattern::new(64, 32, fps)) as Box<dyn Screen>));
        let mut setup = ShareSetup {
            fps: 30,
            kbps: 500,
            sharp: true,
            active: false,
            source: None,
        };
        let s = Sending::start(open, setup);
        assert_eq!(s.out.recv_timeout(Duration::from_secs(5)), Ok(Out::Open));
        std::thread::sleep(Duration::from_millis(200));
        assert!(s.out.try_recv().is_err(), "nothing while nobody watches");

        setup.active = true;
        s.set(setup);
        let frame = || loop {
            match s.out.recv_timeout(Duration::from_secs(5)).unwrap() {
                Out::Frame { data, .. } => return data,
                _ => continue,
            }
        };
        let is_key = |d: &[u8]| d[0] & 1 == 0; // RFC 6386 frame tag: bit 0 clear is a keyframe
        let mut dec = vp8_decoder::Decoder::new();
        let first = frame();
        assert!(is_key(&first), "the first frame is a keyframe");
        assert!(dec.decode(&first).unwrap().is_some());
        for _ in 0..5 {
            let f = frame();
            assert!(!is_key(&f));
            assert!(
                dec.decode(&f).unwrap().is_some(),
                "the next frames decode too"
            );
        }
        s.keyframe();
        assert!((0..5).any(|_| is_key(&frame())), "a keyframe on request");
    }

    #[test]
    fn a_failed_capture_ends_the_share() {
        let open: OpenScreen = Box::new(|_| Err("No screen.".into()));
        let setup = ShareSetup {
            fps: 30,
            kbps: 500,
            sharp: true,
            active: true,
            source: None,
        };
        let s = Sending::start(open, setup);
        assert_eq!(
            s.out.recv_timeout(Duration::from_secs(5)),
            Ok(Out::Ended("No screen.".into()))
        );
    }

    /// Where a share's time goes, on a real picture (web/e2e/parity/image-viewer-side.png, from
    /// `desktop/scripts/parity.sh image-viewer`). `cargo test --release --lib -- --ignored share_bench --nocapture`.
    #[test]
    #[ignore]
    fn share_bench() {
        let png = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/e2e/parity/image-viewer-side.png"
        );
        let src = image::open(png)
            .expect("run desktop/scripts/parity.sh image-viewer first")
            .to_rgba8();
        let (iw, ih) = (src.width() as usize, src.height() as usize);
        for (sw, sh) in [(1920u32, 1080u32), (2560, 1440), (3840, 2160)] {
            for sharp in [true, false] {
                // Scrolling 8 px a frame, or still but for a blinking block; and a scroll on a tight budget.
                for (motion, step, kbps) in [
                    ("scroll", 8usize, 8_000),
                    ("still", 0, 8_000),
                    ("scroll", 8, 2_000),
                ] {
                    let (w, h) = fit(sw, sh, sharp);
                    let mut enc = Vp8::new(w, h, kbps).unwrap();
                    let mut dec = vp8_decoder::Decoder::new();
                    let (mut conv, mut code, mut bytes, mut psnr, mut n, mut key) =
                        (Duration::ZERO, Duration::ZERO, 0, 0., 0., 0);
                    for f in 0..40 {
                        let mut data = vec![0u8; sw as usize * sh as usize * 4];
                        for (y, row) in data.chunks_exact_mut(sw as usize * 4).enumerate() {
                            let sy = (y + f * step) % ih;
                            for (x, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                                *p = src.get_pixel((x % iw) as u32, sy as u32).0;
                            }
                        }
                        if f % 2 == 0 {
                            data[..32 * 4].fill(255); // a blinking block, so a still screen still changes
                        }
                        let s = Shot {
                            width: sw,
                            height: sh,
                            stride: sw as usize * 4,
                            bpp: 4,
                            rgb: [0, 1, 2],
                            data,
                        };
                        let t = Instant::now();
                        let i420 = to_i420(&s, w, h);
                        let t2 = Instant::now();
                        let out = enc.encode(&i420, f as u64 * 3000, 3000, f == 0);
                        let t3 = Instant::now();
                        if f >= 5 {
                            (conv, code) = (conv + (t2 - t), code + (t3 - t2));
                        }
                        let Some(out) = out else { continue };
                        (bytes, n) = (bytes + out.len(), n + f64::from(f >= 5));
                        if f == 0 {
                            key = out.len();
                        }
                        let d = dec.decode(&out).unwrap().unwrap();
                        let (wu, hu) = (w as usize, h as usize);
                        let mut se = 0f64;
                        for y in 0..hu {
                            for x in 0..wu {
                                let e = f64::from(i420[y * wu + x])
                                    - f64::from(d.y[y * d.y_stride + x]);
                                se += e * e;
                            }
                        }
                        if f >= 5 {
                            psnr +=
                                10. * (255f64 * 255. / (se / (wu * hu) as f64).max(1e-9)).log10();
                        }
                    }
                    let (c, e) = (
                        conv.as_secs_f64() * 1000. / 35.,
                        code.as_secs_f64() * 1000. / 35.,
                    );
                    println!(
                        "BENCH {sw}x{sh} -> {w}x{h} {motion:6} {kbps:4} kbps convert+scale {c:5.1} ms  encode {e:5.1} ms  max fps {:5.1}  \
                         avg frame {:4} KB (budget at 30 fps {} KB, keyframe {} KB)  Y PSNR {:4.1} dB",
                        1000. / (c + e),
                        bytes / 40 / 1024,
                        kbps / 8 / 30 * 1000 / 1024,
                        key / 1024,
                        psnr / n
                    );
                }
            }
        }
    }
}
