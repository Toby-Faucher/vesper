//! Sending my screen share. A capture thread (`vesper-capture`) takes pictures of the screen (`capture.rs`) and keeps
//! only the newest. An encode thread (`vesper-encode`) turns it into VP8 (`vp8-encoder`, pure Rust) at the share's
//! frame rate and bitrate, and hands each frame to the call engine, which sends it on its video m-line. Neither runs
//! on the UI or the call's thread, so a slow encode costs frames, never voice.
//!
//! The encoder runs only while someone watches (`ShareSetup::active`, from the server's watcher count), as the web
//! turns its encoding off. A still screen gives no new pictures on Wayland and Windows, so the newest one is kept
//! and encoded again when a viewer asks for a keyframe.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use vp8_encoder::{Encoder, EncoderConfig, Picture, scale_i420};

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
}

/// The most pixels sent: 1080p for sharp text, 720p for smooth motion. A bigger screen is scaled down to fit.
/// shortcut: the pure Rust encoder is single threaded (about 20 fps at 1080p on one core); a hardware or libvpx
/// encoder would allow more, if a 1080p share is too slow for the group.
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
    let mut enc: Option<Encoder> = None;
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
        let Some(pic) = Picture::from_i420(&i420, w, h) else {
            continue;
        };
        let bps = setup.kbps.saturating_mul(1000);
        let same = enc
            .as_ref()
            .is_some_and(|e| e.config().width == w && e.config().height == h);
        if !same {
            enc = Encoder::new(EncoderConfig::new(w, h, setup.fps, bps)).ok();
        }
        let Some(e) = enc.as_mut() else {
            continue;
        };
        if e.config().bitrate_bps != bps || e.config().fps != setup.fps {
            e.set_rates(bps, setup.fps);
        }
        shared.keyframe.store(false, Ordering::Relaxed);
        let Ok(Some(f)) = e.encode(&pic, key) else {
            continue; // skipped by rate control
        };
        (frames, bytes) = (frames + 1, bytes + f.data.len());
        let rtp = (now - start).as_micros() as u64 * 90 / 1000;
        if out.send(Out::Frame { data: f.data, rtp }).is_err() {
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

/// The size to send `w` x `h` at: as it is up to the preference's pixels, else scaled down to fit, kept even.
pub fn fit(w: u32, h: u32, sharp: bool) -> (u32, u32) {
    let most = if sharp { SHARP_PIXELS } else { SMOOTH_PIXELS };
    let px = u64::from(w) * u64::from(h);
    if px <= u64::from(most) {
        return (w.max(1), h.max(1));
    }
    let k = (f64::from(most) / px as f64).sqrt();
    let even = |n: u32| ((f64::from(n) * k) as u32 & !1).max(2);
    (even(w), even(h))
}

/// The shot as I420 of `w` x `h`, BT.601 limited range (what the viewers' VP8 decoders turn back into RGB).
pub fn to_i420(s: &Shot, w: u32, h: u32) -> Vec<u8> {
    let (sw, sh) = (s.width as usize, s.height as usize);
    let (cw, ch) = (sw.div_ceil(2), sh.div_ceil(2));
    let mut out = vec![0u8; sw * sh + 2 * cw * ch];
    let (y_plane, uv) = out.split_at_mut(sw * sh);
    let (u_plane, v_plane) = uv.split_at_mut(cw * ch);
    let [r, g, b] = s.rgb;
    let px = |x: usize, y: usize| {
        let i = y * s.stride + x * s.bpp;
        s.data.get(i..i + s.bpp).map_or([0; 3], |p| {
            [i32::from(p[r]), i32::from(p[g]), i32::from(p[b])]
        })
    };
    for y in 0..sh {
        for x in 0..sw {
            let [r, g, b] = px(x, y);
            y_plane[y * sw + x] = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
        }
    }
    for cy in 0..ch {
        for cx in 0..cw {
            // The mean of the 2x2 block (an odd edge repeats its last pixel).
            let mut sum = [0; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let p = px((cx * 2 + dx).min(sw - 1), (cy * 2 + dy).min(sh - 1));
                (0..3).for_each(|i| sum[i] += p[i]);
            }
            let [r, g, b] = sum.map(|c| c / 4);
            u_plane[cy * cw + cx] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128) as u8;
            v_plane[cy * cw + cx] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128) as u8;
        }
    }
    if (w, h) == (s.width, s.height) {
        return out;
    }
    let mut scaled = Vec::new();
    if let Some(p) = Picture::from_i420(&out, s.width, s.height) {
        scale_i420(&p, w, h, &mut scaled);
    }
    scaled
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
        let mut enc = Encoder::new(EncoderConfig::new(64, 16, 30, 2_000_000)).unwrap();
        let f = enc
            .encode(&Picture::from_i420(&i420, 64, 16).unwrap(), true)
            .unwrap()
            .unwrap();
        let mut dec = vp8_decoder::Decoder::new();
        let pic = dec.decode(&f.data).unwrap().unwrap();
        let bgra = super::super::video::bgra(&pic);
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
    fn frames_flow_only_while_watched_and_a_keyframe_comes_on_request() {
        let open: OpenScreen =
            Box::new(|fps| Ok(Box::new(Pattern::new(64, 32, fps)) as Box<dyn Screen>));
        let mut setup = ShareSetup {
            fps: 30,
            kbps: 500,
            sharp: true,
            active: false,
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
        };
        let s = Sending::start(open, setup);
        assert_eq!(
            s.out.recv_timeout(Duration::from_secs(5)),
            Ok(Out::Ended("No screen.".into()))
        );
    }
}
