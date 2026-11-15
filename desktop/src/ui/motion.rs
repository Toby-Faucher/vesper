//! The web's motion (`web/src/lib/ui/motion.ts` and the CSS transitions) for GPUI:
//!
//! - [`cubic_bezier`] and [`ease`]: CSS easing, also for `Animation::new(d).with_easing(..)`.
//! - [`Tween`]: a number easing to a target (the sidebar slide, the selected room's glide).
//! - [`hover`]: a CSS `transition` on hover (fills and colours, `--dur-1`, ease-out), kept per element id.
//! - [`Roster`] and [`pose`]: things that come and go (people in voice, unread badges) and the preset pose of each
//!   one at a time: [`ROW`] and [`BUMP`] in, a reversed shorter ease-in out, and `--dur-calm` opacity only under
//!   reduced motion.
//!
//! The tokens and reduced motion live in `theme::Motion`. Anything moving asks for the next frame itself
//! (`window.request_animation_frame()`), so an idle window draws nothing.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

use gpui::{App, ElementId, Window};

use super::theme::{Motion, tokens};

/// The easing CSS draws for `cubic-bezier(x1, y1, x2, y2)`: progress in time (0 to 1) to progress of the value.
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 + Clone + 'static {
    let (x1, y1, x2, y2) = (x1 as f64, y1 as f64, x2 as f64, y2 as f64);
    // B(t) = 3(1-t)^2 t p1 + 3(1-t) t^2 p2 + t^3, with p0 = 0 and p3 = 1.
    let at = |p1: f64, p2: f64, t: f64| {
        ((1. - 3. * p2 + 3. * p1) * t + (3. * p2 - 6. * p1)) * t * t + 3. * p1 * t
    };
    let slope = |p1: f64, p2: f64, t: f64| {
        (3. - 9. * p2 + 9. * p1) * t * t + (6. * p2 - 12. * p1) * t + 3. * p1
    };
    move |x: f32| {
        let x = (x as f64).clamp(0., 1.);
        // Newton first, bisection if it does not settle (flat spots in x).
        let mut t = x;
        for _ in 0..8 {
            let d = slope(x1, x2, t);
            if d.abs() < 1e-6 {
                break;
            }
            t = (t - (at(x1, x2, t) - x) / d).clamp(0., 1.);
        }
        if (at(x1, x2, t) - x).abs() > 1e-6 {
            let (mut lo, mut hi) = (0., 1.);
            for _ in 0..40 {
                t = (lo + hi) / 2.;
                if at(x1, x2, t) < x { lo = t } else { hi = t }
            }
        }
        at(y1, y2, t) as f32
    }
}

/// An easing from a token (`Motion::ease_out` and the others).
pub fn ease(c: [f32; 4]) -> impl Fn(f32) -> f32 + Clone + 'static {
    cubic_bezier(c[0], c[1], c[2], c[3])
}

/// A value moving from `from` to `to` over `dur` from `start`, eased.
#[derive(Clone, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    start: Instant,
    dur: Duration,
    ease: [f32; 4],
}

impl Tween {
    /// At rest at `v`.
    pub fn new(v: f32) -> Tween {
        Tween {
            from: v,
            to: v,
            start: Instant::now(),
            dur: Duration::ZERO,
            ease: [0., 0., 1., 1.],
        }
    }

    pub fn target(&self) -> f32 {
        self.to
    }

    /// The value at `now`.
    pub fn at(&self, now: Instant) -> f32 {
        let t = fraction(self.start, now, self.dur);
        self.from + (self.to - self.from) * ease(self.ease)(t)
    }

    pub fn running(&self, now: Instant) -> bool {
        fraction(self.start, now, self.dur) < 1.
    }

    /// Heads for `v` from wherever it is now, like a CSS transition that changes target half way.
    pub fn go(&mut self, v: f32, now: Instant, dur: Duration, ease: [f32; 4]) {
        if v == self.to {
            return;
        }
        self.from = self.at(now);
        (self.to, self.start, self.dur, self.ease) = (v, now, dur, ease);
    }
}

/// How far through `dur` `now` is, 0 to 1 (1 for a zero duration).
pub fn fraction(start: Instant, now: Instant, dur: Duration) -> f32 {
    if dur.is_zero() {
        return 1.;
    }
    (now.saturating_duration_since(start).as_secs_f32() / dur.as_secs_f32()).clamp(0., 1.)
}

