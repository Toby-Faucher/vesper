//! The web app's design tokens (`web/src/styles/tokens.css`, the accent in `web/src/lib/ui/accent.ts`) as a runtime
//! [`Tokens`] global. Components read roles only: `tokens(cx).fg_2`.
//! [`set`] switches scheme or accent at runtime and repaints gpui-component's theme from the roles.
//!
//! The `pub const` colours below are the default look (Vesper, dark, amber) for older call sites; a test keeps them equal
//! to `Tokens::default()`. New code reads the global.

use std::sync::OnceLock;
use std::time::Duration;

use gpui::{
    App, BoxShadow, FontWeight, Global, Hsla, Pixels, Rgba, SharedString, Styled, point, px, rgb,
    rgba,
};
use gpui_component::{Theme, ThemeMode};

pub const BG_APP: u32 = 0x100e0c;
pub const BG_SIDEBAR: u32 = 0x151311;
pub const BG_RAISED: u32 = 0x1a1816;
pub const BG_HOVER: u32 = 0x201e1b;
pub const BG_SELECTED: u32 = 0x262422;
pub const LINE_SUBTLE: u32 = 0x2d2b29;
pub const LINE: u32 = 0x3a3835;
pub const FG: u32 = 0xeeecea;
pub const FG_2: u32 = 0xb3b1ad;
pub const FG_3: u32 = 0x888683;
pub const ACCENT: u32 = 0xf0b010;
/// The accent roles are what `accent.ts` computes from amber (the web app paints them over tokens.css at start).
pub const ACCENT_HOVER: u32 = 0xf2bc34;
pub const ACCENT_FG: u32 = 0xf0bc3c;
pub const ACCENT_SUBTLE: u32 = 0x382b0d;
pub const ON_ACCENT: u32 = 0x100e0c;
pub const DANGER_FG: u32 = 0xf66d67;
pub const DANGER_SOLID: u32 = 0xba2b2e;
pub const WARN_FG: u32 = 0xf7945a;

/// The type scale of `web/src/app.css` (`--text-*`): size and line height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStep {
    pub size: f32,
    pub line: f32,
}
pub const TEXT_XS: TextStep = TextStep {
    size: 12.,
    line: 16.,
};
/// The body text: `body { text-sm }`.
pub const TEXT_SM: TextStep = TextStep {
    size: 13.,
    line: 20.,
};
pub const TEXT_BASE: TextStep = TextStep {
    size: 14.,
    line: 20.,
};
pub const TEXT_LG: TextStep = TextStep {
    size: 16.,
    line: 24.,
};
pub const TEXT_XL: TextStep = TextStep {
    size: 20.,
    line: 28.,
};
pub const TEXT_2XL: TextStep = TextStep {
    size: 24.,
    line: 32.,
};
/// Tailwind's `font-semibold`, used by headings outside the tokens (Login's h1).
pub const SEMIBOLD: FontWeight = FontWeight(600.);

/// `.text_step(TEXT_SM)`: a step's size and line height in one call.
pub trait TextStepExt: Styled + Sized {
    fn text_step(self, s: TextStep) -> Self {
        self.text_size(px(s.size)).line_height(px(s.line))
    }
}
impl<T: Styled> TextStepExt for T {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Scheme {
    #[default]
    Dark,
    Light,
}

/// The accent presets of `accent.ts`: id, name, fill.
pub const PRESETS: [(&str, &str, u32); 8] = [
    ("amber", "Amber", 0xf0b010),
    ("rose", "Rose", 0xf0819f),
    ("orchid", "Orchid", 0xd98ae0),
    ("violet", "Violet", 0xb69cf4),
    ("indigo", "Indigo", 0x949ff6),
    ("sky", "Sky", 0x4db9ee),
    ("teal", "Teal", 0x2fc4b2),
    ("lime", "Lime", 0xa6cf4c),
];
pub const AMBER: u32 = 0xf0b010;

/// The motion tokens of tokens.css, and reduced motion (`motion.ts`: every preset becomes a `dur_calm` opacity fade).
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    pub dur_1: Duration,
    pub dur_2: Duration,
    pub dur_3: Duration,
    /// Exits are shorter and ease in.
    pub dur_out: Duration,
    /// Every preset's duration under reduced motion.
    pub dur_calm: Duration,
    pub ease_out: [f32; 4],
    pub ease_in: [f32; 4],
    /// Overshoots a little.
    pub ease_pop: [f32; 4],
    /// The OS asks for less motion. The web reads `prefers-reduced-motion`; GPUI 0.2.2 does not expose it, so it is
    /// off until a setting or the platform sets it.
    pub reduced: bool,
}

