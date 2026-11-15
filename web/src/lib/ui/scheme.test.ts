import { expect, it } from 'vitest';
import html from '../../../public/scheme-init.js?raw';
import page from '../../../index.html?raw';
import { loadScheme, resolveScheme, SCHEME_KEY } from './scheme';

const store = (v: string | null) => ({ getItem: (k: string) => (k === SCHEME_KEY ? v : null) }) as Storage;

it('load_scheme_defaults_to_dark', () => {
  expect(loadScheme(null)).toBe('dark');
  expect(loadScheme(store(null))).toBe('dark');
  expect(loadScheme(store('nope'))).toBe('dark');
  expect(loadScheme(store('light'))).toBe('light');
  expect(loadScheme(store('system'))).toBe('system');
  expect(loadScheme({ getItem: () => { throw new Error('blocked'); } } as unknown as Storage)).toBe('dark');
});

it('resolve_follows_the_choice', () => {
  expect(resolveScheme('dark', true)).toBe('dark');
  expect(resolveScheme('light', false)).toBe('light');
  expect(resolveScheme('system', true)).toBe('light');
  expect(resolveScheme('system', false)).toBe('dark');
});

it('the_scheme_script_loaded_by_index_html_reads_the_choice_only', () => {
  expect(html).toContain("'vesper.scheme'");
  expect(html).not.toContain('vesper.style'); // the old Style choice no longer fixes the scheme
  expect(page).toContain('src="/scheme-init.js"'); // a file, not inline: the CSP forbids inline script
});
