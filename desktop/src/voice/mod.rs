//! Voice: the call engine and the audio around it. One call runs on its own thread (`engine.rs`): a str0m client over
//! UDP to the server's SFU, Opus both ways, a jitter buffer and decoder per person, and the mix to the speakers.
//! Watched screen shares decode on a second thread (`video.rs`).
//!
//! Signaling is not done here. The engine makes an offer and answers the SFU's offers; the model sends them over the
//! socket, as the web app's `join.ts` does. Audio comes in through `AudioInput` and goes out through `AudioOutput`, so
//! tests run a call with no devices; the real ones are cpal (`device.rs`).

pub mod denoise;
mod device;
pub mod echo;
mod engine;
pub mod jitter;
pub mod mix;
pub mod ptt;
pub mod video;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

pub use device::{Cpal, Resampler, device_names};
pub use video::Frame;

use crate::settings::Denoise;

/// Opus at 48 kHz, mono, 20 ms frames.
pub const RATE: u32 = 48_000;
pub const FRAME: usize = 960;

/// Captured audio, 48 kHz mono, -1 to 1.
pub trait AudioInput {
    /// Moves what was captured since the last call to the end of `out`.
    fn read(&mut self, out: &mut Vec<f32>);
    /// False once the device is gone (unplugged, or its stream failed). The engine then opens it again.
    fn alive(&self) -> bool {
        true
    }
}

/// Audio to play, 48 kHz mono, -1 to 1.
pub trait AudioOutput {
    /// Queues `samples` to play after what is queued.
    fn write(&mut self, samples: &[f32]);
    /// False once the device is gone.
    fn alive(&self) -> bool {
        true
    }
}

/// Opens the devices for a call. Called on the call's thread, so a device that must stay on one thread can.
pub trait AudioDevices: Send + Sync {
    /// `name` is the device chosen in the settings; `None`, or a name that is not there, is the system default.
    /// The error is a sentence for the user.
    fn input(&self, name: Option<&str>) -> Result<Box<dyn AudioInput>, String>;
    fn output(&self, name: Option<&str>) -> Result<Box<dyn AudioOutput>, String>;
}

/// How a call starts, from the settings.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Setup {
    pub bitrate_kbps: u32,
    /// Percent by user id.
    pub volumes: HashMap<String, f32>,
    pub denoise: Denoise,
    /// Echo cancellation on the microphone.
    pub echo: bool,
    pub input: Option<String>,
    pub output: Option<String>,
}

/// From the model to the call engine (through the network thread).
#[derive(Debug, Clone, PartialEq)]
pub enum VoiceCmd {
    /// Open the devices and make an offer. `call` tags this call's events.
    Start {
        call: u64,
        setup: Setup,
    },
    /// The SFU's answer to the offer.
    Answer(String),
    /// A renegotiation offer from the SFU; the engine answers with `VoiceEvent::Answer`.
    Offer(String),
    /// Not sending: muted, or push to talk not held.
    Muted(bool),
    Deafened(bool),
    /// 0 to 400 percent.
    Volume {
        user_id: String,
        percent: f32,
    },
    /// Decode `user_id`'s screen share (sent with the server's `watch`), or stop.
    Watch {
        user_id: String,
        on: bool,
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
    /// What the noise filter is doing, for the call bar. Sent when it changes.
    Filter(String),
    /// Echo cancellation turned off during the call: why, for the call bar.
    EchoOff(String),
    /// A device was lost ("Your microphone was disconnected."), or empty once both are back.
    Devices(String),
    /// The newest picture of a watched screen share.
    Frame(Frame),
    /// A watched share loses packets or frames, or stopped doing so. Sent when it changes.
    Struggling { user_id: String, on: bool },
    /// The call cannot go on. The text is for the user.
    Failed(String),
}

/// A running call. Dropping it ends the call's thread.
pub struct Call {
    tx: Sender<VoiceCmd>,
}

impl Call {
    /// `denoiser` brings DeepFilterNet when it has loaded (see `denoise::load`); the call starts without it.
    pub fn start(
        devices: Arc<dyn AudioDevices>,
        setup: Setup,
        denoiser: Receiver<denoise::Loaded>,
        emit: impl Fn(VoiceEvent) + Send + Sync + 'static,
    ) -> Call {
        let (tx, rx) = std::sync::mpsc::channel();
        let emit = Arc::new(emit);
        let pictures = emit.clone();
        // Screen shares decode on their own thread, so a big frame never delays the 20 ms audio turn.
        let video = video::Video::start(move |f| pictures(VoiceEvent::Frame(f)));
        std::thread::Builder::new()
            .name("vesper-voice".into())
            .spawn(move || engine::run(&*devices, setup, denoiser, video, rx, &*emit))
            .expect("the call thread starts");
        Call { tx }
    }

    pub fn send(&self, cmd: VoiceCmd) {
        let _ = self.tx.send(cmd);
    }
}
