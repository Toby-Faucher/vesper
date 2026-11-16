//! Watching screen shares: the call engine hands each watched share's VP8 frames to a thread of that share's own
//! (`vesper-video`), which decodes them (`vp8-decoder`, pure Rust) and turns each picture into BGRA, what GPUI draws.
//! A thread each, so several shares decode side by side.
//!
//! Each picture comes at the size the UI draws it (`Video::size`), never larger than sent: a 1440p share in a smaller
//! tile is scaled down here, so the UI thread uploads only what it shows.
//!
//! Only the newest picture waits for the UI. The next one goes out once the UI has dropped the last, so a slow UI
//! skips pictures instead of queueing them. Compressed frames cannot be skipped (the next ones predict from them),
//! so when a decoder falls `QUEUE` frames behind, the engine drops frames and asks the sharer for a keyframe.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Weak};
use std::time::Duration;

use vp8_decoder::{DecodeError, Decoder, Picture};

/// Compressed frames waiting for a decoder at most (half a second at 60 fps).
pub const QUEUE: usize = 30;

/// A decoded picture of `user_id`'s screen: BGRA, `width * height * 4` bytes, what GPUI's `RenderImage` holds.
#[derive(Clone, PartialEq)]
pub struct Frame {
    pub user_id: String,
    pub width: u32,
    pub height: u32,
    pub bgra: Arc<Vec<u8>>,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Frame({} {}x{})", self.user_id, self.width, self.height)
    }
}

/// From the engine to a share's decoder thread.
#[derive(Debug)]
enum In {
    /// One VP8 frame. `gap`: frames before it were lost or dropped, so only a keyframe decodes.
    Frame { data: Vec<u8>, gap: bool },
    /// The size the share is drawn at, in device pixels.
    Size(u32, u32),
}

/// From a decoder thread to the engine: ask `user_id` for a keyframe. `broken`: a frame did not decode, which
/// counts as the share struggling; otherwise the decoder only waits for its first keyframe.
#[derive(Debug, PartialEq)]
pub struct Need {
    pub user_id: String,
    pub broken: bool,
}

type Emit = Arc<dyn Fn(Frame) + Send + Sync>;

/// One watched share's decoder thread. Dropping it ends the thread.
struct Worker {
    tx: Sender<In>,
    /// Frames sent and not yet taken by the decoder.
    queued: Arc<AtomicUsize>,
}

/// The engine's end of the decoder threads. Dropping it ends them.
pub struct Video {
    emit: Emit,
    workers: HashMap<String, Worker>,
    /// The size each share is drawn at. It can come before the watch (the tile is drawn first).
    sizes: HashMap<String, (u32, u32)>,
    need_tx: Sender<Need>,
    pub needs: Receiver<Need>,
}

impl Video {
    /// `emit` gets each picture for the UI, from the share's own thread.
    pub fn start(emit: impl Fn(Frame) + Send + Sync + 'static) -> Video {
        let (need_tx, needs) = channel();
        Video {
            emit: Arc::new(emit),
            workers: HashMap::new(),
            sizes: HashMap::new(),
            need_tx,
            needs,
        }
    }

    /// Queues a frame of a watched share (others are ignored). False when its decoder is `QUEUE` frames behind: the
    /// frame was dropped.
    pub fn frame(&self, user_id: String, data: Vec<u8>, gap: bool) -> bool {
        let Some(w) = self.workers.get(&user_id) else {
            return true;
        };
        if w.queued.load(Ordering::Relaxed) >= QUEUE {
            return false;
        }
        w.queued.fetch_add(1, Ordering::Relaxed);
        let _ = w.tx.send(In::Frame { data, gap });
        true
    }