impl Default for Motion {
    fn default() -> Self {
        Motion {
            dur_1: Duration::from_millis(100),
            dur_2: Duration::from_millis(160),
            dur_3: Duration::from_millis(220),
            dur_out: Duration::from_millis(120),
            dur_calm: Duration::from_millis(80),
            ease_out: [0.2, 0., 0., 1.],
            ease_in: [0.4, 0., 1., 1.],
            ease_pop: [0.34, 1.56, 0.64, 1.],
            reduced: false,
        }
    }
}

impl Motion {
    /// `d`, or `dur_calm` under reduced motion (presets). CSS transitions drop to nothing there; pass `Duration::ZERO`
    /// as `calm` for those through [`Motion::or`].
    pub fn dur(&self, d: Duration) -> Duration {
        self.or(d, self.dur_calm)
    }

    pub fn or(&self, d: Duration, calm: Duration) -> Duration {
        if self.reduced { calm } else { d }
    }
}

/// Every role of tokens.css for one scheme and accent.
#[derive(Clone, Debug, PartialEq)]
pub struct Tokens {
    pub scheme: Scheme,
    /// The accent fill as chosen (`fillOf`), before the light scheme darkens it: the logo is tinted to it.
    pub fill: u32,
    pub bg_app: Rgba,
    pub bg_sidebar: Rgba,
    pub bg_raised: Rgba,
    pub bg_element: Rgba,
    pub bg_hover: Rgba,
    pub bg_selected: Rgba,
    pub line_subtle: Rgba,
    pub line: Rgba,
    pub line_strong: Rgba,
    pub fg: Rgba,
    pub fg_2: Rgba,
    pub fg_3: Rgba,
    pub fg_disabled: Rgba,
    pub accent: Rgba,
    pub accent_hover: Rgba,
    pub on_accent: Rgba,
    pub accent_fg: Rgba,
    pub accent_subtle: Rgba,
    pub accent_line: Rgba,
    pub focus: Rgba,
    pub danger_fg: Rgba,
    pub danger_solid: Rgba,
    pub on_danger: Rgba,
    pub danger_subtle: Rgba,
    pub warn_fg: Rgba,
    pub warn_subtle: Rgba,
    pub ok_fg: Rgba,
    pub ok_subtle: Rgba,
    /// `--avatar-1` to `--avatar-8`.
    pub avatar: [Rgba; 8],
    pub on_avatar: Rgba,
    pub syn_kw: Rgba,
    pub syn_str: Rgba,
    pub syn_num: Rgba,
    pub syn_ty: Rgba,
    pub syn_com: Rgba,
    pub weight_strong: FontWeight,
    pub weight_ui: FontWeight,
    pub font_sans: SharedString,
    pub font_mono: SharedString,
    pub radius_sm: Pixels,
    pub radius: Pixels,
    pub radius_lg: Pixels,
    pub shadow_float: Vec<BoxShadow>,
    pub motion: Motion,
}

impl Global for Tokens {}

impl Default for Tokens {
    fn default() -> Self {
        Tokens::new(Scheme::Dark, None)
    }
}