/// A hover transition (`transition: background-color var(--dur-1) var(--ease-out)`): 0 at rest, 1 hovered, eased
/// in between. Kept per `id` across frames; give the element the same id and pass the listener to its `on_hover`.
/// Instant under reduced motion, as the web's transitions are.
pub fn hover(
    id: impl Into<ElementId>,
    window: &mut Window,
    cx: &mut App,
) -> (f32, impl Fn(&bool, &mut Window, &mut App) + 'static) {
    let m = &tokens(cx).motion;
    let dur = m.or(m.dur_1, Duration::ZERO);
    transition(id, dur, window, cx)
}

/// [`hover`] with its own duration (the gear's turn is `--dur-3`). `id` must differ from other transitions'.
pub fn transition(
    id: impl Into<ElementId>,
    dur: Duration,
    window: &mut Window,
    cx: &mut App,
) -> (f32, impl Fn(&bool, &mut Window, &mut App) + 'static) {
    fade(id, dur, dur, window, cx)
}

/// [`transition`] with one length on the way in and another on the way out: a message row's highlight is `--dur-2`
/// in and `--dur-3` out, so moving between rows crossfades.
pub fn fade(
    id: impl Into<ElementId>,
    dur_in: Duration,
    dur_out: Duration,
    window: &mut Window,
    cx: &mut App,
) -> (f32, impl Fn(&bool, &mut Window, &mut App) + 'static) {
    let ease = tokens(cx).motion.ease_out;
    let state = window.use_keyed_state(id, cx, |_, _| Tween::new(0.));
    let now = Instant::now();
    let t = state.read(cx);
    let v = t.at(now);
    if t.running(now) {
        window.request_animation_frame();
    }
    let listen = move |on: &bool, _: &mut Window, cx: &mut App| {
        let (to, dur) = if *on { (1., dur_in) } else { (0., dur_out) };
        state.update(cx, |t, cx| {
            t.go(to, Instant::now(), dur, ease);
            cx.notify();
        });
    };
    (v, listen)
}

/// A motion preset of `motion.ts`: the move's length and easing on the way in.
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub dur: fn(&Motion) -> Duration,
    pub ease: fn(&Motion) -> [f32; 4],
}

/// `row`: a person joins or leaves voice, 8 px from the left, `--dur-2`.
pub const ROW: Preset = Preset {
    dur: |m| m.dur_2,
    ease: |m| m.ease_out,
};
/// `bump`: an unread count comes or changes, scale 0.6 to 1, `--dur-2`, `--ease-pop`.
pub const BUMP: Preset = Preset {
    dur: |m| m.dur_2,
    ease: |m| m.ease_pop,
};

/// `rise`: a new message (and the web's timeline pills), 6 px up, `--dur-3`.
pub const RISE: Preset = Preset {
    dur: |m| m.dur_3,
    ease: |m| m.ease_out,
};
/// `swap`: another room in the timeline, 4 px up, `--dur-2`.
pub const SWAP: Preset = Preset {
    dur: |m| m.dur_2,
    ease: |m| m.ease_out,
};
/// `pop`: popovers, menus and the @ list, scale 0.97 to 1, `--dur-2`.
pub const POP: Preset = Preset {
    dur: |m| m.dur_2,
    ease: |m| m.ease_out,
};

/// The jump flash (`flash` in motion.ts): how much of `--accent-subtle` is on the row, 0 to 1. Five `--dur-2` long,
/// linear: in over the first fifth, held, out over the last. Under reduced motion it is on at once for 600 ms. None once
/// it is over.
pub fn flash(m: &Motion, start: Instant, now: Instant) -> Option<f32> {
    let gone = now.saturating_duration_since(start);
    if m.reduced {
        return (gone < Duration::from_millis(600)).then_some(1.);
    }
    let x = fraction(start, now, m.dur_2 * 5);
    (x < 1.).then(|| (x / 0.2).min(1.).min((1. - x) / 0.2))
}

/// The typing dots (`@keyframes dot` in Composer.svelte): opacity 0.35, up to 1 at 30 %, back down at 60 %, each step
/// ease-out, every 1.2 s; dot `i` starts `i` x 0.15 s late and rests at 0.35 until then.
pub fn dot(m: &Motion, since: Duration, i: usize) -> f32 {
    let delay = Duration::from_millis(150 * i as u64);
    let Some(t) = since.checked_sub(delay) else {
        return 0.35;
    };
    let x = (t.as_secs_f32() / 1.2).fract();
    let e = ease(m.ease_out);
    if x < 0.3 {
        0.35 + 0.65 * e(x / 0.3)
    } else if x < 0.6 {
        1. - 0.65 * e((x - 0.3) / 0.3)
    } else {
        0.35
    }
}

/// Where a preset is: `opacity`, and `t`, how far the move is from its start pose (0) to its rest pose (1). Under
/// reduced motion `t` stays 1 and only the opacity fades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub opacity: f32,
    pub t: f32,
}