    /// Start decoding a share (a fresh decoder on a thread of its own, waiting for a keyframe), or stop and forget it.
    pub fn watch(&mut self, user_id: String, on: bool) {
        if !on {
            self.workers.remove(&user_id);
            return;
        }
        let (tx, rx) = channel();
        if let Some(&(w, h)) = self.sizes.get(&user_id) {
            let _ = tx.send(In::Size(w, h));
        }
        let queued = Arc::new(AtomicUsize::new(0));
        let (q, need, emit, id) = (
            queued.clone(),
            self.need_tx.clone(),
            self.emit.clone(),
            user_id.clone(),
        );
        std::thread::Builder::new()
            .name("vesper-video".into())
            .spawn(move || run(&id, &rx, &q, &need, &*emit))
            .expect("the video thread starts");
        self.workers.insert(user_id, Worker { tx, queued });
    }

    pub fn size(&mut self, user_id: String, width: u32, height: u32) {
        if let Some(w) = self.workers.get(&user_id) {
            let _ = w.tx.send(In::Size(width, height));
        }
        self.sizes.insert(user_id, (width, height));
    }
}

fn run(
    user_id: &str,
    rx: &Receiver<In>,
    queued: &AtomicUsize,
    need: &Sender<Need>,
    emit: &dyn Fn(Frame),
) {
    let mut decoder = Decoder::new();
    // The newest picture the UI has not had, as I420 with its size.
    let mut waiting: Option<(Vec<u8>, u32, u32)> = None;
    // The picture the UI holds. The next one goes once this is dropped.
    let mut shown: Weak<Vec<u8>> = Weak::new();
    // The size the UI draws it at; none until it says.
    let mut size = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(In::Size(w, h)) => size = Some((w, h)),
            Ok(In::Frame { data, gap }) => {
                queued.fetch_sub(1, Ordering::Relaxed);
                if gap {
                    decoder.reset();
                }
                let broken = match decoder.decode(&data) {
                    Ok(Some(p)) => {
                        if shown.strong_count() == 0 {
                            shown = hand_over(user_id, &p, size, emit);
                            waiting = None;
                        } else {
                            waiting = Some((p.to_i420(), p.width, p.height));
                        }
                        continue;
                    }
                    Ok(None) => continue, // a hidden frame: references only
                    Err(DecodeError::NeedKeyframe) => false,
                    Err(_) => true,
                };
                let user_id = user_id.into();
                let _ = need.send(Need { user_id, broken });
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // A picture held back while the UI had the last one.
        if shown.strong_count() == 0
            && let Some((i420, w, h)) = waiting.take()
            && let Some(p) = Picture::from_i420(&i420, w, h)
        {
            shown = hand_over(user_id, &p, size, emit);
        }
    }
}

fn hand_over(
    user_id: &str,
    p: &Picture,
    size: Option<(u32, u32)>,
    emit: &dyn Fn(Frame),
) -> Weak<Vec<u8>> {
    let (width, height) = scaled(p.width, p.height, size);
    let bgra = Arc::new(bgra(p, width, height));
    let shown = Arc::downgrade(&bgra);
    emit(Frame {
        user_id: user_id.into(),
        width,
        height,
        bgra,
    });
    shown
}

/// A `w` x `h` picture's size inside a box of `size`, at its own aspect ratio, never larger than it is.
pub fn scaled(w: u32, h: u32, size: Option<(u32, u32)>) -> (u32, u32) {
    let Some((bw, bh)) = size.filter(|&(bw, bh)| bw > 0 && bh > 0) else {
        return (w, h);
    };
    let s = (f64::from(bw) / f64::from(w)).min(f64::from(bh) / f64::from(h));
    if s >= 1. {
        return (w, h);
    }
    let r = |n: u32| ((f64::from(n) * s).round() as u32).max(1);
    (r(w), r(h))
}

