//! Debug builds only (the module is `cfg(debug_assertions)`, so a release build has none of it): puts the app in a
//! state for the parity harness (`web/e2e/parity.mjs`) without typing.
//!
//! - `VESPER_DEV_SERVER`, `VESPER_DEV_USER`, `VESPER_DEV_PASSWORD`: sign in with these once the sign in screen shows.
//! - `VESPER_DEV_SCENE`: `sign-in` (stay there), or after signing in `room:<name>` (and wait for its messages),
//!   `settings`, or anything else for the main screen as it opens.
//!
//! Prints `VESPER_DEV_READY` on stderr once the state is reached; the harness takes its picture after that.

use gpui::{Context, Window};

use super::Vesper;
use crate::model::Screen;

#[derive(Debug, Default, PartialEq)]
enum Step {
    #[default]
    SignIn,
    Scene,
    /// The room is open; ready once its messages are in.
    Room,
    Done,
}

#[derive(Debug, Default)]
pub struct Dev {
    login: Option<(String, String, String)>,
    scene: Option<String>,
    step: Step,
}

impl Dev {
    pub fn from_env() -> Dev {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let login = match (
            var("VESPER_DEV_SERVER"),
            var("VESPER_DEV_USER"),
            var("VESPER_DEV_PASSWORD"),
        ) {
            (Some(s), Some(u), Some(p)) => Some((s, u, p)),
            _ => None,
        };
        let any = login.is_some() || var("VESPER_DEV_SCENE").is_some();
        Dev {
            login,
            scene: var("VESPER_DEV_SCENE"),
            step: if any { Step::SignIn } else { Step::Done },
        }
    }
}

fn ready(what: &str) {
    eprintln!("VESPER_DEV_READY {what}");
}

impl Vesper {
    /// One step towards the asked-for state, after each event from the network.
    pub(super) fn dev_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.dev.step {
            Step::SignIn if self.model.screen == Screen::SignIn => {
                let Some((server, user, pass)) = self.dev.login.take() else {
                    self.dev.step = Step::Done;
                    return ready("sign-in");
                };
                self.server
                    .update(cx, |i, cx| i.set_value(server, window, cx));
                self.username
                    .update(cx, |i, cx| i.set_value(user, window, cx));
                self.password
                    .update(cx, |i, cx| i.set_value(pass, window, cx));
                self.submit_sign_in(window, cx);
                self.dev.step = Step::Scene;
            }
            Step::Scene
                if self.model.screen == Screen::Main && !self.model.state.rooms.is_empty() =>
            {
                match self.dev.scene.as_deref().unwrap_or("") {
                    "settings" => {
                        self.open_settings(window, cx);
                        self.dev.step = Step::Done;
                        ready("settings");
                    }
                    s if s.starts_with("room:") => {
                        let name = &s["room:".len()..];
                        let Some(id) = self
                            .model
                            .state
                            .rooms
                            .iter()
                            .find(|r| r.name == name)
                            .map(|r| r.id)
                        else {
                            return;
                        };
                        self.open_room(id, window, cx);
                        self.dev.step = Step::Room;
                    }
                    _ => {
                        self.dev.step = Step::Done;
                        ready("main");
                    }
                }
            }
            Step::Room if self.model.log().is_some_and(|l| !l.messages.is_empty()) => {
                // Nothing focused, as the web scenes' rest(): no caret or focus border in the picture.
                self.key_focus.focus(window);
                self.dev.step = Step::Done;
                ready("room");
            }
            _ => {}
        }
    }
}
