//! The microphone and speakers through cpal: the ones chosen in the settings by name, else the system defaults.
//! Capture becomes 48 kHz mono; playback goes from 48 kHz mono to the device's own rate and channels.
//!
//! A stream error that means the device is gone, or a microphone that stops giving samples, makes `alive` false; the
//! engine then says so and opens the device again.
//!
//! shortcut: linear resampling; upgrade when people hear it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};

use super::{AudioDevices, AudioInput, AudioOutput, RATE};

type Queue = Arc<Mutex<VecDeque<f32>>>;

/// A microphone that gives nothing for this long is gone, even if its backend said nothing.
const SILENT_MIC: Duration = Duration::from_secs(2);

/// Set by the stream's error callback when the error means the device is gone. A glitch (an underrun), a route that
/// followed the default device, or real-time scheduling refused, do not count.
fn on_error(failed: &Arc<AtomicBool>) -> impl FnMut(cpal::Error) + Send + 'static {
    let failed = failed.clone();
    move |e| {
        use cpal::ErrorKind::{DeviceChanged, RealtimeDenied, Xrun};
        if !matches!(e.kind(), Xrun | DeviceChanged | RealtimeDenied) {
            failed.store(true, Ordering::Relaxed);
        }
    }
}

/// The input and output device names, for the settings. Empty when there are none or the system cannot list them.
pub fn device_names() -> (Vec<String>, Vec<String>) {
    let host = cpal::default_host();
    let input = host
        .input_devices()
        .map(|d| d.map(|d| d.to_string()).collect());
    let output = host
        .output_devices()
        .map(|d| d.map(|d| d.to_string()).collect());
    (input.unwrap_or_default(), output.unwrap_or_default())
}

/// The device called `name`, else the default.
fn pick(
    list: Result<impl Iterator<Item = cpal::Device>, cpal::Error>,
    name: Option<&str>,
    default: Option<cpal::Device>,
) -> Option<cpal::Device> {
    name.and_then(|n| list.ok()?.find(|d| d.to_string() == n))
        .or(default)
}

/// cpal's devices.
pub struct Cpal;

impl AudioDevices for Cpal {
    fn input(&self, name: Option<&str>) -> Result<Box<dyn AudioInput>, String> {
        let fail = |e: &dyn std::fmt::Display| format!("Could not open the microphone: {e}");
        let host = cpal::default_host();
        let dev = pick(host.input_devices(), name, host.default_input_device())
            .ok_or("No microphone was found.")?;
        let supported = dev.default_input_config().map_err(|e| fail(&e))?;
        let cfg = supported.config();
        let queue = Queue::default();
        let failed = Arc::new(AtomicBool::new(false));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => capture::<f32>(&dev, cfg, queue.clone(), &failed),
            SampleFormat::I16 => capture::<i16>(&dev, cfg, queue.clone(), &failed),
            SampleFormat::U16 => capture::<u16>(&dev, cfg, queue.clone(), &failed),
            SampleFormat::I32 => capture::<i32>(&dev, cfg, queue.clone(), &failed),
            f => return Err(fail(&format!("sample format {f} is not supported"))),
        }
        .map_err(|e| fail(&e))?;
        stream.play().map_err(|e| fail(&e))?;
        Ok(Box::new(Input {
            _stream: stream,
            queue,
            failed,
            heard: Instant::now(),
        }))
    }

    fn output(&self, name: Option<&str>) -> Result<Box<dyn AudioOutput>, String> {
        let fail = |e: &dyn std::fmt::Display| format!("Could not open the speakers: {e}");
        let host = cpal::default_host();
        let dev = pick(host.output_devices(), name, host.default_output_device())
            .ok_or("No speakers or headphones were found.")?;
        let supported = dev.default_output_config().map_err(|e| fail(&e))?;
        let cfg = supported.config();
        let queue = Queue::default();
        let failed = Arc::new(AtomicBool::new(false));
        let stream = match supported.sample_format() {
            SampleFormat::F32 => play::<f32>(&dev, cfg, queue.clone(), &failed),
            SampleFormat::I16 => play::<i16>(&dev, cfg, queue.clone(), &failed),
            SampleFormat::U16 => play::<u16>(&dev, cfg, queue.clone(), &failed),
            SampleFormat::I32 => play::<i32>(&dev, cfg, queue.clone(), &failed),
            f => return Err(fail(&format!("sample format {f} is not supported"))),
        }
        .map_err(|e| fail(&e))?;
        stream.play().map_err(|e| fail(&e))?;
        Ok(Box::new(Output {
            _stream: stream,
            queue,
            failed,
            resampler: Resampler::new(RATE, cfg.sample_rate),
            channels: usize::from(cfg.channels),
            // 200 ms of device samples: more means the engine runs ahead of the device, so the oldest go.
            cap: cfg.sample_rate as usize * usize::from(cfg.channels) / 5,
        }))
    }
}