/// I420 to BGRA with BT.601 limited range, VP8's colour space (RFC 6386 section 9.2), in integers, at `w` x `h`: a
/// smaller size takes the nearest source pixel, so the work follows the size drawn, not the size sent.
/// shortcut: nearest pixel, no filter; small text in a much smaller tile can shimmer. Average a block if it shows.
pub fn bgra(p: &Picture, w: u32, h: u32) -> Vec<u8> {
    let (sw, sh) = (p.width as usize, p.height as usize);
    let (w, h) = (w as usize, h as usize);
    let xs: Vec<usize> = (0..w)
        .map(|x| ((2 * x + 1) * sw / (2 * w)).min(sw - 1))
        .collect();
    let mut out = vec![255u8; w * h * 4];
    for (y, row) in out.chunks_exact_mut(w * 4).enumerate() {
        let sy = ((2 * y + 1) * sh / (2 * h)).min(sh - 1);
        let luma = &p.y[sy * p.y_stride..][..sw];
        let (u, v) = (&p.u[sy / 2 * p.uv_stride..], &p.v[sy / 2 * p.uv_stride..]);
        for (&x, px) in xs.iter().zip(row.as_chunks_mut::<4>().0.iter_mut()) {
            let c = 298 * (i32::from(luma[x]) - 16) + 128;
            let (d, e) = (i32::from(u[x / 2]) - 128, i32::from(v[x / 2]) - 128);
            let clip = |n: i32| (n >> 8).clamp(0, 255) as u8;
            px[0] = clip(c + 516 * d);
            px[1] = clip(c - 100 * d - 208 * e);
            px[2] = clip(c + 409 * e);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Instant;

    /// `n` frames of one colour (Y, U, V), a keyframe then interframes, from the pure Rust encoder.
    fn solid(w: u32, h: u32, yuv: (u8, u8, u8), n: usize) -> Vec<Vec<u8>> {
        let (cw, ch) = (w.div_ceil(2) as usize, h.div_ceil(2) as usize);
        let mut i420 = vec![yuv.0; (w * h) as usize];
        i420.extend(std::iter::repeat_n(yuv.1, cw * ch));
        i420.extend(std::iter::repeat_n(yuv.2, cw * ch));
        let mut enc =
            vp8_encoder::Encoder::new(vp8_encoder::EncoderConfig::new(w, h, 30, 500_000)).unwrap();
        let pic = Picture::from_i420(&i420, w, h).unwrap();
        (0..n)
            .filter_map(|_| enc.encode(&pic, false).unwrap())
            .map(|f| f.data)
            .collect()
    }

    /// The mean of each BGRA channel.
    fn mean(f: &Frame) -> [u32; 3] {
        let n = (f.width * f.height) as u64;
        let sum = |c: usize| {
            f.bgra
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| u64::from(p[c]))
                .sum::<u64>()
        };
        [0, 1, 2].map(|c| (sum(c) / n) as u32)
    }

    #[test]
    fn bt601_colours() {
        // Limited-range white, black and red (Y 81, U 90, V 240), one 2x2 block each.
        for (yuv, want) in [
            ((235, 128, 128), [255, 255, 255]),
            ((16, 128, 128), [0, 0, 0]),
            ((81, 90, 240), [0, 0, 255]),
        ] {
            let i420 = [yuv.0, yuv.0, yuv.0, yuv.0, yuv.1, yuv.2];
            let out = bgra(&Picture::from_i420(&i420, 2, 2).unwrap(), 2, 2);
            for px in out.as_chunks::<4>().0 {
                for (got, want) in px[..3].iter().zip(want) {
                    assert!(got.abs_diff(want) <= 2, "{yuv:?}: {px:?}");
                }
                assert_eq!(px[3], 255);
            }
        }
    }

    #[test]
    fn pictures_come_at_the_size_drawn() {
        assert_eq!(scaled(2560, 1440, Some((1280, 900))), (1280, 720));
        assert_eq!(scaled(2560, 1440, Some((1920, 1080))), (1920, 1080));
        assert_eq!(
            scaled(1280, 720, Some((2560, 1440))),
            (1280, 720),
            "never larger"
        );
        assert_eq!(scaled(1280, 720, None), (1280, 720));
        assert_eq!(scaled(1000, 10, Some((10, 10))), (10, 1));
        // Black on the left, white on the right: halved, one pixel of each.
        let i420 = [16, 16, 235, 235, 16, 16, 235, 235, 128, 128, 128, 128];
        let out = bgra(&Picture::from_i420(&i420, 4, 2).unwrap(), 2, 1);
        assert_eq!(out, [0, 0, 0, 255, 255, 255, 255, 255]);

        // The size can come before the watch, as the tile draws first.
        let got = Arc::new(Mutex::new(Vec::<Frame>::new()));
        let g = got.clone();
        let mut video = Video::start(move |f| g.lock().unwrap().push(f));
        video.size("ann".into(), 32, 32);
        video.watch("ann".into(), true);
        for f in solid(64, 48, (81, 90, 240), 2) {
            video.frame("ann".into(), f, false);
        }
        wait("a picture", || !got.lock().unwrap().is_empty());
        let f = got.lock().unwrap()[0].clone();
        assert_eq!((f.width, f.height, f.bgra.len()), (32, 24, 32 * 24 * 4));
    }

    fn wait(what: &str, done: impl Fn() -> bool) {
        let end = Instant::now() + Duration::from_secs(10);
        while !done() {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn decodes_only_watched_shares_and_keeps_only_the_newest_picture() {
        let got = Arc::new(Mutex::new(Vec::<Frame>::new()));
        let g = got.clone();
        let mut video = Video::start(move |f| g.lock().unwrap().push(f));
        let frames = solid(64, 48, (81, 90, 240), 10);

        // Not watched: nothing is decoded.
        video.frame("ann".into(), frames[0].clone(), false);
        std::thread::sleep(Duration::from_millis(50));
        assert!(got.lock().unwrap().is_empty());

        // Watched: red pictures of the right size. The first is held by the "UI" here, so the rest wait.
        video.watch("ann".into(), true);
        for f in &frames {
            assert!(video.frame("ann".into(), f.clone(), false));
        }
        wait("the first picture", || !got.lock().unwrap().is_empty());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            got.lock().unwrap().len(),
            1,
            "one picture while the UI holds it"
        );
        let first = got.lock().unwrap()[0].clone();
        assert_eq!((first.width, first.height), (64, 48));
        let [b, g, r] = mean(&first);
        assert!(r > 200 && g < 50 && b < 50, "red: {r} {g} {b}");

        // The UI drops it: the newest one comes, and only it.
        got.lock().unwrap().clear();
        drop(first);
        wait("the newest picture", || !got.lock().unwrap().is_empty());
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(got.lock().unwrap().len(), 1);

        // An interframe after a gap waits for a keyframe and asks for one.
        got.lock().unwrap().clear();
        video.frame("ann".into(), frames[5].clone(), true);
        let need = video.needs.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            need,
            Need {
                user_id: "ann".into(),
                broken: false
            }
        );
        video.frame("ann".into(), b"not vp8".to_vec(), false);
        assert!(
            video
                .needs
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .user_id
                == "ann"
        );

        // Unwatched: frames are ignored again.
        video.watch("ann".into(), false);
        video.frame("ann".into(), frames[0].clone(), false);
        std::thread::sleep(Duration::from_millis(50));
        assert!(got.lock().unwrap().is_empty());
    }

    #[test]
    fn a_full_queue_drops_frames() {
        // Nothing drains this queue: the thread is stuck in emit.
        let (stuck_tx, stuck) = std::sync::mpsc::channel::<()>();
        let stuck = Mutex::new(stuck);
        let mut video = Video::start(move |_| {
            let _ = stuck.lock().unwrap().recv();
        });
        let frames = solid(16, 16, (128, 128, 128), 1);
        video.watch("ann".into(), true);
        video.frame("ann".into(), frames[0].clone(), false);
        std::thread::sleep(Duration::from_millis(100));
        let taken = (0..QUEUE + 5)
            .filter(|_| video.frame("ann".into(), frames[0].clone(), false))
            .count();
        assert_eq!(taken, QUEUE);
        drop(stuck_tx);
    }
}
