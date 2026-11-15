//! Connection quality, after the web's `quality.ts`, `link.ts` and `link-stats.ts`: the call bar's grade from the round
//! trip and loss, and each person's grade from their audio's loss and jitter, held so it does not flicker.

/// One stats window. `None` is not measured in it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sample {
    pub loss_pct: Option<f32>,
    pub jitter_ms: Option<f32>,
    pub rtt_ms: Option<f32>,
    /// Packets that arrived in the window (`None` for your own row).
    pub packets: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    Good,
    Fair,
    Poor,
}

impl Grade {
    pub fn word(self) -> &'static str {
        match self {
            Grade::Good => "good",
            Grade::Fair => "fair",
            Grade::Poor => "poor",
        }
    }
    /// Bars lit in the icon.
    pub fn bars(self) -> u8 {
        match self {
            Grade::Good => 3,
            Grade::Fair => 2,
            Grade::Poor => 1,
        }
    }
}

/// `FAIR` and `POOR` of quality.ts (guesses): round trip in ms and loss in percent.
const FAIR: (f32, f32) = (150., 2.);
const POOR: (f32, f32) = (300., 5.);
/// `LINK_FAIR` and `LINK_POOR`: loss, jitter, round trip.
const LINK_FAIR: (f32, f32, f32) = (2., 30., 150.);
const LINK_POOR: (f32, f32, f32) = (5., 60., 300.);
/// Fewer packets than this in a window make the loss figure too coarse.
pub const MIN_PACKETS: u64 = 25;
const SAMPLES_TO_SHOW: u32 = 2;
const SAMPLES_TO_CHANGE: u32 = 2;
/// Windows in a row with no packet before a person who is not muted goes back to measuring.
const DEAD_WINDOWS: u32 = 3;

/// The call bar's grade (`quality()`): `None` is unknown (nothing measured yet).
pub fn quality(s: Option<&Sample>) -> Option<Grade> {
    let s = s.filter(|s| s.rtt_ms.is_some() || s.loss_pct.is_some())?;
    let (rtt, loss) = (s.rtt_ms.unwrap_or(0.), s.loss_pct.unwrap_or(0.));
    Some(if rtt >= POOR.0 || loss >= POOR.1 {
        Grade::Poor
    } else if rtt >= FAIR.0 || loss >= FAIR.1 {
        Grade::Fair
    } else {
        Grade::Good
    })
}

/// The call bar's line: "Good connection", "Checking connection", or "Weak connection: 420 ms delay" (`qualityText`).
pub fn quality_text(s: Option<&Sample>) -> String {
    match (quality(s), s) {
        (None, _) | (_, None) => "Checking connection".into(),
        (Some(Grade::Good), _) => "Good connection".into(),
        (Some(_), Some(s)) => {
            let loss = s.loss_pct.unwrap_or(0.);
            if loss >= FAIR.1 {
                format!("Weak connection: {} % loss", loss.round())
            } else {
                format!(
                    "Weak connection: {} ms delay",
                    s.rtt_ms.unwrap_or(0.).round()
                )
            }
        }
    }
}

/// A person's shown grade (`Link`): `None` while measuring.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Link {
    pub grade: Option<Grade>,
    seen: u32,
    next: Option<Grade>,
    streak: u32,
    pub idle: u32,
    pub last: Option<Sample>,
}

fn over(s: &Sample, t: (f32, f32, f32)) -> bool {
    s.loss_pct.unwrap_or(0.) >= t.0
        || s.jitter_ms.unwrap_or(0.) >= t.1
        || s.rtt_ms.unwrap_or(0.) >= t.2
}

fn grade_sample(s: &Sample) -> Grade {
    if over(s, LINK_POOR) {
        Grade::Poor
    } else if over(s, LINK_FAIR) {
        Grade::Fair
    } else {
        Grade::Good
    }
}

