import { expect, it } from 'vitest';
import html from '../../../index.html?raw';
import { loadScheme, resolveScheme, SCHEME_KEY } from './scheme';
import { STYLES, type StyleDef } from './style';

const store = (v: string | null) => ({ getItem: (k: string) => (k === SCHEME_KEY ? v : null) }) as Storage;

it('load_scheme_defaults_to_dark', () => {
  expect(loadScheme(null)).toBe('dark');
  expect(loadScheme(store(null))).toBe('dark');
  expect(loadScheme(store('nope'))).toBe('dark');
  expect(loadScheme(store('light'))).toBe('light');
  expect(loadScheme(store('system'))).toBe('system');
  expect(loadScheme({ getItem: () => { throw new Error('blocked'); } } as unknown as Storage)).toBe('dark');
});

it('resolve_follows_the_choice_and_a_fixed_style_wins', () => {
  expect(resolveScheme('dark', 'both', true)).toBe('dark');
  expect(resolveScheme('light', 'both', false)).toBe('light');
  expect(resolveScheme('system', 'both', true)).toBe('light');
  expect(resolveScheme('system', 'both', false)).toBe('dark');
  expect(resolveScheme('light', 'dark', true)).toBe('dark');
  expect(resolveScheme('dark', 'light', false)).toBe('light');
});

it('the_inline_script_in_index_html_knows_the_fixed_styles', () => {
  const map = html.match(/f = \{([^}]*)\}/)![1];
  const fixed = Object.fromEntries([...map.matchAll(/(\w+): '(\w+)'/g)].map((m) => [m[1], m[2]]));
  const expected = Object.fromEntries((STYLES as readonly StyleDef[]).filter((s) => s.scheme !== 'both').map((s) => [s.id, s.scheme]));
  expect(fixed).toEqual(expected);
  expect(html).toContain("'vesper.scheme'");
  expect(html).toContain("'vesper.style'");
});
