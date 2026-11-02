import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { tint } from './accent';

// favicon() with fetch and the canvas APIs stubbed: each draw makes blob:N, and a draw can be held open or made to fail.
const link = { href: '/favicon.ico' };
let made = 0;
let revoked: string[] = [];
let fail = false;
let gate: Promise<void> = Promise.resolve();
let closed = 0;

beforeEach(() => {
  vi.resetModules();
  [link.href, made, revoked, fail, gate, closed] = ['/favicon.ico', 0, [], false, Promise.resolve(), 0];
  vi.stubGlobal('document', { querySelector: () => link });
  vi.stubGlobal('fetch', async () => {
    const broken = fail; // decided when the draw starts
    await gate;
    return new Response(new Blob(), { status: broken ? 404 : 200 });
  });
  vi.stubGlobal('createImageBitmap', async () => ({ close: () => closed++ }));
  vi.stubGlobal('OffscreenCanvas', class {
    getContext() {
      const data = new Uint8ClampedArray(32 * 32 * 4);
      return { drawImage() {}, getImageData: () => ({ data }), putImageData() {} };
    }
    convertToBlob = async () => new Blob();
  });
  vi.spyOn(URL, 'createObjectURL').mockImplementation(() => `blob:${++made}`);
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation((u) => void revoked.push(u));
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it('replaces_and_revokes_and_amber_puts_the_file_back', async () => {
  const { favicon } = await import('./favicon');
  await favicon(tint('#4db9ee'));
  expect(link.href).toBe('blob:1');
  await favicon(tint('#2fc4b2'));
  expect([link.href, revoked]).toEqual(['blob:2', ['blob:1']]);
  await favicon(null);
  expect([link.href, revoked]).toEqual(['/favicon.ico', ['blob:1', 'blob:2']]);
  expect(closed).toBe(2); // every ImageBitmap closed
});

it('quick_changes_draw_only_the_latest', async () => {
  const { favicon } = await import('./favicon');
  let open!: () => void;
  gate = new Promise((r) => (open = r));
  const first = favicon(tint('#4db9ee'));
  void favicon(tint('#2fc4b2'));
  void favicon(tint('#a6cf4c'));
  open();
  await first; // the loop drains the queue before it returns
  expect(made).toBe(2); // the first, then only the last wish
  expect(link.href).toBe('blob:2');
  expect(revoked).toEqual(['blob:1']);
});

it('a_failed_draw_falls_back_to_the_file_and_keeps_the_queue', async () => {
  const { favicon } = await import('./favicon');
  await favicon(tint('#4db9ee'));
  fail = true;
  await expect(favicon(tint('#2fc4b2'))).resolves.toBeUndefined();
  expect([link.href, revoked]).toEqual(['/favicon.ico', ['blob:1']]);
  let open!: () => void;
  gate = new Promise((r) => (open = r));
  const failing = favicon(tint('#2fc4b2'));
  void favicon(tint('#a6cf4c')); // queued while the failing draw runs
  fail = false;
  open();
  await failing;
  expect(link.href).toBe('blob:2'); // the queued wish was drawn after the failure
});
