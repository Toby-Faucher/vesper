import { expect, it } from 'vitest';
import tokens from '../../styles/tokens.css?raw';
import { ACCENT_KEY, applyAccent, check, contrast, fillOf, loadAccent, nearest, PRESETS, roles, SURFACE } from './accent';

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
  expect(props).toEqual(roles(PRESETS.sky.hex));
  expect(loadAccent(store)).toBe('sky');
  expect(kept.get(ACCENT_KEY)).toBe('sky');
  applyAccent(null, root, store);
  expect(props['--accent']).toBe(PRESETS.amber.hex);
  expect(loadAccent(store)).toBeNull();
  expect(loadAccent({ getItem: () => { throw new Error('blocked'); } } as unknown as Storage)).toBeNull();
});