const DARK_GRAY: [u32; 12] = [
    0x100e0c, 0x151311, 0x1a1816, 0x201e1b, 0x262422, 0x2d2b29, 0x3a3835, 0x4f4d4a, 0x767471,
    0x888683, 0xb3b1ad, 0xeeecea,
];
const LIGHT_GRAY: [u32; 12] = [
    0xfbfaf8, 0xf4f2ef, 0xffffff, 0xece9e5, 0xe3e0db, 0xd9d6d0, 0xc8c5bf, 0xa8a59f, 0x8d8a85,
    0x6a6763, 0x4d4b47, 0x1b1917,
];
fn shadow(x: f32, y: f32, blur: f32, spread: f32, color: u32) -> BoxShadow {
    BoxShadow {
        color: rgba(color).into(),
        offset: point(px(x), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
    }
}

impl Tokens {
    /// The look for the scheme and the account's accent (a preset id or `#rrggbb`; `None` or a refused colour is
    /// amber), as the web app paints it.
    pub fn new(scheme: Scheme, accent: Option<&str>) -> Tokens {
        let gray = match scheme {
            Scheme::Light => LIGHT_GRAY,
            Scheme::Dark => DARK_GRAY,
        };
        let g = |i: usize| rgb(gray[i - 1]);
        let (sans, mono) = fonts();
        // :root, with the grays of the scheme.
        let mut t = Tokens {
            scheme,
            fill: fill_of(accent),
            bg_app: g(1),
            bg_sidebar: g(2),
            bg_raised: g(3),
            bg_element: g(3),
            bg_hover: g(4),
            bg_selected: g(5),
            line_subtle: g(6),
            line: g(7),
            line_strong: g(8),
            fg: g(12),
            fg_2: g(11),
            fg_3: g(10),
            fg_disabled: g(9),
            accent: rgb(0xf0b010),
            accent_hover: rgb(0xffbe15),
            on_accent: g(1),
            accent_fg: rgb(0xf7c243),
            accent_subtle: rgb(0x2f1e00),
            accent_line: rgb(0x754c00),
            focus: rgb(0xf0b010),
            danger_fg: rgb(0xf66d67),
            danger_solid: rgb(0xba2b2e),
            on_danger: g(12),
            danger_subtle: rgb(0x3a1412),
            warn_fg: rgb(0xf7945a),
            warn_subtle: rgb(0x371c0c),
            ok_fg: rgb(0x68ca80),
            ok_subtle: rgb(0x102816),
            avatar: [
                0xed8d7d, 0xe5955d, 0x99b860, 0x6fc082, 0x31c3b5, 0x41bae4, 0x6db0f4, 0xe68aaf,
            ]
            .map(rgb),
            on_avatar: g(1),
            syn_kw: rgb(0xb8a4e6),
            syn_str: rgb(0xa3c58a),
            syn_num: rgb(0xe09bbf),
            syn_ty: rgb(0x86b9ec),
            syn_com: g(10),
            weight_strong: FontWeight(600.),
            weight_ui: FontWeight(500.),
            font_sans: sans,
            font_mono: mono,
            radius_sm: px(4.),
            radius: px(6.),
            radius_lg: px(10.),
            shadow_float: vec![shadow(0., 4., 16., 0., 0x00000059)],
            motion: Motion::default(),
        };
        if scheme == Scheme::Light {
            t.light();
        }
        t.syn_com = t.fg_3;
        t.set_accent(&roles(t.fill, scheme));
        t
    }

    fn set_accent(&mut self, r: &AccentRoles) {
        self.accent = rgb(r.accent);
        self.accent_hover = rgb(r.accent_hover);
        self.on_accent = rgb(r.on_accent);
        self.accent_fg = rgb(r.accent_fg);
        self.accent_subtle = rgb(r.accent_subtle);
        self.accent_line = rgb(r.accent_line);
        self.focus = rgb(r.focus);
    }

    /// `html[data-scheme='light']`, past the grays.
    fn light(&mut self) {
        self.set_accent(&AccentRoles {
            accent: 0x976f0a,
            accent_hover: 0x856209,
            on_accent: 0xffffff,
            accent_fg: 0x856209,
            accent_subtle: 0xede7d7,
            accent_line: 0xc9b581,
            focus: 0x976f0a,
        });
        (
            self.danger_fg,
            self.danger_solid,
            self.on_danger,
            self.danger_subtle,
        ) = (rgb(0xb42318), rgb(0xc4281f), rgb(0xffffff), rgb(0xfde8e6));
        (self.warn_fg, self.warn_subtle) = (rgb(0xa14a00), rgb(0xfdebd9));
        (self.ok_fg, self.ok_subtle) = (rgb(0x1a7a3a), rgb(0xe1f4e6));
        self.avatar = [
            0xa66357, 0xa06841, 0x677b40, 0x497f56, 0x208177, 0x2c7e9b, 0x4a78a6, 0xa1617a,
        ]
        .map(rgb);
        self.on_avatar = rgb(0xffffff);
        (self.syn_kw, self.syn_str, self.syn_num, self.syn_ty) =
            (rgb(0x6f4fb8), rgb(0x3f7a22), rgb(0xb0346f), rgb(0x1f6aae));
        self.shadow_float = vec![shadow(0., 4., 16., 0., 0x281e1429)];
    }
}

/// The global tokens.
pub fn tokens(cx: &App) -> &Tokens {
    cx.global::<Tokens>()
}

/// The default look, with gpui-component's colours set from it.
pub fn apply(cx: &mut App) {
    set(Tokens::default(), cx);
}

/// Switches the look: the global and gpui-component's theme. Windows redraw on their next frame.
pub fn set(t: Tokens, cx: &mut App) {
    let mode = if t.scheme == Scheme::Light {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    Theme::change(mode, None, cx);
    let c = |x: Rgba| -> Hsla { x.into() };
    let th = Theme::global_mut(cx);
    th.font_family = t.font_sans.clone();
    th.mono_font_family = t.font_mono.clone();
    // gpui-component makes this the window's rem: 16 px as in the browser, so `p_6()` is Tailwind's 24 px. The text size
    // (13 px, `body`) is set on the root view.
    th.font_size = px(16.);
    th.radius = t.radius;
    th.radius_lg = t.radius_lg;
    th.shadow = false;
    th.background = c(t.bg_app);
    th.foreground = c(t.fg);
    th.border = c(t.line);
    th.input = c(t.line);
    th.ring = c(t.focus);
    th.caret = c(t.fg);
    th.selection = c(t.accent_subtle);
    th.primary = c(t.accent);
    th.primary_hover = c(t.accent_hover);
    th.primary_active = c(t.accent_hover);
    th.primary_foreground = c(t.on_accent);
    th.secondary = c(t.bg_element);
    th.secondary_hover = c(t.bg_hover);
    th.secondary_active = c(t.bg_selected);
    th.secondary_foreground = c(t.fg);
    th.muted = c(t.bg_raised);
    th.muted_foreground = c(t.fg_3);
    th.accent = c(t.bg_hover);
    th.accent_foreground = c(t.fg);
    th.danger = c(t.danger_solid);
    th.danger_foreground = c(t.on_danger);
    th.popover = c(t.bg_raised);
    th.popover_foreground = c(t.fg);
    th.sidebar = c(t.bg_sidebar);
    th.sidebar_foreground = c(t.fg_2);
    th.sidebar_border = c(t.line_subtle);
    th.list = c(t.bg_app);
    th.list_hover = c(t.bg_hover);
    th.list_active = c(t.bg_selected);
    th.scrollbar = c(t.bg_app);
    th.scrollbar_thumb = c(t.line_strong);
    th.scrollbar_thumb_hover = c(t.fg_3);
    cx.set_global(t);
}

/// The families the web's stacks resolve to here. `system-ui` is Segoe UI on Windows and the system face on macOS; on
/// Linux Chromium asks fontconfig for the default sans (Liberation Sans on the dev machine), and so does this.
/// `ui-monospace, 'Cascadia Mono', ..., Consolas, monospace` is the same idea for code.
fn fonts() -> (SharedString, SharedString) {
    static F: OnceLock<(SharedString, SharedString)> = OnceLock::new();
    F.get_or_init(|| {
        if cfg!(windows) {
            let cascadia = std::env::var_os("WINDIR").is_some_and(|w| {
                std::path::Path::new(&w)
                    .join("Fonts/CascadiaMono.ttf")
                    .exists()
            });
            let mono = if cascadia {
                "Cascadia Mono"
            } else {
                "Consolas"
            };
            ("Segoe UI".into(), mono.into())
        } else if cfg!(target_os = "macos") {
            (".SystemUIFont".into(), "Menlo".into())
        } else {
            let fc = |pattern: &str, or: &'static str| -> SharedString {
                std::process::Command::new("fc-match")
                    .args(["-f", "%{family[0]}", pattern])
                    .output()
                    .ok()
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .filter(|s| !s.trim().is_empty())
                    .map_or(or.into(), |s| s.trim().to_string().into())
            };
            (
                fc("sans-serif", "Liberation Sans"),
                fc("monospace", "DejaVu Sans Mono"),
            )
        }
    })
    .clone()
}

/// The accent roles of one fill (`accent.ts` `roles`), as `0xrrggbb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccentRoles {
    pub accent: u32,
    pub accent_hover: u32,
    pub on_accent: u32,
    pub accent_fg: u32,
    pub accent_subtle: u32,
    pub accent_line: u32,
    pub focus: u32,
}

