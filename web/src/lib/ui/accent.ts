import type { AccentPreset } from '../gen/AccentPreset';
// The accent: one colour in, every accent role out. Besides tokens.css this is the one file with colour values: the presets
// and the surfaces the contrast rules check against (accent.test.ts checks those equal tokens.css).

export const PRESETS: Record<AccentPreset, { name: string; hex: string }> = {
  amber: { name: 'Amber', hex: '#f0b010' }, // the logo colour, --amber-9
  rose: { name: 'Rose', hex: '#f0819f' },
  orchid: { name: 'Orchid', hex: '#d98ae0' },
  violet: { name: 'Violet', hex: '#b69cf4' },
  indigo: { name: 'Indigo', hex: '#949ff6' },
  sky: { name: 'Sky', hex: '#4db9ee' },
  teal: { name: 'Teal', hex: '#2fc4b2' },
  lime: { name: 'Lime', hex: '#a6cf4c' },
};
export const SURFACE = { app: '#100e0c', sidebar: '#151311', raised: '#1a1816', fg: '#eeecea' }; // --gray-1, 2, 3, 12
/** The light scheme's surfaces, the same four roles (--gray-1, 2, 3, 12 under html[data-scheme='light']). */
export const SURFACE_LIGHT = { app: '#fbfaf8', sidebar: '#f4f2ef', raised: '#ffffff', fg: '#1b1917' };
export type Scheme = 'dark' | 'light';
const WHITE = '#ffffff';
const BLACK = '#000000';
export const EXAMPLE = '#a0c0ff'; // the custom field's placeholder
export const ACCENT_KEY = 'vesper.accent';
let tinted = false; // the favicon was drawn once, so amber has to put the file back
const storage = (() => { try { return localStorage; } catch { return null; } })();

