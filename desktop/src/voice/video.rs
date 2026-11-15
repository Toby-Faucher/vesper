//! Watching screen shares: the call engine hands each watched share's VP8 frames to a thread of their own
//! (`vesper-video`), which decodes them (`vp8-decoder`, pure Rust) and turns each picture into BGRA, what GPUI draws.
//!
//! Only the newest picture waits for the UI. The next one goes out once the UI has dropped the last, so a slow UI
//! skips pictures instead of queueing them. Compressed frames cannot be skipped (the next ones predict from them),
//! so when the decoder falls `QUEUE` frames behind, the engine drops frames and asks the sharer for a keyframe.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Weak};
use std::time::Duration;

use vp8_decoder::{DecodeError, Decoder, Picture};

/// Compressed frames waiting for the decoder at most (half a second at 60 fps).
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

/// From the engine to the decoder thread.
#[derive(Debug)]
pub enum In {
    /// One VP8 frame. `gap`: frames before it were lost or dropped, so only a keyframe decodes.
    Frame {
        user_id: String,
        data: Vec<u8>,
        gap: bool,
    },
    /// Start decoding a share (a fresh decoder, waiting for a keyframe) or stop and forget it.
    Watch { user_id: String, on: bool },
}

/// From the decoder thread to the engine: ask `user_id` for a keyframe. `broken`: a frame did not decode, which
/// counts as the share struggling; otherwise the decoder only waits for its first keyframe.
#[derive(Debug, PartialEq)]
pub struct Need {
    pub user_id: String,
    pub broken: bool,
}

/// The engine's end of the decoder thread. Dropping it ends the thread.
pub struct Video {
    tx: Sender<In>,
    /// Frames sent and not yet taken by the decoder.
    queued: Arc<AtomicUsize>,
    pub needs: Receiver<Need>,
}

impl Video {
    /// Starts the decoder thread. `emit` gets each picture for the UI.
    pub fn start(emit: impl Fn(Frame) + Send + 'static) -> Video {
        let (tx, rx) = channel();
        let (need_tx, needs) = channel();
        let queued = Arc::new(AtomicUsize::new(0));
        let q = queued.clone();
        std::thread::Builder::new()
            .name("vesper-video".into())
            .spawn(move || run(&rx, &q, &need_tx, &emit))
            .expect("the video thread starts");
        Video { tx, queued, needs }
    }

    /// Queues a frame. False when the decoder is `QUEUE` frames behind: the frame was dropped.
    pub fn frame(&self, user_id: String, data: Vec<u8>, gap: bool) -> bool {
        if self.queued.load(Ordering::Relaxed) >= QUEUE {
            return false;
        }
        self.queued.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(In::Frame { user_id, data, gap });
        true
    }

    pub fn watch(&self, user_id: String, on: bool) {
        let _ = self.tx.send(In::Watch { user_id, on });
    }
}

struct Share {
    decoder: Decoder,
    /// The newest picture the UI has not had, as I420 with its size.
    waiting: Option<(Vec<u8>, u32, u32)>,
    /// The picture the UI holds. The next one goes once this is dropped.
    shown: Weak<Vec<u8>>,
}

fn run(rx: &Receiver<In>, queued: &AtomicUsize, need: &Sender<Need>, emit: &dyn Fn(Frame)) {
    let mut shares: HashMap<String, Share> = HashMap::new();
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(In::Watch { user_id, on: true }) => {
                let share = Share {
                    decoder: Decoder::new(),
                    waiting: None,
                    shown: Weak::new(),
                };
                shares.insert(user_id, share);
            }
            Ok(In::Watch { user_id, on: false }) => {
                shares.remove(&user_id);
            }
            Ok(In::Frame { user_id, data, gap }) => {
                queued.fetch_sub(1, Ordering::Relaxed);
                let Some(s) = shares.get_mut(&user_id) else {
                    continue;
                };
                if gap {
                    s.decoder.reset();
                }
                let broken = match s.decoder.decode(&data) {
                    Ok(Some(p)) => {
                        if s.shown.strong_count() == 0 {
                            s.shown = hand_over(&user_id, &p, emit);
                            s.waiting = None;
                        } else {
                            s.waiting = Some((p.to_i420(), p.width, p.height));
                        }
                        continue;
                    }
                    Ok(None) => continue, // a hidden frame: references only
                    Err(DecodeError::NeedKeyframe) => false,
                    Err(_) => true,
                };
                let _ = need.send(Need { user_id, broken });
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        // A picture held back while the UI had the last one.
        for (id, s) in &mut shares {
            if s.shown.strong_count() == 0
                && let Some((i420, w, h)) = s.waiting.take()
                && let Some(p) = Picture::from_i420(&i420, w, h)
            {
                s.shown = hand_over(id, &p, emit);
            }
        }
    }
}

fn hand_over(user_id: &str, p: &Picture, emit: &dyn Fn(Frame)) -> Weak<Vec<u8>> {
    let bgra = Arc::new(bgra(p));
    let shown = Arc::downgrade(&bgra);
    emit(Frame {
        user_id: user_id.into(),
        width: p.width,
        height: p.height,
        bgra,
    });
    shown
}

/// I420 to BGRA with BT.601 limited range, VP8's colour space (RFC 6386 section 9.2), in integers.
pub fn bgra(p: &Picture) -> Vec<u8> {
    let (w, h) = (p.width as usize, p.height as usize);
    let mut out = vec![255u8; w * h * 4];
    for (y, row) in out.chunks_exact_mut(w * 4).enumerate() {
        let luma = &p.y[y * p.y_stride..][..w];
        let (u, v) = (&p.u[y / 2 * p.uv_stride..], &p.v[y / 2 * p.uv_stride..]);
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
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
            let out = bgra(&Picture::from_i420(&i420, 2, 2).unwrap());
            for px in out.as_chunks::<4>().0 {
                for (got, want) in px[..3].iter().zip(want) {
                    assert!(got.abs_diff(want) <= 2, "{yuv:?}: {px:?}");
                }
                assert_eq!(px[3], 255);
            }
        }
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
        let video = Video::start(move |f| g.lock().unwrap().push(f));
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
        let video = Video::start(move |_| {
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
