import { expect, it } from 'vitest';
import tokens from '../../styles/tokens.css?raw';
import { ACCENT_KEY, applyAccent, check, contrast, fillOf, loadAccent, nearest, PRESETS, roles, SURFACE, tint } from './accent';
import { recolour } from './favicon';

const token = (name: string) => tokens.match(new RegExp(`${name}:\\s*(#[0-9a-f]{6})`))![1];

it('surfaces_and_amber_match_tokens_css', () => {
  expect(SURFACE).toEqual({ app: token('--gray-1'), sidebar: token('--gray-2'), raised: token('--gray-3'), fg: token('--gray-12') });
  expect(PRESETS.amber.hex).toBe(token('--amber-9'));
});

it('every_preset_passes_the_contrast_rules', () => {
  for (const [key, p] of Object.entries(PRESETS)) {
    const r = roles(p.hex);
    expect(check(p.hex), key).toEqual({ ok: true });
    // The pairs spelled out, so a change to check() cannot quietly drop one.
    expect(contrast(SURFACE.app, r['--accent']), key).toBeGreaterThanOrEqual(4.5);
    expect(contrast(SURFACE.app, r['--accent-hover']), key).toBeGreaterThanOrEqual(4.5);
    for (const s of [SURFACE.app, SURFACE.sidebar, SURFACE.raised]) expect(contrast(r['--focus'], s), key).toBeGreaterThanOrEqual(3);
    expect(contrast(r['--accent-fg'], SURFACE.app), key).toBeGreaterThanOrEqual(4.5);
    expect(contrast(r['--accent-fg'], r['--accent-subtle']), key).toBeGreaterThanOrEqual(4.5);
  }
  expect(new Set(Object.values(PRESETS).map((p) => p.hex)).size).toBe(8);
});

it('derived_roles_stay_near_the_hand_tuned_amber', () => {
  const r = roles(PRESETS.amber.hex);
  expect(Object.keys(r).sort()).toEqual(['--accent', '--accent-fg', '--accent-hover', '--accent-line', '--accent-subtle', '--focus']);
  // Within 1 of tokens.css's contrast, so amber looks about the same before and after the app paints it.
  const near = (a: string, b: string) => Math.abs(contrast(a, SURFACE.app) - contrast(b, SURFACE.app));
  expect(near(r['--accent-fg'], token('--amber-11'))).toBeLessThan(1);
  expect(near(r['--accent-hover'], token('--amber-10'))).toBeLessThan(1);
  expect(near(r['--accent-subtle'], token('--amber-3'))).toBeLessThan(1);
});

it('a_dark_custom_colour_is_refused_with_its_ratio_and_nearest_passes', () => {
  const c = check('#3050a0');
  expect(c).toMatchObject({ ok: false, what: 'Dark text on this colour', need: 4.5 });
  expect(c.ok || c.ratio).toBeCloseTo(contrast(SURFACE.app, '#3050a0'), 5);
  const n = nearest('#3050a0');
  expect(check(n)).toEqual({ ok: true });
  expect(n).not.toBe('#ffffff'); // moved only as far as needed
  expect(nearest('#a0c0ff')).toBe('#a0c0ff');
});

it('fill_of_falls_back_to_amber', () => {
  expect(fillOf(null)).toBe(PRESETS.amber.hex);
  expect(fillOf('teal')).toBe(PRESETS.teal.hex);
  expect(fillOf('#A0C0FF')).toBe('#a0c0ff');
  for (const bad of ['#3050a0', 'red', 'toString', '#zzzzzz', '']) expect(fillOf(bad), bad).toBe(PRESETS.amber.hex);
});

it('apply_sets_the_roles_and_caches_the_choice', () => {
  const props: Record<string, string> = {};
  const root = { style: { setProperty: (k: string, v: string) => (props[k] = v) } } as unknown as HTMLElement;
  const kept = new Map<string, string>();
  const store = { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => kept.set(k, v), removeItem: (k: string) => kept.delete(k) } as unknown as Storage;
  applyAccent('sky', root, store);
  expect(props).toEqual({ ...roles(PRESETS.sky.hex), '--logo-filter': 'url(#logo-tint)' });
  expect(loadAccent(store)).toBe('sky');
  expect(kept.get(ACCENT_KEY)).toBe('sky');
  applyAccent(null, root, store);
  expect(props['--accent']).toBe(PRESETS.amber.hex);
  expect(props['--logo-filter']).toBe('none'); // the shipped logo, unfiltered
  expect(loadAccent(store)).toBeNull();
  expect(loadAccent({ getItem: () => { throw new Error('blocked'); } } as unknown as Storage)).toBeNull();
});

// `m` on one `#rrggbb`, through the same code as the favicon.
const through = (m: number[], hex: string) => {
  const px = new Uint8ClampedArray([...[1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)), 255]);
  recolour(px, m);
  return '#' + [...px.slice(0, 3)].map((v) => v.toString(16).padStart(2, '0')).join('');
};
// The most common pixels of logo.webp and icon-192.png (decoded in Chromium for #69); update them with a new logo.
const LOGO = { disc: '#fdb91c', ring: '#b54601', white: '#f3f3f3', black: '#0d0d0d' };

it('the_logo_tint_is_the_identity_for_amber_and_keeps_greys', () => {
  expect(tint(PRESETS.amber.hex)).toEqual([1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0]);
  for (const p of Object.values(PRESETS)) {
    const m = tint(p.hex);
    expect(through(m, PRESETS.amber.hex), p.name).toBe(p.hex); // amber lands on the accent exactly
    for (const g of [LOGO.white, LOGO.black, '#808080', '#ffffff', '#000000']) expect(through(m, g), `${p.name} ${g}`).toBe(g);
  }
});

it('the_tinted_logo_disc_keeps_3_to_1_on_every_surface', () => {
  // Every preset and a grid of the custom colours Settings allows.
  const steps = [...Array(8).keys()].map((i) => (0x40 + i * 0x1b).toString(16).padStart(2, '0'));
  const fills = [...Object.values(PRESETS).map((p) => p.hex), ...steps.flatMap((r) => steps.flatMap((g) => steps.map((b) => `#${r}${g}${b}`)))].filter((f) => check(f).ok);
  expect(fills.length).toBeGreaterThan(100);
  for (const f of fills) {
    const disc = through(tint(f), LOGO.disc);
    for (const s of [SURFACE.app, SURFACE.sidebar, SURFACE.raised]) expect(contrast(disc, s), f).toBeGreaterThanOrEqual(3);
  }
});