fn capture<T>(
    dev: &cpal::Device,
    cfg: StreamConfig,
    queue: Queue,
    failed: &Arc<AtomicBool>,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(cfg.channels);
    let mut resampler = Resampler::new(cfg.sample_rate, RATE);
    let mut mono = vec![];
    dev.build_input_stream::<T, _, _>(
        cfg,
        move |data: &[T], _| {
            mono.clear();
            mono.extend(
                data.chunks(channels)
                    .map(|f| f.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32),
            );
            let mut q = queue.lock().expect("audio queue");
            resampler.run(&mono, |s| q.push_back(s));
            // One second at most, if the call thread stops reading.
            let extra = q.len().saturating_sub(RATE as usize);
            q.drain(..extra);
        },
        on_error(failed),
        None,
    )
}

fn play<T>(
    dev: &cpal::Device,
    cfg: StreamConfig,
    queue: Queue,
    failed: &Arc<AtomicBool>,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    dev.build_output_stream::<T, _, _>(
        cfg,
        move |data: &mut [T], _| {
            let mut q = queue.lock().expect("audio queue");
            for s in data {
                *s = T::from_sample(q.pop_front().unwrap_or(0.0));
            }
        },
        on_error(failed),
        None,
    )
}

struct Input {
    _stream: Stream,
    queue: Queue,
    failed: Arc<AtomicBool>,
    /// When samples last came.
    heard: Instant,
}

impl AudioInput for Input {
    fn read(&mut self, out: &mut Vec<f32>) {
        let mut q = self.queue.lock().expect("audio queue");
        if !q.is_empty() {
            self.heard = Instant::now();
        }
        out.extend(q.drain(..));
    }

    fn alive(&self) -> bool {
        !self.failed.load(Ordering::Relaxed) && self.heard.elapsed() < SILENT_MIC
    }
}

struct Output {
    _stream: Stream,
    queue: Queue,
    failed: Arc<AtomicBool>,
    resampler: Resampler,
    channels: usize,
    cap: usize,
}

impl AudioOutput for Output {
    fn write(&mut self, samples: &[f32]) {
        let mut q = self.queue.lock().expect("audio queue");
        let ch = self.channels;
        self.resampler
            .run(samples, |s| q.extend(std::iter::repeat_n(s, ch)));
        let extra = q.len().saturating_sub(self.cap);
        q.drain(..extra);
    }

    fn alive(&self) -> bool {
        !self.failed.load(Ordering::Relaxed)
    }
}

/// Linear interpolation from one sample rate to another, carried across calls.
#[derive(Debug)]
pub struct Resampler {
    /// Input samples per output sample.
    step: f64,
    /// The read position, in input samples, where 0 is `prev`, the last sample of the previous call.
    pos: f64,
    prev: f32,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Resampler {
        Resampler {
            step: f64::from(from) / f64::from(to),
            pos: 0.0,
            prev: 0.0,
        }
    }

    pub fn run(&mut self, input: &[f32], mut out: impl FnMut(f32)) {
        let Some(&last) = input.last() else { return };
        let len = input.len() as f64;
        while self.pos < len {
            let i = self.pos as usize;
            let a = if i == 0 { self.prev } else { input[i - 1] };
            let b = input[i];
            out(a + (b - a) * (self.pos - i as f64) as f32);
            self.pos += self.step;
        }
        self.pos -= len;
        self.prev = last;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resample(from: u32, to: u32, input: &[f32], chunk: usize) -> Vec<f32> {
        let mut r = Resampler::new(from, to);
        let mut out = vec![];
        for c in input.chunks(chunk) {
            r.run(c, |s| out.push(s));
        }
        out
    }

    #[test]
    fn resampler_keeps_the_rate_and_the_shape() {
        let input = vec![0.5; 44_100];
        let out = resample(44_100, 48_000, &input, 441);
        assert!((out.len() as i64 - 48_000).abs() <= 1, "{}", out.len());
        assert!(out[10..].iter().all(|s| (s - 0.5).abs() < 1e-6));

        let same: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = resample(48_000, 48_000, &same, 7);
        assert_eq!(out.len(), 100);
        assert_eq!(out[1..], same[..99], "one sample late, otherwise the same");

        let ramp: Vec<f32> = (0..96).map(|i| i as f32).collect();
        let out = resample(96_000, 48_000, &ramp, 10);
        assert_eq!(out.len(), 48);
        assert_eq!(out[5], 9.0);
    }
}
