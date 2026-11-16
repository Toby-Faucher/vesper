//! What runs on the microphone between capture and Opus: DeepFilterNet (`vesper-dsp`, native tract), RNNoise
//! (`vesper-dsp`, model built in), the web's noise gate, or nothing. Like the web app, either denoiser falls back to the
//! gate when it cannot keep up (`Late`, the web's `ModeController`), and the call goes on with the gate while
//! DeepFilterNet's model loads or when it is missing.
//!
//! The model is not in the binary: it comes from the signed-in server's `/dsp/model.tar.gz` (the same file the web app
//! loads, the upstream DeepFilterNet3 tar.gz that tract reads natively), is checked against the pinned sha256, and is
//! kept in the OS data dir.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::settings::Denoise;

/// The same pin as `scripts/fetch-model.sh` (a test checks they match).
pub const MODEL_SHA256: &str = "c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616";
pub const MODEL_FILE: &str = "DeepFilterNet3_onnx.tar.gz";
/// Where the web app loads it from, on the same server.
pub const MODEL_URL: &str = "/dsp/model.tar.gz";
/// A larger answer is not the model (it is 8 MB).
pub const MODEL_MAX: usize = 32 << 20;
/// DeepFilterNet's frame: 10 ms at 48 kHz.
const HOP: usize = vesper_dsp::HOP;

pub const LOADING: &str =
    "Noise filter: loading DeepFilterNet. The noise gate runs until it is ready.";
pub const DEEPFILTER: &str = "Noise filter: DeepFilterNet.";
pub const RNNOISE: &str = "Noise filter: RNNoise.";
pub const GATE: &str = "Noise filter: noise gate.";
pub const OFF: &str = "Noise filter: off.";
pub const SLOW: &str = "DeepFilterNet could not keep up on this computer, so the noise gate runs for the rest of the call.";
pub const RNNOISE_SLOW: &str =
    "RNNoise could not keep up on this computer, so the noise gate runs for the rest of the call.";

pub fn unavailable(why: &str) -> String {
    format!("DeepFilterNet is not available ({why}), so the noise gate runs for this call.")
}

fn rnnoise_unavailable(why: &str) -> String {
    format!("RNNoise is not available ({why}), so the noise gate runs for this call.")
}

/// A denoiser on 10 ms frames. `vesper_dsp::Denoiser` in the app; tests use fakes. Not `Send`: tract's state is not,
/// so it is built and run on one worker thread.
pub trait Filter {
    fn process(&mut self, frame: &[f32]) -> Result<Vec<f32>, String>;
}

impl Filter for vesper_dsp::Denoiser {
    fn process(&mut self, frame: &[f32]) -> Result<Vec<f32>, String> {
        vesper_dsp::Denoiser::process(self, frame).map_err(|e| e.to_string())
    }
}

impl Filter for vesper_dsp::Rnnoise {
    fn process(&mut self, frame: &[f32]) -> Result<Vec<f32>, String> {
        vesper_dsp::Rnnoise::process(self, frame).map_err(|e| e.to_string())
    }
}

/// The checked model bytes, or why there are none. Sent to the call when ready.
pub type Loaded = Result<Vec<u8>, String>;
/// Builds the denoiser from the model bytes.
pub type Build = fn(&[u8]) -> Result<Box<dyn Filter>, String>;

/// DeepFilterNet with the web's defaults: no attenuation limit, post filter 0.02.
pub fn deep_filter(model: &[u8]) -> Result<Box<dyn Filter>, String> {
    vesper_dsp::Denoiser::new(model, 100.0, 0.02)
        .map(|d| Box::new(d) as Box<dyn Filter>)
        .map_err(|e| e.to_string())
}

/// RNNoise ignores the model bytes: its model is in the binary.
pub fn rnnoise(_: &[u8]) -> Result<Box<dyn Filter>, String> {
    Ok(Box::new(vesper_dsp::Rnnoise::default()))
}