impl Link {
    /// One window (`stepLink`): no data changes nothing, a grade shows after two windows and changes after two
    /// that agree, and three windows without a packet from someone not `muted` go back to measuring.
    pub fn step(self, s: Option<Sample>, muted: bool) -> Link {
        let Some(s) = s else { return self };
        if s.packets == Some(0) {
            let idle = self.idle + 1;
            return if idle >= DEAD_WINDOWS && !muted {
                Link {
                    idle,
                    last: self.last,
                    ..Link::default()
                }
            } else {
                Link { idle, ..self }
            };
        }
        if s.loss_pct.is_none() && s.jitter_ms.is_none() && s.rtt_ms.is_none() {
            return self;
        }
        let g = grade_sample(&s);
        let seen = self.seen + 1;
        let fresh = Link {
            seen,
            last: Some(s),
            ..Link::default()
        };
        match self.grade {
            None => Link {
                grade: (seen >= SAMPLES_TO_SHOW).then_some(g),
                ..fresh
            },
            Some(cur) if cur == g => Link {
                grade: Some(cur),
                ..fresh
            },
            Some(cur) => {
                let streak = if self.next == Some(g) {
                    self.streak + 1
                } else {
                    1
                };
                if streak >= SAMPLES_TO_CHANGE {
                    Link {
                        grade: Some(g),
                        ..fresh
                    }
                } else {
                    Link {
                        grade: Some(cur),
                        next: Some(g),
                        streak,
                        ..fresh
                    }
                }
            }
        }
    }

    /// "Connection to alice: good" or "Your connection: measuring" (`linkLabel`).
    pub fn label(&self, name: Option<&str>) -> String {
        let who = name.map_or("Your connection".into(), |n| format!("Connection to {n}"));
        format!("{who}: {}", self.grade.map_or("measuring", Grade::word))
    }

    /// The figures in words (`linkDetail`): "1 % of audio lost, timing varies by 12 ms."
    pub fn detail(&self) -> String {
        if self.idle >= DEAD_WINDOWS {
            return "No audio has arrived for a few seconds.".into();
        }
        let Some(s) = self.last.filter(|_| self.grade.is_some()) else {
            return "Measuring, a few seconds after joining.".into();
        };
        let parts: Vec<String> = [
            s.loss_pct.map(|v| format!("{} % of audio lost", v.round())),
            s.jitter_ms
                .map(|v| format!("timing varies by {} ms", v.round())),
            s.rtt_ms.map(|v| format!("{} ms round trip", v.round())),
        ]
        .into_iter()
        .flatten()
        .collect();
        let text = parts.join(", ");
        let mut c = text.chars();
        match c.next() {
            Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
            None => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(loss: f32, jitter: f32) -> Option<Sample> {
        Some(Sample {
            loss_pct: Some(loss),
            jitter_ms: Some(jitter),
            rtt_ms: None,
            packets: Some(50),
        })
    }

    #[test]
    fn the_call_grade_and_its_words_follow_quality_ts() {
        let rtt = |ms| Sample {
            rtt_ms: Some(ms),
            ..Sample::default()
        };
        assert_eq!(quality(None), None);
        assert_eq!(quality(Some(&Sample::default())), None);
        assert_eq!(quality(Some(&rtt(40.))), Some(Grade::Good));
        assert_eq!(quality(Some(&rtt(150.))), Some(Grade::Fair));
        assert_eq!(quality(Some(&rtt(420.))), Some(Grade::Poor));
        assert_eq!(quality_text(None), "Checking connection");
        assert_eq!(quality_text(Some(&rtt(40.))), "Good connection");
        assert_eq!(
            quality_text(Some(&rtt(420.))),
            "Weak connection: 420 ms delay"
        );
        let lossy = Sample {
            loss_pct: Some(4.2),
            ..rtt(420.)
        };
        assert_eq!(quality_text(Some(&lossy)), "Weak connection: 4 % loss");
    }

    #[test]
    fn a_link_shows_after_two_windows_holds_and_goes_idle() {
        let l = Link::default().step(s(0., 5.), false);
        assert_eq!(l.grade, None, "one window is still measuring");
        let l = l.step(s(0., 5.), false);
        assert_eq!(l.grade, Some(Grade::Good));
        assert_eq!(l.step(None, false), l, "no data changes nothing");
        let l = l.step(s(6., 5.), false);
        assert_eq!(l.grade, Some(Grade::Good), "one poor window is held");
        let l = l.step(s(6., 5.), false);
        assert_eq!(l.grade, Some(Grade::Poor));
        assert_eq!(l.detail(), "6 % of audio lost, timing varies by 5 ms.");
        assert_eq!(l.label(Some("bob")), "Connection to bob: poor");
        let quiet = Some(Sample {
            packets: Some(0),
            ..Sample::default()
        });
        let muted = l.step(quiet, true).step(quiet, true).step(quiet, true);
        assert_eq!(muted.grade, Some(Grade::Poor), "a muted person keeps it");
        let gone = l.step(quiet, false).step(quiet, false).step(quiet, false);
        assert_eq!(gone.grade, None);
        assert_eq!(gone.detail(), "No audio has arrived for a few seconds.");
    }
}
