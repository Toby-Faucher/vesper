import { describe, expect, it } from 'vitest';
import tokens from '../../styles/tokens.css?raw';
import app from '../../app.css?raw';
import { contrast as ratio } from './accent';
import persona from '../../styles/persona.css?raw';
import { block } from './cssvars';

// Each look to check: the base block is Vesper dark; the light block overrides it. A style adds its own entry below.
const base = block(tokens, ':root');
const LOOKS: Record<string, Map<string, string>> = {
  'vesper dark': base,
  'vesper light': new Map([...base, ...block(tokens, "html[data-scheme='light']")]),
  persona: new Map([...base, ...block(persona, "html[data-style='persona']")]),
};

function resolve(vars: Map<string, string>, name: string, seen = 0): string {
  const v = vars.get(name);
  if (v === undefined) throw new Error(`${name} is not defined`);
  const ref = v.match(/^var\((--[\w-]+)\)$/);
  if (ref && seen < 10) return resolve(vars, ref[1], seen + 1);
  if (!/^#[0-9a-f]{6}$/i.test(v)) throw new Error(`${name} is not a hex colour: ${v}`);
  return v;
}

const slots = [1, 2, 3, 4, 5, 6, 7, 8].map((n) => `--avatar-${n}`);
// The pairs in the plan's "Measured contrast": text needs 4.5, UI parts (rings, swatches) 3.
const TEXT: [string, string][] = [
  ['--fg', '--bg-app'], ['--fg-2', '--bg-app'], ['--fg-2', '--bg-selected'], ['--fg-2', '--line'], ['--fg-3', '--bg-app'], ['--fg-3', '--bg-raised'],
  ['--fg-3', '--bg-sidebar'], ['--on-accent', '--accent'], ['--on-accent', '--accent-hover'], ['--accent-fg', '--bg-app'],
  ['--accent-fg', '--accent-subtle'], ['--on-danger', '--danger-solid'], ['--danger-fg', '--bg-app'], ['--warn-fg', '--bg-app'],
  ['--ok-fg', '--bg-app'], ...['--syn-kw', '--syn-str', '--syn-num', '--syn-ty', '--syn-com', '--fg-3'].map((s): [string, string] => [s, '--bg-element']), ...slots.map((s): [string, string] => ['--on-avatar', s]),
];
const UI: [string, string][] = [
  ['--focus', '--bg-sidebar'], ['--focus', '--bg-app'], ['--focus', '--bg-raised'],
  ...slots.flatMap((s): [string, string][] => [[s, '--bg-app'], [s, '--bg-sidebar']]),
];

describe.each(Object.entries(LOOKS))('%s', (_look, vars) => {
  const contrast = (a: string, b: string) => ratio(resolve(vars, a), resolve(vars, b));

  it('text_pairs_meet_4_5', () => {
    const low = TEXT.map(([f, b]) => [f, b, contrast(f, b)] as const).filter((p) => p[2] < 4.5);
    expect(low).toEqual([]);
  });

  it('ui_pairs_meet_3', () => {
    const low = UI.map(([f, b]) => [f, b, contrast(f, b)] as const).filter((p) => p[2] < 3);
    expect(low).toEqual([]);
  });

  // --fg-3 is borderline on hover rows and fails on selected ones: selected rows use --fg-2 for meta (checked above).
  it.runIf(_look.startsWith('vesper'))('fg_3_passes_on_hover_and_fails_on_selected', () => {
    expect(contrast('--fg-3', '--bg-hover')).toBeGreaterThanOrEqual(4.5);
    expect(contrast('--fg-3', '--bg-selected')).toBeLessThan(4.5);
  });
});

it('measured_values_match_the_plan', () => {
  const contrast = (a: string, b: string) => ratio(resolve(base, a), resolve(base, b));
  expect(contrast('--fg', '--bg-app')).toBeCloseTo(16.4, 0);
  expect(contrast('--on-accent', '--accent')).toBeCloseTo(10.0, 0);
});

it('light_redefines_roles_and_adds_none', () => {
  const light = block(tokens, "html[data-scheme='light']");
  expect([...light.keys()].filter((k) => !base.has(k))).toEqual([]);
});

it('every_role_in_the_theme_exists', () => {
  const theme = app.slice(app.indexOf('@theme'));
  const used = [...theme.matchAll(/var\((--[\w-]+)\)/g)].map((m) => m[1]);
  expect(used.length).toBeGreaterThan(30);
  expect(used.filter((v) => !base.has(v))).toEqual([]);
});