/// Trips once `max_late` of the last `window` frames took longer than `budget`, after `warmup` frames; one way for the
/// rest of the call. The web's `ModeController`. The numbers differ: the web budget (30 ms) is the depth of its
/// output queue, while here a frame must be done in its own 10 ms or the call thread falls behind.
#[derive(Debug)]
pub struct Late {
    budget: Duration,
    window: usize,
    max_late: usize,
    warmup: usize,
    seen: usize,
    late: VecDeque<bool>,
    count: usize,
    tripped: bool,
}

impl Late {
    pub fn new(budget: Duration, window: usize, max_late: usize, warmup: usize) -> Late {
        Late {
            budget,
            window,
            max_late,
            warmup,
            seen: 0,
            late: VecDeque::new(),
            count: 0,
            tripped: false,
        }
    }

    /// One frame's time. True once tripped.
    pub fn report(&mut self, took: Duration) -> bool {
        if self.seen < self.warmup {
            self.seen += 1;
        } else if !self.tripped {
            let l = took > self.budget;
            self.late.push_back(l);
            self.count += usize::from(l);
            if self.late.len() > self.window && self.late.pop_front() == Some(true) {
                self.count -= 1;
            }
            self.tripped = self.count >= self.max_late;
        }
        self.tripped
    }
}

impl Default for Late {
    /// Over the last second (100 frames), 20 late ones (200 ms of slow audio) trip it; the first 0.5 s is skipped
    /// while the model's buffers are first touched.
    fn default() -> Late {
        Late::new(Duration::from_millis(10), 100, 20, 50)
    }
}

fn db(x: f32) -> f32 {
    10f32.powf(x / 20.0)
}

/// The web's noise gate (`dynamics.ts`): opens on a 10 ms frame whose peak is above -50 dBFS, closes after 200 ms
/// of frames below 4 dB under that, with linear 2 ms and 120 ms gain ramps so it never clicks. The web runs it with
/// its output queue as lookahead; here it opens on the first loud frame itself.
#[derive(Debug)]
pub struct Gate {
    open: f32,
    close: f32,
    quiet: usize,
    target: f32,
    gain: f32,
}

const GATE_HOLD: usize = 48_000 / 5;
const GATE_UP: f32 = 1.0 / (0.002 * 48_000.0);
const GATE_DOWN: f32 = 1.0 / (0.12 * 48_000.0);

impl Default for Gate {
    fn default() -> Gate {
        Gate {
            open: db(-50.0),
            close: db(-54.0),
            quiet: 0,
            target: 1.0, // starts open, so the first word is not cut
            gain: 1.0,
        }
    }
}

impl Gate {
    pub fn process(&mut self, frame: &mut [f32]) {
        let peak = frame.iter().fold(0f32, |p, x| p.max(x.abs()));
        if peak > self.open {
            self.target = 1.0;
        }
        if peak >= self.close {
            self.quiet = 0;
        } else {
            self.quiet += frame.len();
            if self.quiet >= GATE_HOLD {
                self.target = 0.0;
            }
        }
        for x in frame {
            self.gain = if self.target > self.gain {
                (self.gain + GATE_UP).min(1.0)
            } else {
                (self.gain - GATE_DOWN).max(self.target)
            };
            *x *= self.gain;
        }
    }
}

/// A frame that takes longer than this is not waited for: the worker is stuck, and the call falls back.
const STUCK: Duration = Duration::from_millis(100);

/// DeepFilterNet on its own thread. tract's state cannot move between threads and building it takes about 2 s in a
/// debug build, so the worker builds it and keeps it; the call thread hands it each 10 ms frame and waits for the
/// answer. The worker ends when this is dropped.
struct Worker {
    frames: Sender<Vec<f32>>,
    out: Receiver<Result<Vec<f32>, String>>,
    ready: bool,
}

