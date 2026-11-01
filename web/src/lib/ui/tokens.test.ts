import { expect, it } from 'vitest';
import tokens from '../../styles/tokens.css?raw';
import app from '../../app.css?raw';

// The vesper style block: every `--name: value;` in it.
const block = tokens.slice(tokens.indexOf("[data-style='vesper']"));
const vars = new Map([...block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));

function hex(name: string, seen = 0): string {
  const v = vars.get(name);
  if (v === undefined) throw new Error(`${name} is not defined`);
  const ref = v.match(/^var\((--[\w-]+)\)$/);
  if (ref && seen < 10) return hex(ref[1], seen + 1);
  if (!/^#[0-9a-f]{6}$/i.test(v)) throw new Error(`${name} is not a hex colour: ${v}`);
  return v;
}

const lum = (h: string) => {
  const c = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255).map((x) => (x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
const contrast = (a: string, b: string) => {
  const [x, y] = [lum(hex(a)), lum(hex(b))].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
};

const slots = [1, 2, 3, 4, 5, 6, 7, 8].map((n) => `--avatar-${n}`);
// The pairs in the plan's "Measured contrast": text needs 4.5, UI parts (rings, swatches) 3.
const TEXT: [string, string][] = [
  ['--fg', '--bg-app'], ['--fg-2', '--bg-app'], ['--fg-2', '--bg-selected'], ['--fg-2', '--line'], ['--fg-3', '--bg-app'], ['--fg-3', '--bg-raised'],
  ['--fg-3', '--bg-sidebar'], ['--on-accent', '--accent'], ['--on-accent', '--accent-hover'], ['--accent-fg', '--bg-app'],
  ['--accent-fg', '--accent-subtle'], ['--on-danger', '--danger-solid'], ['--danger-fg', '--bg-app'], ['--warn-fg', '--bg-app'],
  ['--ok-fg', '--bg-app'], ...slots.map((s): [string, string] => ['--on-avatar', s]),
];
const UI: [string, string][] = [
  ['--focus', '--bg-sidebar'], ['--focus', '--bg-app'], ['--focus', '--bg-raised'],
  ...slots.flatMap((s): [string, string][] => [[s, '--gray-1'], [s, '--gray-2']]),
];

it('text_pairs_meet_4_5', () => {
  const low = TEXT.map(([f, b]) => [f, b, contrast(f, b)] as const).filter((p) => p[2] < 4.5);
  expect(low).toEqual([]);
});

it('ui_pairs_meet_3', () => {
  const low = UI.map(([f, b]) => [f, b, contrast(f, b)] as const).filter((p) => p[2] < 3);
  expect(low).toEqual([]);
});

// --fg-3 is borderline on hover rows and fails on selected ones: selected rows use --fg-2 for meta (checked above).
it('fg_3_passes_on_hover_and_fails_on_selected', () => {
  expect(contrast('--fg-3', '--bg-hover')).toBeGreaterThanOrEqual(4.5);
  expect(contrast('--fg-3', '--bg-selected')).toBeLessThan(4.5);
});

it('measured_values_match_the_plan', () => {
  expect(contrast('--fg', '--bg-app')).toBeCloseTo(16.4, 0);
  expect(contrast('--on-accent', '--accent')).toBeCloseTo(10.0, 0);
});

it('every_role_in_the_theme_exists', () => {
  const theme = app.slice(app.indexOf('@theme'));
  const used = [...theme.matchAll(/var\((--[\w-]+)\)/g)].map((m) => m[1]);
  expect(used.length).toBeGreaterThan(30);
  expect(used.filter((v) => !vars.has(v))).toEqual([]);
});
