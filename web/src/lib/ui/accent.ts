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
const WHITE = '#ffffff';
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

/** Every accent role from one fill. Text on the fill and the hover is --on-accent (--gray-1, near-black). */
export function roles(fill: string): Record<string, string> {
  return {
    '--accent': fill,
    '--accent-hover': mix(fill, WHITE, 0.15),
    '--accent-fg': mix(fill, SURFACE.fg, 0.2), // links, mentions, the unread line
    '--accent-subtle': mix(fill, SURFACE.app, 0.82), // soft fills, text selection
    '--accent-line': mix(fill, SURFACE.app, 0.5),
    '--focus': fill,
  };
}

// The rules of tokens.test.ts for the accent roles: text 4.5, UI parts 3.
const RULES: [string, number, (r: Record<string, string>) => [string, string]][] = [
  ['Dark text on this colour', 4.5, (r) => [SURFACE.app, r['--accent']]],
  ['Dark text on its hover', 4.5, (r) => [SURFACE.app, r['--accent-hover']]],
  ['This colour on the page', 3, (r) => [r['--accent'], SURFACE.app]],
  ['This colour on the sidebar', 3, (r) => [r['--accent'], SURFACE.sidebar]],
  ['This colour on a panel', 3, (r) => [r['--accent'], SURFACE.raised]],
  ['Link text on the page', 4.5, (r) => [r['--accent-fg'], SURFACE.app]],
  ['Link text on a soft fill', 4.5, (r) => [r['--accent-fg'], r['--accent-subtle']]],
];
export type Check = { ok: true } | { ok: false; what: string; ratio: number; need: number };
/** The first contrast rule `fill` breaks, with its ratio. */
export function check(fill: string): Check {
  const r = roles(fill);
  for (const [what, need, pair] of RULES) {
    const ratio = contrast(...pair(r));
    if (ratio < need) return { ok: false, what, ratio, need };
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
  for (const [k, val] of Object.entries(roles(fill))) root.style.setProperty(k, val);
  const amber = fill === PRESETS.amber.hex;
  root.style.setProperty('--logo-filter', amber ? 'none' : 'url(#logo-tint)'); // amber: the shipped file, untouched
  const doc = root.ownerDocument; // absent in the unit tests
  doc?.getElementById('logo-matrix')?.setAttribute('values', tint(fill).join(' '));
  if (doc && (!amber || tinted)) {
    tinted = true;
    void import('./favicon').then((f) => f.favicon(amber ? null : tint(fill)));
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