/// The surfaces the accent rules check against: app, sidebar, raised, fg (`SURFACE` and `SURFACE_LIGHT`).
const SURFACE: [u32; 4] = [0x100e0c, 0x151311, 0x1a1816, 0xeeecea];
const SURFACE_LIGHT: [u32; 4] = [0xfbfaf8, 0xf4f2ef, 0xffffff, 0x1b1917];
const WHITE: u32 = 0xffffff;
const BLACK: u32 = 0x000000;

fn chans(h: u32) -> [f64; 3] {
    [(h >> 16) & 0xff, (h >> 8) & 0xff, h & 0xff].map(|c| c as f64)
}

/// WCAG relative luminance.
pub fn lum(h: u32) -> f64 {
    let c = chans(h).map(|x| {
        let x = x / 255.;
        if x <= 0.03928 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    });
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn contrast(a: u32, b: u32) -> f64 {
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// `a` moved `t` of the way to `b` in sRGB, rounded like JS `Math.round`.
fn mix(a: u32, b: u32, t: f64) -> u32 {
    let (p, q) = (chans(a), chans(b));
    (0..3).fold(0, |acc, i| {
        (acc << 8) | (p[i] + (q[i] - p[i]) * t + 0.5).floor() as u32
    })
}

/// Every accent role from one fill, for the scheme (`accent.ts` `roles`, the same steps and rounding).
pub fn roles(fill: u32, scheme: Scheme) -> AccentRoles {
    if scheme == Scheme::Light {
        let s = SURFACE_LIGHT;
        let dark = |c: u32, ok: &dyn Fn(u32) -> bool| {
            let mut t = 0.;
            while t < 1. && !ok(mix(c, BLACK, t)) {
                t += 0.01;
            }
            mix(c, BLACK, t)
        };
        let f = dark(fill, &|x| contrast(WHITE, x) >= 4.5);
        let subtle = mix(f, s[0], 0.86);
        return AccentRoles {
            accent: f,
            accent_hover: mix(f, BLACK, 0.12),
            on_accent: WHITE,
            accent_fg: dark(f, &|x| {
                contrast(x, s[0]) >= 4.5 && contrast(x, subtle) >= 4.5
            }),
            accent_subtle: subtle,
            accent_line: mix(f, s[0], 0.5),
            focus: f,
        };
    }
    AccentRoles {
        accent: fill,
        accent_hover: mix(fill, WHITE, 0.15),
        on_accent: SURFACE[0],
        accent_fg: mix(fill, SURFACE[3], 0.2),
        accent_subtle: mix(fill, SURFACE[0], 0.82),
        accent_line: mix(fill, SURFACE[0], 0.5),
        focus: fill,
    }
}

/// `check(fill)` in dark: the fill passes every contrast rule Settings asks of a custom colour.
pub fn allowed(fill: u32) -> bool {
    let (r, s) = (roles(fill, Scheme::Dark), SURFACE);
    [
        (contrast(r.on_accent, r.accent), 4.5),
        (contrast(r.on_accent, r.accent_hover), 4.5),
        (contrast(r.accent, s[0]), 3.),
        (contrast(r.accent, s[1]), 3.),
        (contrast(r.accent, s[2]), 3.),
        (contrast(r.accent_fg, s[0]), 4.5),
        (contrast(r.accent_fg, r.accent_subtle), 4.5),
    ]
    .iter()
    .all(|(ratio, need)| ratio >= need)
}

/// The fill for a stored choice: a preset id, an allowed `#rrggbb`, else amber.
pub fn fill_of(v: Option<&str>) -> u32 {
    let Some(v) = v else { return AMBER };
    if let Some(p) = PRESETS.iter().find(|p| p.0 == v) {
        return p.2;
    }
    match v.strip_prefix('#') {
        Some(h) if h.len() == 6 => u32::from_str_radix(h, 16)
            .ok()
            .filter(|&f| allowed(f))
            .unwrap_or(AMBER),
        _ => AMBER,
    }
}

/// The logo's colour matrix (`accent.ts` `tint`, the `#logo-tint` feColorMatrix): row `i` gives output channel `i`
/// from the input r, g, b (0 to 1, sRGB). It moves amber to `fill` and keeps every grey, so the white letters and the
/// black outline stay. The identity for amber.
pub fn tint(fill: u32) -> [[f64; 3]; 3] {
    let (a, t) = (
        chans(AMBER).map(|x| x / 255.),
        chans(fill).map(|x| x / 255.),
    );
    let mean = (a[0] + a[1] + a[2]) / 3.;
    let c = a.map(|x| x - mean); // amber off the grey axis: greys give 0, amber gives 1
    let n = c.iter().map(|x| x * x).sum::<f64>();
    std::array::from_fn(|i| {
        std::array::from_fn(|j| f64::from(u8::from(i == j)) + (t[i] - a[i]) * c[j] / n)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Rgba) -> u32 {
        let b = |x: f32| (x * 255.).round() as u32;
        (b(c.r) << 16) | (b(c.g) << 8) | b(c.b)
    }

    fn r(a: [u32; 7]) -> AccentRoles {
        AccentRoles {
            accent: a[0],
            accent_hover: a[1],
            on_accent: a[2],
            accent_fg: a[3],
            accent_subtle: a[4],
            accent_line: a[5],
            focus: a[6],
        }
    }

    #[test]
    fn roles_match_accent_ts() {
        // Printed by accent.ts roles() under vitest; amber light is also the light block of tokens.css.
        let want = [
            (
                "amber",
                Scheme::Dark,
                [
                    0xf0b010, 0xf2bc34, 0x100e0c, 0xf0bc3c, 0x382b0d, 0x805f0e, 0xf0b010,
                ],
            ),
            (
                "amber",
                Scheme::Light,
                [
                    0x976f0a, 0x856209, 0xffffff, 0x856209, 0xede7d7, 0xc9b581, 0x976f0a,
                ],
            ),
            (
                "sky",
                Scheme::Dark,
                [
                    0x4db9ee, 0x68c4f1, 0x100e0c, 0x6dc3ed, 0x1b2d35, 0x2f647d, 0x4db9ee,
                ],
            ),
            (
                "sky",
                Scheme::Light,
                [
                    0x347ea2, 0x2e6f8f, 0xffffff, 0x2d6e8d, 0xdfe9ec, 0x98bccd, 0x347ea2,
                ],
            ),
            (
                "rose",
                Scheme::Dark,
                [
                    0xf0819f, 0xf294ad, 0x100e0c, 0xf096ae, 0x382326, 0x804856, 0xf0819f,
                ],
            ),
            (
                "rose",
                Scheme::Light,
                [
                    0xad5d72, 0x985264, 0xffffff, 0x985264, 0xf0e4e5, 0xd4acb5, 0xad5d72,
                ],
            ),
        ];
        for (id, scheme, roles_want) in want {
            assert_eq!(
                roles(fill_of(Some(id)), scheme),
                r(roles_want),
                "{id} {scheme:?}"
            );
        }
    }

    #[test]
    fn fill_of_falls_back_to_amber() {
        assert_eq!(fill_of(None), AMBER);
        assert_eq!(fill_of(Some("teal")), 0x2fc4b2);
        assert_eq!(fill_of(Some("#a0c0ff")), 0xa0c0ff);
        for bad in ["#3050a0", "red", "#zzzzzz", ""] {
            assert_eq!(fill_of(Some(bad)), AMBER, "{bad}");
        }
        assert!(PRESETS.iter().all(|p| allowed(p.2)));
    }

    #[test]
    fn default_tokens_match_the_old_constants() {
        let t = Tokens::default();
        for (got, want) in [
            (t.bg_app, BG_APP),
            (t.bg_sidebar, BG_SIDEBAR),
            (t.bg_raised, BG_RAISED),
            (t.bg_hover, BG_HOVER),
            (t.bg_selected, BG_SELECTED),
            (t.line_subtle, LINE_SUBTLE),
            (t.line, LINE),
            (t.fg, FG),
            (t.fg_2, FG_2),
            (t.fg_3, FG_3),
            (t.accent, ACCENT),
            (t.accent_hover, ACCENT_HOVER),
            (t.accent_fg, ACCENT_FG),
            (t.accent_subtle, ACCENT_SUBTLE),
            (t.on_accent, ON_ACCENT),
            (t.danger_fg, DANGER_FG),
            (t.danger_solid, DANGER_SOLID),
            (t.warn_fg, WARN_FG),
        ] {
            assert_eq!(hex(got), want);
        }
    }

    #[test]
    fn the_logo_tint_is_the_identity_for_amber_and_moves_amber_to_the_fill() {
        let id = tint(AMBER);
        for (i, row) in id.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - f64::from(u8::from(i == j))).abs() < 1e-9);
            }
        }
        let through = |m: [[f64; 3]; 3], c: u32| -> [f64; 3] {
            let x = chans(c).map(|v| v / 255.);
            m.map(|r| (r[0] * x[0] + r[1] * x[1] + r[2] * x[2]) * 255.)
        };
        for (_, _, fill) in PRESETS {
            let got = through(tint(fill), AMBER);
            let want = chans(fill);
            assert!((0..3).all(|i| (got[i] - want[i]).abs() < 0.5), "{fill:06x}");
            let grey = through(tint(fill), 0x808080);
            assert!(grey.iter().all(|v| (v - 128.).abs() < 0.5), "greys stay");
        }
    }

    #[test]
    fn light_scheme_and_a_chosen_accent() {
        let light = Tokens::new(Scheme::Light, None);
        assert_eq!(
            (hex(light.bg_raised), hex(light.accent)),
            (0xffffff, 0x976f0a)
        );
        assert_eq!(hex(Tokens::new(Scheme::Dark, Some("sky")).accent), 0x4db9ee);
    }
}