const rgb = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
/** WCAG relative luminance of `#rrggbb`. */
export const lum = (h: string) => {
  const c = rgb(h).map((x) => x / 255).map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
export const contrast = (a: string, b: string) => {
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};
/** `a` moved `t` (0 to 1) of the way to `b`, in sRGB. */
const mix = (a: string, b: string, t: number) => {
  const [p, q] = [rgb(a), rgb(b)];
  return '#' + p.map((v, i) => Math.round(v + (q[i] - v) * t).toString(16).padStart(2, '0')).join('');
};

/** Every accent role from one fill, for the scheme. Dark: text on the fill is --on-accent (--gray-1, near-black). Light: the fill is
 * darkened until white text on it passes 4.5 (the same hue, so a preset keeps its name), and every other role follows from that. */
export function roles(fill: string, scheme: Scheme = 'dark'): Record<string, string> {
  if (scheme === 'light') {
    const S = SURFACE_LIGHT;
    const dark = (c: string, ok: (x: string) => boolean) => { // the lightest mix of `c` towards black that passes
      let t = 0;
      while (t < 1 && !ok(mix(c, BLACK, t))) t += 0.01;
      return mix(c, BLACK, t);
    };
    const f = dark(fill, (x) => contrast(WHITE, x) >= 4.5);
    const subtle = mix(f, S.app, 0.86);
    return {
      '--accent': f,
      '--accent-hover': mix(f, BLACK, 0.12),
      '--on-accent': WHITE,
      '--accent-fg': dark(f, (x) => contrast(x, S.app) >= 4.5 && contrast(x, subtle) >= 4.5), // links, mentions, the unread line
      '--accent-subtle': subtle, // soft fills, text selection
      '--accent-line': mix(f, S.app, 0.5),
      '--focus': f,
    };
  }
  return {
    '--accent': fill,
    '--accent-hover': mix(fill, WHITE, 0.15),
    '--on-accent': SURFACE.app,
    '--accent-fg': mix(fill, SURFACE.fg, 0.2), // links, mentions, the unread line
    '--accent-subtle': mix(fill, SURFACE.app, 0.82), // soft fills, text selection
    '--accent-line': mix(fill, SURFACE.app, 0.5),
    '--focus': fill,
  };
}

// The rules of tokens.test.ts for the accent roles: text 4.5, UI parts 3. Text on the fill is whatever --on-accent is.
const RULES: [string, number, (r: Record<string, string>, s: typeof SURFACE) => [string, string]][] = [
  ['text on this colour', 4.5, (r) => [r['--on-accent'], r['--accent']]],
  ['text on its hover', 4.5, (r) => [r['--on-accent'], r['--accent-hover']]],
  ['This colour on the page', 3, (r, s) => [r['--accent'], s.app]],
  ['This colour on the sidebar', 3, (r, s) => [r['--accent'], s.sidebar]],
  ['This colour on a panel', 3, (r, s) => [r['--accent'], s.raised]],
  ['Link text on the page', 4.5, (r, s) => [r['--accent-fg'], s.app]],
  ['Link text on a soft fill', 4.5, (r) => [r['--accent-fg'], r['--accent-subtle']]],
];
export type Check = { ok: true } | { ok: false; what: string; ratio: number; need: number };
/** The first contrast rule `fill` breaks in the scheme, with its ratio. Settings checks a custom colour in dark (the stricter
 * of the two for a light colour); in light the fill is darkened first, so a colour that passes dark paints in both. */
export function check(fill: string, scheme: Scheme = 'dark'): Check {
  const [r, s] = [roles(fill, scheme), scheme === 'light' ? SURFACE_LIGHT : SURFACE];
  for (const [what, need, pair] of RULES) {
    const ratio = contrast(...pair(r, s));
    if (ratio < need) return { ok: false, what: what.startsWith('text') ? `${scheme === 'light' ? 'White' : 'Dark'} ${what}` : what, ratio, need };
  }
  return { ok: true };
}

/** The allowed colour closest to `fill` on the way to white (every rule wants a lighter fill). */
export function nearest(fill: string): string {
  if (check(fill).ok) return fill;
  let [lo, hi] = [0, 1];
  for (let i = 0; i < 12; i++) {
    const t = (lo + hi) / 2;
    if (check(mix(fill, WHITE, t)).ok) hi = t;
    else lo = t;
  }
  return mix(fill, WHITE, hi);
}

export const isHex = (v: string) => /^#[0-9a-f]{6}$/i.test(v);
/** The fill for a stored choice: a preset, an allowed `#rrggbb`, else amber (a refused colour never paints). */
export function fillOf(v: string | null | undefined): string {
  if (v && Object.hasOwn(PRESETS, v)) return PRESETS[v as AccentPreset].hex;
  if (v && isHex(v) && check(v.toLowerCase()).ok) return v.toLowerCase();
  return PRESETS.amber.hex;
}

/** The logo's colour matrix (feColorMatrix values): moves amber to `fill` and keeps every grey (the white letters, the black
 * outline, the glow) and the blends between them, so the logo's flat colours and edges stay as drawn. Identity for amber. */
export function tint(fill: string): number[] {
  const [a, t] = [rgb(PRESETS.amber.hex), rgb(fill)].map((c) => c.map((x) => x / 255));
  const mean = (a[0] + a[1] + a[2]) / 3;
  const c = a.map((x) => x - mean); // amber off the grey axis: greys give 0, amber gives 1
  const n = c[0] ** 2 + c[1] ** 2 + c[2] ** 2;
  return [0, 1, 2].flatMap((i) => [...c.map((cj, j) => +(i === j) + ((t[i] - a[i]) * cj) / n), 0, 0]).concat(0, 0, 0, 1, 0);
}

/** Paints the accent roles on <html>, and keeps the choice for the sign in page of the next visit. The logo follows through
 * the #logo-tint filter in index.html (set here, before the first paint, so it never shows amber first), the tab icon
 * through favicon.ts (lazy: only once a colour other than amber is in use). */
export function applyAccent(v: string | null | undefined, root: HTMLElement = document.documentElement, store: Storage | null = storage): void {
  const fill = fillOf(v);
  const amber = fill === PRESETS.amber.hex;
  // A style with its own accent (data-own-accent, set by applyStyle) keeps it while the accent is the default amber.
  const own = amber && root.getAttribute?.('data-own-accent') != null;
  const light = root.getAttribute?.('data-scheme') === 'light';
  for (const [k, val] of Object.entries(roles(fill, light ? 'light' : 'dark'))) {
    if (own) root.style.removeProperty(k);
    else root.style.setProperty(k, val);
  }
  root.style.setProperty('--logo-filter', amber ? 'none' : 'url(#logo-tint)'); // amber: the shipped file, untouched
  const doc = root.ownerDocument; // absent in the unit tests
  doc?.getElementById('logo-matrix')?.setAttribute('values', tint(fill).join(' '));
  if (doc && (!amber || tinted)) {
    tinted = true;
    void import('./favicon').then((f) => f.favicon(amber ? null : tint(fill))).catch(() => {}); // a failed chunk load: the tab keeps its icon
  }
  try {
    if (v) store?.setItem(ACCENT_KEY, v);
    else store?.removeItem(ACCENT_KEY);
  } catch {
    // storage blocked: the sign in page shows amber
  }
}

export function loadAccent(store: Storage | null = storage): string | null {
  try {
    return store?.getItem(ACCENT_KEY) ?? null;
  } catch {
    return null;
  }
}
