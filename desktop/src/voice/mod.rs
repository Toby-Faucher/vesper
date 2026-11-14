//! Voice: the call engine and the audio around it. One call runs on its own thread (`engine.rs`): a str0m client over
//! UDP to the server's SFU, Opus both ways, a jitter buffer and decoder per person, and the mix to the speakers.
//!
//! Signaling is not done here. The engine makes an offer and answers the SFU's offers; the model sends them over the
//! socket, as the web app's `join.ts` does. Audio comes in through `AudioInput` and goes out through `AudioOutput`, so
//! tests run a call with no devices; the real ones are cpal (`device.rs`).

mod device;
mod engine;
pub mod jitter;
pub mod mix;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::Sender;

pub use device::{Cpal, Resampler};

/// Opus at 48 kHz, mono, 20 ms frames.
pub const RATE: u32 = 48_000;
pub const FRAME: usize = 960;

/// Captured audio, 48 kHz mono, -1 to 1.
pub trait AudioInput {
    /// Moves what was captured since the last call to the end of `out`.
    fn read(&mut self, out: &mut Vec<f32>);
}

/// Audio to play, 48 kHz mono, -1 to 1.
pub trait AudioOutput {
    /// Queues `samples` to play after what is queued.
    fn write(&mut self, samples: &[f32]);
}

/// Opens the devices for a call. Called on the call's thread, so a device that must stay on one thread can.
pub trait AudioDevices: Send + Sync {
    /// The error is a sentence for the user.
    fn input(&self) -> Result<Box<dyn AudioInput>, String>;
    fn output(&self) -> Result<Box<dyn AudioOutput>, String>;
}

/// From the model to the call engine (through the network thread).
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceCmd {
    /// Open the devices and make an offer. `call` tags this call's events; `volumes` are percent by user id.
    Start {
        call: u64,
        bitrate_kbps: u32,
        volumes: HashMap<String, f32>,
    },
    /// The SFU's answer to the offer.
    Answer(String),
    /// A renegotiation offer from the SFU; the engine answers with `VoiceEvent::Answer`.
    Offer(String),
    Muted(bool),
    Deafened(bool),
    /// 0 to 400 percent.
    Volume {
        user_id: String,
        percent: f32,
    },
    Stop,
}

/// From the call engine to the model.
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceEvent {
    /// The offer to send with `join`.
    Offer(String),
    /// The answer to a renegotiation offer.
    Answer(String),
    /// ICE and DTLS are up: media flows.
    Connected,
    /// Who speaks now: me, and the user ids of the others. Sent when it changes.
    Speaking { me: bool, peers: Vec<String> },
    /// The call cannot go on. The text is for the user.
    Failed(String),
}

/// A running call. Dropping it ends the call's thread.
pub struct Call {
    tx: Sender<VoiceCmd>,
}

impl Call {
    pub fn start(
        devices: Arc<dyn AudioDevices>,
        bitrate_kbps: u32,
        volumes: HashMap<String, f32>,
        emit: impl Fn(VoiceEvent) + Send + 'static,
    ) -> Call {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("vesper-voice".into())
            .spawn(move || engine::run(&*devices, bitrate_kbps, volumes, rx, &emit))
            .expect("the call thread starts");
        Call { tx }
    }

    pub fn send(&self, cmd: VoiceCmd) {
        let _ = self.tx.send(cmd);
    }
}