impl Worker {
    fn spawn(loading: Receiver<Loaded>, build: Build) -> Worker {
        let (frames, frames_rx) = std::sync::mpsc::channel::<Vec<f32>>();
        let (out_tx, out) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("vesper-denoise".into())
            .spawn(move || {
                let filter = match loading.recv() {
                    Ok(Ok(model)) => build(&model),
                    Ok(Err(why)) => Err(why),
                    Err(_) => Err("it did not load".into()),
                };
                let mut f = match filter {
                    Ok(f) => f,
                    Err(why) => return drop(out_tx.send(Err(why))),
                };
                let _ = out_tx.send(Ok(vec![])); // ready
                while let Ok(frame) = frames_rx.recv() {
                    if out_tx.send(f.process(&frame)).is_err() {
                        return;
                    }
                }
            });
        Worker {
            frames,
            out,
            ready: false,
        }
    }
}

/// The capture filter of one call.
pub struct Capture {
    deep: Option<Worker>,
    /// What the worker runs: true for RNNoise, false for DeepFilterNet. Only picks the status texts.
    light: bool,
    gate: Option<Gate>,
    late: Late,
    status: String,
    shown: bool,
    /// DeepFilterNet frames so far and their time (with the hand-over), for the tests.
    pub frames: u32,
    pub total: Duration,
}

impl Capture {
    /// `loading` brings the model for DeepFilterNet and `build` makes the denoiser from it, on the worker. Until it
    /// is ready, and after a fall back, the noise gate runs. RNNoise uses `build` with no model (the app passes
    /// [`rnnoise`]); the other modes ignore both.
    pub fn new(mode: Denoise, loading: Receiver<Loaded>, build: Build, late: Late) -> Capture {
        let (deep, gate, status) = match mode {
            Denoise::DeepFilter => (
                Some(Worker::spawn(loading, build)),
                Some(Gate::default()),
                LOADING,
            ),
            Denoise::Rnnoise => {
                let (tx, rx) = std::sync::mpsc::channel();
                let _ = tx.send(Ok(vec![]));
                (
                    Some(Worker::spawn(rx, build)),
                    Some(Gate::default()),
                    RNNOISE,
                )
            }
            Denoise::Gate => (None, Some(Gate::default()), GATE),
            Denoise::Off => (None, None, OFF),
        };
        Capture {
            deep,
            light: mode == Denoise::Rnnoise,
            gate,
            late,
            status: status.into(),
            shown: false,
            frames: 0,
            total: Duration::ZERO,
        }
    }

    /// The noise gate opens on a peak above `threshold` dBFS (the level marker) and closes 4 dB under it.
    pub fn set_gate(&mut self, threshold: f32) {
        if let Some(g) = &mut self.gate {
            (g.open, g.close) = (db(threshold), db(threshold - 4.0));
        }
    }

    fn fall_back(&mut self, status: String) {
        self.deep = None;
        self.status = status;
        self.shown = false;
    }

    fn unavailable(&self, why: &str) -> String {
        if self.light {
            rnnoise_unavailable(why)
        } else {
            unavailable(why)
        }
    }

    fn slow(&self) -> String {
        (if self.light { RNNOISE_SLOW } else { SLOW }).into()
    }

    /// Filters `frame` (a multiple of 10 ms) in place, unless it is not `live` (muted: not sent). Returns the status
    /// text when it changed.
    pub fn process(&mut self, frame: &mut [f32], live: bool) -> Option<String> {
        if let Some(w) = self.deep.as_mut().filter(|w| !w.ready) {
            match w.out.try_recv() {
                Ok(Ok(_)) => {
                    w.ready = true;
                    if !self.light {
                        (self.status, self.shown) = (DEEPFILTER.into(), false);
                    }
                }
                Ok(Err(why)) => self.fall_back(self.unavailable(&why)),
                Err(TryRecvError::Disconnected) => self.fall_back(self.unavailable("it stopped")),
                Err(TryRecvError::Empty) => {}
            }
        }
        let frame = if live { frame } else { &mut [] };
        for chunk in frame.chunks_mut(HOP) {
            if let Some(w) = self.deep.as_ref().filter(|w| w.ready) {
                let t = Instant::now();
                let _ = w.frames.send(chunk.to_vec());
                match w.out.recv_timeout(STUCK) {
                    Ok(Ok(out)) if out.len() == chunk.len() => chunk.copy_from_slice(&out),
                    Ok(Ok(_)) => self.fall_back(self.unavailable("a wrong frame size")),
                    Ok(Err(e)) => self.fall_back(self.unavailable(&e)),
                    Err(_) => self.fall_back(self.slow()),
                }
                let took = t.elapsed();
                (self.frames, self.total) = (self.frames + 1, self.total + took);
                if self.late.report(took) && self.deep.is_some() {
                    self.fall_back(self.slow());
                }
                if self.deep.is_some() {
                    continue;
                }
            }
            if let Some(g) = &mut self.gate {
                g.process(chunk);
            }
        }
        (!std::mem::replace(&mut self.shown, true)).then(|| self.status.clone())
    }
}

