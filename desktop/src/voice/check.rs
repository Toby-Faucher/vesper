//! The mic test outside a call (`MicCheck.svelte`'s Mic test): the chosen microphone through the call's noise filter,
//! played on the chosen speakers, with its level every 100 ms. In a call the engine does the same with the call's own
//! capture (`VoiceCmd::Hear`), so the microphone is not opened twice.
//!
//! The echo canceller is left out: its far end would be your own voice, which it would take out.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use super::denoise::{Capture, Late, Loaded, deep_filter};
use super::mix::Meter;
use super::{AudioDevices, FRAME, Setup, VoiceEvent};

pub const MIC_LOST: &str = "Your microphone was disconnected.";

/// A running mic test. Dropping it stops the test and closes the devices.
pub struct Check {
    _stop: Sender<()>,
}

impl Check {
    /// Emits `Level`, or `Failed` with a sentence for the user when a device does not open or is lost.
    pub fn start(
        devices: Arc<dyn AudioDevices>,
        setup: Setup,
        denoiser: Receiver<Loaded>,
        emit: impl Fn(VoiceEvent) + Send + 'static,
    ) -> Check {
        let (stop, stopped) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("vesper-mic-test".into())
            .spawn(move || {
                if let Err(why) = run(&*devices, setup, denoiser, stopped, &emit) {
                    emit(VoiceEvent::Failed(why));
                }
            });
        Check { _stop: stop }
    }
}

fn run(
    devices: &dyn AudioDevices,
    setup: Setup,
    denoiser: Receiver<Loaded>,
    stopped: Receiver<()>,
    emit: &dyn Fn(VoiceEvent),
) -> Result<(), String> {
    let mut input = devices.input(setup.input.as_deref())?;
    let mut output = devices.output(setup.output.as_deref())?;
    let mut capture = Capture::new(setup.denoise, denoiser, deep_filter, Late::default());
    if let Some(g) = setup.gate_db {
        capture.set_gate(g);
    }
    let (mut captured, mut meter) = (vec![], Meter::default());
    while let Err(TryRecvError::Empty) = stopped.try_recv() {
        std::thread::sleep(Duration::from_millis(20));
        if !input.alive() {
            return Err(MIC_LOST.into());
        }
        input.read(&mut captured);
        // At most 100 ms behind: a stalled thread drops the oldest, as the engine does.
        let extra = captured.len().saturating_sub(5 * FRAME);
        captured.drain(..extra);
        while captured.len() >= FRAME {
            let mut frame: Vec<f32> = captured.drain(..FRAME).collect();
            capture.process(&mut frame, true);
            if let Some(db) = meter.update(&frame) {
                emit(VoiceEvent::Level(db));
            }
            output.write(&frame);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::settings::Denoise;
    use crate::voice::{AudioInput, AudioOutput};

    struct Tone;
    impl AudioInput for Tone {
        fn read(&mut self, out: &mut Vec<f32>) {
            out.extend((0..FRAME).map(|i| 0.5 * (i as f32 * 0.05).sin()));
        }
    }
    struct Out(Arc<Mutex<Vec<f32>>>);
    impl AudioOutput for Out {
        fn write(&mut self, samples: &[f32]) {
            self.0.lock().unwrap().extend_from_slice(samples);
        }
    }
    struct Fake(Arc<Mutex<Vec<f32>>>, bool);
    impl AudioDevices for Fake {
        fn input(&self, _: Option<&str>) -> Result<Box<dyn AudioInput>, String> {
            if self.1 {
                return Err("No microphone was found.".into());
            }
            Ok(Box::new(Tone))
        }
        fn output(&self, _: Option<&str>) -> Result<Box<dyn AudioOutput>, String> {
            Ok(Box::new(Out(self.0.clone())))
        }
    }

    #[test]
    fn the_mic_is_played_and_metered_until_dropped() {
        let played = Arc::new(Mutex::new(vec![]));
        let (tx, rx) = std::sync::mpsc::channel();
        let setup = Setup {
            denoise: Denoise::Off,
            ..Setup::default()
        };
        let check = Check::start(
            Arc::new(Fake(played.clone(), false)),
            setup,
            std::sync::mpsc::channel().1,
            move |ev| drop(tx.send(ev)),
        );
        let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            matches!(ev, VoiceEvent::Level(db) if db > -7.0 && db < -5.0),
            "{ev:?}"
        );
        assert!(played.lock().unwrap().len() >= 5 * FRAME);
        drop(check);
        while rx.recv_timeout(Duration::from_secs(5)).is_ok() {}
        let n = played.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(played.lock().unwrap().len(), n, "stopped");
    }

    #[test]
    fn a_mic_that_does_not_open_says_why() {
        let (tx, rx) = std::sync::mpsc::channel();
        let _check = Check::start(
            Arc::new(Fake(Arc::default(), true)),
            Setup::default(),
            std::sync::mpsc::channel().1,
            move |ev| drop(tx.send(ev)),
        );
        let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(ev, VoiceEvent::Failed("No microphone was found.".into()));
    }
}
