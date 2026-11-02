import { expect, it, vi } from 'vitest';

// The call chunk cannot load (a deploy replaced its hashed file): the join stops with the reload text, closes the two
// AudioContexts it made inside the click, and asks for focus on the error.
vi.mock('./call-stack', () => { throw new Error('404'); });
vi.stubGlobal('localStorage', { getItem: () => null, setItem: () => {}, removeItem: () => {} });
vi.stubGlobal('location', { pathname: '/', search: '', hash: '', protocol: 'http:', host: 'x' });
const made: { closed: boolean }[] = [];
vi.stubGlobal('AudioContext', class {
  closed = false;
  constructor() { made.push(this); }
  resume() { return Promise.resolve(); }
  close() { this.closed = true; return Promise.resolve(); }
});

const { session } = await import('./session.svelte');
const v = await import('./voice.svelte');

it('a call chunk that does not load: "Vesper was updated. Reload the page to join.", contexts closed', async () => {
  session.sig = { send() {}, leave() {} } as never;
  session.online = true;
  expect(await v.join(1)).toBe(true);
  expect(v.voice.error).toBe('Vesper was updated. Reload the page to join.');
  expect(v.voice.joining).toBe(false);
  expect(made.map((c) => c.closed)).toEqual([true, true]);
});
