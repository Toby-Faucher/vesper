import { expect, it } from 'vitest';
import { applyStyle, loadStyle, STYLE_KEY, STYLES } from './style';

const store = (v: string | null) => ({ getItem: (k: string) => (k === STYLE_KEY ? v : null) }) as Storage;

it('load_style_falls_back_to_vesper', () => {
  expect(loadStyle(null)).toBe('vesper');
  expect(loadStyle(store(null))).toBe('vesper');
  expect(loadStyle(store('no-such-style'))).toBe('vesper');
  expect(loadStyle(store('vesper'))).toBe('vesper');
  const throwing = { getItem: () => { throw new Error('blocked'); } } as unknown as Storage;
  expect(loadStyle(throwing)).toBe('vesper');
  expect(STYLES.map((s) => s.id)[0]).toBe('vesper');
});

it('apply_style_sets_data_style', () => {
  const attrs: Record<string, string> = {};
  const root = { setAttribute: (k: string, v: string) => (attrs[k] = v), removeAttribute: (k: string) => delete attrs[k] } as unknown as HTMLElement;
  applyStyle('vesper', root);
  expect(attrs).toEqual({ 'data-style': 'vesper' });
});
