//! CSS `cubic-bezier()` easing for GPUI animations: `Animation::new(d).with_easing(cubic_bezier(0.2, 0., 0., 1.))`.
//! The motion tokens and reduced motion live in `theme::Motion`.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::Motion;

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
}