/// The pose of `p` started at `start`: entering, or leaving (`out`: backwards in `--dur-out`, ease-in). `None` once
/// it is over.
pub fn pose(m: &Motion, p: Preset, out: bool, start: Instant, now: Instant) -> Option<Pose> {
    let dur = if m.reduced {
        m.dur_calm
    } else if out {
        m.dur_out
    } else {
        (p.dur)(m)
    };
    let x = fraction(start, now, dur);
    if x >= 1. {
        return None;
    }
    let e = ease(if out { m.ease_in } else { (p.ease)(m) })(x);
    // anime.js eases every property with the one easing; the opacity is clamped where ease-pop overshoots.
    let opacity = (if out { 1. - e } else { e }).clamp(0., 1.);
    let t = if m.reduced {
        1.
    } else if out {
        1. - e
    } else {
        e
    };
    Some(Pose { opacity, t })
}

/// Things that come and go, by key: when each came, and the ones that left with where they were and when, kept until
/// `keep` has passed. While calm (`settle`, like `motion.ts`: the first hello fills the sidebar), arrivals count as
/// long since and departures are not kept.
#[derive(Debug)]
pub struct Roster<K, V> {
    came: HashMap<K, Instant>,
    /// (key, value, index it had, when it left).
    gone: Vec<(K, V, usize, Instant)>,
    last: Vec<(K, V)>,
    calm_until: Option<Instant>,
}

impl<K, V> Default for Roster<K, V> {
    fn default() -> Self {
        Roster {
            came: HashMap::new(),
            gone: vec![],
            last: vec![],
            calm_until: None,
        }
    }
}

impl<K: Hash + Eq + Clone, V: Clone> Roster<K, V> {
    /// No motion until `until`.
    pub fn settle(&mut self, until: Instant) {
        self.calm_until = Some(until);
    }

    /// The keys shown now, in order.
    pub fn sync(&mut self, now: Instant, keep: Duration, cur: Vec<(K, V)>) {
        let calm = self.calm_until.is_some_and(|u| now < u);
        let long_ago = now.checked_sub(Duration::from_secs(60)).unwrap_or(now);
        for (k, _) in &cur {
            if !self.came.contains_key(k) {
                self.came
                    .insert(k.clone(), if calm { long_ago } else { now });
            }
            self.gone.retain(|g| &g.0 != k);
        }
        for (i, (k, v)) in self.last.iter().enumerate() {
            if !cur.iter().any(|c| &c.0 == k) {
                self.came.remove(k);
                if !calm {
                    self.gone.push((k.clone(), v.clone(), i, now));
                }
            }
        }
        self.gone
            .retain(|g| now.saturating_duration_since(g.3) < keep);
        self.last = cur;
    }

    /// When `k` came (long ago for the ones there from the start).
    pub fn came(&self, k: &K) -> Option<Instant> {
        self.came.get(k).copied()
    }

