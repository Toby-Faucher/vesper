//! The default microphone and speakers through cpal. Capture becomes 48 kHz mono; playback goes from 48 kHz mono to the
//! device's own rate and channels.
//!
//! shortcut: the default devices only, linear resampling, and a device lost mid-call goes silent without a message;
//! upgrade with device choice in settings.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};

use super::{AudioDevices, AudioInput, AudioOutput, RATE};

type Queue = Arc<Mutex<VecDeque<f32>>>;

/// The system's default input and output.
pub struct Cpal;

impl AudioDevices for Cpal {
    fn input(&self) -> Result<Box<dyn AudioInput>, String> {
        let fail = |e: &dyn std::fmt::Display| format!("Could not open the microphone: {e}");
        let dev = cpal::default_host()
            .default_input_device()
            .ok_or("No microphone was found.")?;
        let supported = dev.default_input_config().map_err(|e| fail(&e))?;
        let cfg = supported.config();
        let queue = Queue::default();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => capture::<f32>(&dev, cfg, queue.clone()),
            SampleFormat::I16 => capture::<i16>(&dev, cfg, queue.clone()),
            SampleFormat::U16 => capture::<u16>(&dev, cfg, queue.clone()),
            SampleFormat::I32 => capture::<i32>(&dev, cfg, queue.clone()),
            f => return Err(fail(&format!("sample format {f} is not supported"))),
        }
        .map_err(|e| fail(&e))?;
        stream.play().map_err(|e| fail(&e))?;
        Ok(Box::new(Input {
            _stream: stream,
            queue,
        }))
    }

    fn output(&self) -> Result<Box<dyn AudioOutput>, String> {
        let fail = |e: &dyn std::fmt::Display| format!("Could not open the speakers: {e}");
        let dev = cpal::default_host()
            .default_output_device()
            .ok_or("No speakers or headphones were found.")?;
        let supported = dev.default_output_config().map_err(|e| fail(&e))?;
        let cfg = supported.config();
        let queue = Queue::default();
        let stream = match supported.sample_format() {
            SampleFormat::F32 => play::<f32>(&dev, cfg, queue.clone()),
            SampleFormat::I16 => play::<i16>(&dev, cfg, queue.clone()),
            SampleFormat::U16 => play::<u16>(&dev, cfg, queue.clone()),
            SampleFormat::I32 => play::<i32>(&dev, cfg, queue.clone()),
            f => return Err(fail(&format!("sample format {f} is not supported"))),
        }
        .map_err(|e| fail(&e))?;
        stream.play().map_err(|e| fail(&e))?;
        Ok(Box::new(Output {
            _stream: stream,
            queue,
            resampler: Resampler::new(RATE, cfg.sample_rate),
            channels: usize::from(cfg.channels),
            // 200 ms of device samples: more means the engine runs ahead of the device, so the oldest go.
            cap: cfg.sample_rate as usize * usize::from(cfg.channels) / 5,
        }))
    }
}

fn capture<T>(dev: &cpal::Device, cfg: StreamConfig, queue: Queue) -> Result<Stream, cpal::Error>
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
        |_| {},
        None,
    )
}

fn play<T>(dev: &cpal::Device, cfg: StreamConfig, queue: Queue) -> Result<Stream, cpal::Error>
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
        |_| {},
        None,
    )
}

struct Input {
    _stream: Stream,
    queue: Queue,
}

impl AudioInput for Input {
    fn read(&mut self, out: &mut Vec<f32>) {
        out.extend(self.queue.lock().expect("audio queue").drain(..));
    }
}

struct Output {
    _stream: Stream,
    queue: Queue,
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