/// An error if `bytes` is not the pinned model.
pub fn verify(bytes: &[u8]) -> Result<(), String> {
    let got: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got == MODEL_SHA256 {
        Ok(())
    } else {
        Err("the model's checksum does not match".into())
    }
}

/// `<data dir>/Vesper/DeepFilterNet3_onnx.tar.gz`.
pub fn cache_path() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join("Vesper").join(MODEL_FILE))
}

fn store(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// The model from the cache if it is there and checks out, else from `fetch` (checked, then cached). File work runs
/// off the async thread.
pub async fn load(
    path: Option<PathBuf>,
    fetch: impl Future<Output = Result<Vec<u8>, String>>,
) -> Loaded {
    let p = path.clone();
    let cached = tokio::task::spawn_blocking(move || {
        p.and_then(|p| std::fs::read(p).ok())
            .filter(|b| verify(b).is_ok())
    })
    .await
    .ok()
    .flatten();
    Ok(match cached {
        Some(b) => b,
        None => {
            let b = fetch.await?;
            verify(&b)?;
            if let Some(p) = path {
                let copy = b.clone();
                // Not cached is not fatal: the next call downloads it again.
                let _ = tokio::task::spawn_blocking(move || store(&p, &copy)).await;
            }
            b
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    const FRAME: usize = 960;

    /// xorshift white noise, uniform, with this RMS.
    struct Noise(u64);
    impl Noise {
        fn next(&mut self, rms: f32) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            ((self.0 >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * rms * 3f32.sqrt()
        }
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|s| f64::from(*s).powi(2)).sum()
    }

    fn rx(v: Option<Loaded>) -> Receiver<Loaded> {
        let (tx, rx) = mpsc::channel();
        if let Some(v) = v {
            tx.send(v).unwrap();
        } else {
            std::mem::forget(tx); // still loading
        }
        rx
    }

    /// Sleeps `ms` per frame and passes the audio through.
    struct Slow(u64);
    impl Filter for Slow {
        fn process(&mut self, frame: &[f32]) -> Result<Vec<f32>, String> {
            std::thread::sleep(Duration::from_millis(self.0));
            Ok(frame.to_vec())
        }
    }

    #[test]
    fn late_trips_after_warmup_and_stays() {
        let mut l = Late::new(Duration::from_millis(10), 10, 3, 2);
        let (slow, fast) = (Duration::from_millis(11), Duration::from_millis(1));
        assert!(!l.report(slow) && !l.report(slow), "warmup is ignored");
        for _ in 0..20 {
            assert!(!l.report(fast));
        }
        assert!(!l.report(slow) && !l.report(slow));
        for _ in 0..9 {
            l.report(fast); // the two late ones leave the window
        }
        assert!(!l.report(slow) && !l.report(slow));
        assert!(l.report(slow), "3 of the last 10");
        assert!(l.report(fast), "one way");
    }

    /// Processes frames until the worker has answered: the status after loading.
    fn settle(c: &mut Capture, frame: &mut [f32]) -> String {
        let t = Instant::now();
        loop {
            match c.process(frame, true) {
                Some(s) if s != LOADING => return s,
                _ => assert!(t.elapsed() < Duration::from_secs(30), "the worker answers"),
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_slow_filter_falls_back_to_the_gate_and_says_why() {
        let mut c = Capture::new(
            Denoise::DeepFilter,
            rx(Some(Ok(vec![]))),
            |_| Ok(Box::new(Slow(12))),
            Late::new(Duration::from_millis(10), 10, 3, 0),
        );
        let mut frame = [0.1f32; FRAME];
        assert_eq!(settle(&mut c, &mut frame), DEEPFILTER);
        assert_eq!(
            c.process(&mut frame, true).as_deref(),
            Some(SLOW),
            "the third late frame"
        );
        assert!(c.deep.is_none());
        let t = Instant::now();
        assert_eq!(c.process(&mut frame, true), None);
        assert!(
            t.elapsed() < Duration::from_millis(10),
            "the slow filter is gone"
        );
    }

    #[test]
    fn modes_and_a_missing_model() {
        let mut frame = [0.0f32; FRAME];
        let mut c = Capture::new(Denoise::DeepFilter, rx(None), deep_filter, Late::default());
        assert_eq!(c.process(&mut frame, true).as_deref(), Some(LOADING));
        assert_eq!(c.process(&mut frame, true), None);
        let mut c = Capture::new(
            Denoise::DeepFilter,
            rx(Some(Err("no answer".into()))),
            deep_filter,
            Late::default(),
        );
        assert_eq!(settle(&mut c, &mut frame), unavailable("no answer"));
        let mut c = Capture::new(
            Denoise::DeepFilter,
            rx(Some(Ok(b"junk".to_vec()))),
            |_| Err("bad model".into()),
            Late::default(),
        );
        assert_eq!(settle(&mut c, &mut frame), unavailable("bad model"));
        assert!(c.deep.is_none());
        let mut c = Capture::new(Denoise::Gate, rx(None), deep_filter, Late::default());
        assert_eq!(c.process(&mut frame, true).as_deref(), Some(GATE));

        // Off passes the audio as it is.
        let mut c = Capture::new(Denoise::Off, rx(None), deep_filter, Late::default());
        let mut n = Noise(7);
        let input: Vec<f32> = (0..FRAME).map(|_| n.next(0.001)).collect();
        let mut x = input.clone();
        assert_eq!(c.process(&mut x, true).as_deref(), Some(OFF));
        assert_eq!(x, input);
    }

    /// RNNoise needs no model: it needs no download, takes out white noise, and says so once. Its fallbacks name it.
    #[test]
    fn rnnoise_runs_without_a_model_and_takes_out_noise() {
        let mut c = Capture::new(
            Denoise::Rnnoise,
            rx(None),
            rnnoise,
            Late::new(Duration::from_secs(10), 100, 20, 0),
        );
        // The worker thread starts it; on a busy machine that takes a moment, and the gate runs meanwhile.
        let mut statuses: Vec<String> = c.process(&mut [0.0; FRAME], true).into_iter().collect();
        let t = Instant::now();
        while c.frames == 0 {
            assert!(t.elapsed() < Duration::from_secs(30), "the worker answers");
            std::thread::sleep(Duration::from_millis(1));
            statuses.extend(c.process(&mut [0.0; FRAME], true));
        }
        let mut n = Noise(11);
        let (mut ein, mut eout) = (0f64, 0f64);
        for i in 0..100 {
            let mut x: Vec<f32> = (0..FRAME).map(|_| n.next(0.02)).collect();
            let e = energy(&x);
            statuses.extend(c.process(&mut x, true));
            if i >= 50 {
                (ein, eout) = (ein + e, eout + energy(&x));
            }
        }
        assert_eq!(statuses, [RNNOISE]);
        let down = 10.0 * (ein / eout).log10();
        println!(
            "RNNoise in the call filter: white noise down {down:.1} dB, {:.3} ms per 10 ms frame (this build)",
            c.total.as_secs_f64() * 1000.0 / f64::from(c.frames)
        );
        assert!(down > 6.0, "white noise down by over 6 dB, got {down:.1}");

        let mut c = Capture::new(
            Denoise::Rnnoise,
            rx(None),
            |_| Err("broken".into()),
            Late::default(),
        );
        let mut said = vec![];
        let t = Instant::now();
        while said.len() < 2 && t.elapsed() < Duration::from_secs(10) {
            said.extend(c.process(&mut [0.0; FRAME], true));
        }
        assert_eq!(said, [RNNOISE.into(), rnnoise_unavailable("broken")]);
        let mut c = Capture::new(
            Denoise::Rnnoise,
            rx(None),
            |_| Ok(Box::new(Slow(12))),
            Late::new(Duration::from_millis(10), 10, 3, 0),
        );
        let mut frame = [0.1f32; FRAME];
        let mut said = vec![];
        let t = Instant::now();
        while said.len() < 2 && t.elapsed() < Duration::from_secs(10) {
            said.extend(c.process(&mut frame, true));
        }
        assert_eq!(said, [RNNOISE, RNNOISE_SLOW]);
    }

    #[test]
    fn the_gate_closes_on_quiet_and_opens_on_a_word() {
        let mut g = Gate::default();
        let mut n = Noise(3);
        let quiet = |n: &mut Noise| -> Vec<f32> { (0..480).map(|_| n.next(0.001)).collect() };
        for _ in 0..19 {
            let mut f = quiet(&mut n);
            g.process(&mut f);
        }
        let mut f = quiet(&mut n);
        let e = energy(&f);
        g.process(&mut f);
        assert!(energy(&f) > 0.5 * e, "held for 200 ms");
        for _ in 0..20 {
            let mut f = quiet(&mut n);
            g.process(&mut f);
        }
        let mut f = quiet(&mut n);
        g.process(&mut f);
        assert_eq!(energy(&f), 0.0, "closed after the release ramp");
        let mut word: Vec<f32> = (0..480).map(|i| 0.2 * (i as f32 * 0.06).sin()).collect();
        g.process(&mut word);
        assert!(word[200].abs() > 0.0 && (word[479] - 0.2 * (479.0f32 * 0.06).sin()).abs() < 1e-6);
    }

    #[test]
    fn the_pin_matches_fetch_model_sh() {
        let script = include_str!("../../../scripts/fetch-model.sh");
        assert!(script.contains(&format!("SHA256={MODEL_SHA256}")));
    }

    #[tokio::test]
    async fn a_model_with_the_wrong_checksum_is_refused_and_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("m").join(MODEL_FILE);
        let r = load(Some(path.clone()), async { Ok(b"not the model".to_vec()) }).await;
        assert_eq!(
            r.err().as_deref(),
            Some("the model's checksum does not match")
        );
        assert!(!path.exists());
        // A bad cached file is not used: the download is tried, and its failure is the reason.
        store(&path, b"tampered").unwrap();
        let r = load(Some(path.clone()), async { Err("no answer".to_string()) }).await;
        assert_eq!(r.err().as_deref(), Some("no answer"));
        assert!(verify(b"").is_err());
    }

    /// The model `scripts/fetch-model.sh` (or `scripts/build-dsp.sh`) puts in `crates/vesper-dsp/model/`. Not
    /// committed, so the tests that need it print a note and pass without it.
    fn local_model() -> Option<Vec<u8>> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../crates/vesper-dsp/model/DeepFilterNet3_onnx.tar.gz"
        );
        let m = std::fs::read(path).ok();
        if m.is_none() {
            println!("SKIPPED: no model at {path}; run scripts/fetch-model.sh");
        }
        m
    }

    /// A speech-like voice (120 Hz harmonics with vibrato, in 300 ms syllables with 200 ms pauses) plus white noise,
    /// through the call's capture filter with the real model. A pure sine does not work: DeepFilterNet takes a steady
    /// tone for noise and outputs silence. Measured: the noise left in the pauses, and the level kept in the syllables.
    #[tokio::test]
    async fn deepfilter_takes_out_noise_with_the_local_model() {
        let Some(bytes) = local_model() else { return };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(MODEL_FILE);
        let model = bytes.clone();
        let loaded = load(Some(path.clone()), async move { Ok(model) }).await;
        assert!(loaded.is_ok(), "{:?}", loaded.err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes, "cached");
        let cached = load(Some(path), async { Err("not asked".to_string()) }).await;
        assert!(cached.is_ok(), "from the cache: {:?}", cached.err());

        let t = Instant::now();
        assert!(deep_filter(&bytes).is_ok());
        println!("building the denoiser took {:?} (this build)", t.elapsed());
        // A budget no debug build misses: this test is about the sound, the fallback has its own.
        let late = Late::new(Duration::from_secs(10), 100, 20, 0);
        let mut c = Capture::new(Denoise::DeepFilter, rx(Some(loaded)), deep_filter, late);
        assert_eq!(settle(&mut c, &mut [0.0; FRAME]), DEEPFILTER);
        let rate = 48_000.0;
        let voice = |i: usize| -> f32 {
            let t = i as f32 / rate;
            let in_syllable = t % 0.5;
            if in_syllable >= 0.3 {
                return 0.0;
            }
            let env = (std::f32::consts::PI * in_syllable / 0.3).sin();
            let f0 = 120.0 * (1.0 + 0.03 * (std::f32::consts::TAU * 5.0 * t).sin());
            let phase = std::f32::consts::TAU * f0 * t;
            let x: f32 = (1..30).map(|k| (k as f32 * phase).sin() / k as f32).sum();
            0.08 * env * x
        };
        let mut n = Noise(0x2545F4914F6CDD1D);
        let seconds = 4;
        let (mut input, mut noise, mut output) = (vec![], vec![], vec![]);
        for f in 0..seconds * 50 {
            let clean: Vec<f32> = (0..FRAME).map(|i| voice(f * FRAME + i)).collect();
            let hiss: Vec<f32> = (0..FRAME).map(|_| n.next(0.02)).collect();
            let mut x: Vec<f32> = clean.iter().zip(&hiss).map(|(a, b)| a + b).collect();
            input.extend(&x);
            noise.extend(hiss);
            c.process(&mut x, true);
            output.extend(x);
        }
        // Skip the first second while the model settles. The output is 30 ms late (1440 samples); take the middle of
        // each pause and syllable so the delay and the fades do not matter.
        let lag = 1440;
        let mid = |from: f32, to: f32| {
            (1..seconds * 2).flat_map(move |s| {
                let base = s as f32 * 0.5;
                let (a, b) = (
                    ((base + from) * rate) as usize,
                    ((base + to) * rate) as usize,
                );
                a..b
            })
        };
        let sum = |x: &[f32], r: &mut dyn Iterator<Item = usize>, lag: usize| -> f64 {
            r.map(|i| f64::from(x[i + lag]).powi(2)).sum()
        };
        let pause_in = sum(&noise, &mut mid(0.35, 0.45), 0);
        let pause_out = sum(&output, &mut mid(0.35, 0.45), lag);
        let word_in = sum(&input, &mut mid(0.1, 0.2), 0);
        let word_out = sum(&output, &mut mid(0.1, 0.2), lag);
        let db = |a: f64, b: f64| 10.0 * (a / b).log10();
        let ms = c.total.as_secs_f64() * 1000.0 / f64::from(c.frames);
        println!(
            "DeepFilterNet on the local model: noise in the pauses down {:.1} dB, syllables at {:.1} dB, \
             {ms:.2} ms per 10 ms frame (this build)",
            db(pause_in, pause_out),
            db(word_out, word_in),
        );
        assert!(db(pause_in, pause_out) > 15.0, "noise down by over 15 dB");
        // The synthetic voice is not real speech: the model takes some of it for noise (about 10 dB on the dev
        // machine). It must still come through, far above what is left of the noise.
        assert!(db(word_out, word_in) > -15.0, "the voice comes through");
    }
}