    pub fn gone(&self) -> &[(K, V, usize, Instant)] {
        &self.gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_css_cubic_bezier() {
        // Reference values: x solved by bisection to 1e-30 in JS. `ease` at 0.5 is the well-known 0.8024.
        let cases: [([f32; 4], [f32; 5]); 4] = [
            (
                [0.25, 0.1, 0.25, 1.],
                [0.094796, 0.408511, 0.802403, 0.960459, 0.994316],
            ),
            (
                [0.42, 0., 1., 1.],
                [0.017027, 0.093465, 0.315357, 0.621862, 0.839428],
            ),
            (
                Motion::default().ease_out,
                [0.15625, 0.60722, 0.877834, 0.97548, 0.996459],
            ),
            (
                Motion::default().ease_pop,
                [0.403933, 0.816289, 1.087401, 1.059647, 1.012616],
            ),
        ];
        for (c, want) in cases {
            let f = ease(c);
            for (x, w) in [0.1, 0.25, 0.5, 0.75, 0.9].into_iter().zip(want) {
                assert!((f(x) - w).abs() < 1e-4, "{c:?} at {x}: {} want {w}", f(x));
            }
            assert!(f(0.).abs() < 1e-6 && (f(1.) - 1.).abs() < 1e-6);
        }
    }

    #[test]
    fn reduced_motion_is_calm() {
        let mut m = Motion::default();
        assert_eq!(m.dur(m.dur_3), m.dur_3);
        m.reduced = true;
        assert_eq!(m.dur(m.dur_3), m.dur_calm);
    }

    #[test]
    fn a_tween_eases_and_turns_back_from_where_it_is() {
        let m = Motion::default();
        let t0 = Instant::now();
        let mut t = Tween::new(0.);
        t.go(1., t0, m.dur_1, m.ease_out);
        let half = t.at(t0 + m.dur_1 / 2);
        assert!((half - ease(m.ease_out)(0.5)).abs() < 1e-5, "{half}");
        assert!(t.running(t0 + m.dur_1 / 2) && !t.running(t0 + m.dur_1));
        assert_eq!(t.at(t0 + m.dur_1), 1.);
        // Back half way: from the value it had, not from 1.
        t.go(0., t0 + m.dur_1 / 2, m.dur_1, m.ease_out);
        assert!((t.at(t0 + m.dur_1 / 2) - half).abs() < 1e-5);
        assert_eq!(t.at(t0 + m.dur_1 * 2), 0.);
    }

    #[test]
    fn presets_pose_in_and_out() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let p = pose(&m, ROW, false, t0, t0 + m.dur_2 / 2).unwrap();
        assert!((p.t - ease(m.ease_out)(0.5)).abs() < 1e-5 && p.opacity == p.t);
        assert!(pose(&m, ROW, false, t0, t0 + m.dur_2).is_none());
        // ease-pop overshoots the scale, never the opacity.
        let b = pose(&m, BUMP, false, t0, t0 + m.dur_2 * 3 / 4).unwrap();
        assert!(b.t > 1. && b.opacity == 1.);
        let o = pose(&m, ROW, true, t0, t0 + m.dur_out / 2).unwrap();
        assert!((o.t - (1. - ease(m.ease_in)(0.5))).abs() < 1e-5);
        assert!(pose(&m, ROW, true, t0, t0 + m.dur_out).is_none());
        m.reduced = true;
        let r = pose(&m, ROW, false, t0, t0 + m.dur_calm / 2).unwrap();
        assert_eq!(r.t, 1., "no move under reduced motion");
        assert!(r.opacity > 0. && r.opacity < 1.);
    }

    #[test]
    fn the_flash_and_the_typing_dots() {
        let mut m = Motion::default();
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        // 5 x 160 ms: in over 160, held to 640, out by 800.
        assert!((flash(&m, t0, at(80)).unwrap() - 0.5).abs() < 0.01);
        assert_eq!(flash(&m, t0, at(400)), Some(1.));
        assert!((flash(&m, t0, at(720)).unwrap() - 0.5).abs() < 0.01);
        assert_eq!(flash(&m, t0, at(800)), None);
        m.reduced = true;
        assert_eq!(flash(&m, t0, at(10)), Some(1.), "no fade");
        assert_eq!(flash(&m, t0, at(600)), None);
        let m = Motion::default();
        let ms = Duration::from_millis;
        assert_eq!(dot(&m, ms(0), 0), 0.35);
        assert!((dot(&m, ms(360), 0) - 1.).abs() < 1e-4, "the top at 30 %");
        assert_eq!(dot(&m, ms(800), 0), 0.35, "rests from 60 %");
        assert_eq!(dot(&m, ms(100), 1), 0.35, "still waiting its 0.15 s");
        assert!((dot(&m, ms(150 + 360), 1) - 1.).abs() < 1e-4);
        assert!(
            (dot(&m, ms(1200 + 300 + 360), 2) - 1.).abs() < 1e-4,
            "it repeats"
        );
    }

    #[test]
    fn a_roster_sees_who_came_and_who_left() {
        let t0 = Instant::now();
        let keep = Duration::from_millis(120);
        let mut r: Roster<&str, i32> = Roster::default();
        r.settle(t0 + Duration::from_secs(1));
        r.sync(t0, keep, vec![("a", 1), ("b", 2)]);
        assert!(
            r.came(&"a").unwrap() < t0,
            "there from the start: no motion"
        );
        r.sync(t0, keep, vec![("b", 2)]);
        assert!(r.gone().is_empty(), "calm: no exit");
        let t1 = t0 + Duration::from_secs(2);
        r.sync(t1, keep, vec![("b", 2), ("c", 3)]);
        assert_eq!(r.came(&"c"), Some(t1));
        r.sync(t1, keep, vec![("c", 3)]);
        assert_eq!(r.gone(), [("b", 2, 0, t1)]);
        r.sync(t1 + keep, keep, vec![("c", 3)]);
        assert!(r.gone().is_empty(), "dropped once its exit is over");
        // Back before the exit ended: in again.
        r.sync(t1, keep, vec![]);
        r.sync(t1, keep, vec![("c", 3)]);
        assert!(r.gone().is_empty() && r.came(&"c") == Some(t1));
    }
}
