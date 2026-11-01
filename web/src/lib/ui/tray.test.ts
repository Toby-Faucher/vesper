import { describe, expect, it } from 'vitest';
import { ApiFailure } from '../api';
import { MAX_UPLOAD } from '../chat.svelte';
import type { Attachment } from '../gen/Attachment';
import { LIMIT, Tray, uploadError } from './tray.svelte';
import { UI_MESSAGES } from './messages';

// A fake upload: each call waits until the test settles it.
function fake() {
  const calls: { name: string; signal: AbortSignal; ok: (a: Attachment) => void; fail: (e: unknown) => void; progress: (p: number) => void }[] = [];
  const up = (_room: number, f: File, onProgress?: (p: number) => void, signal?: AbortSignal) =>
    new Promise<Attachment>((ok, fail) => calls.push({ name: f.name, signal: signal!, ok, fail, progress: onProgress! }));
  return { calls, up };
}
const file = (name: string, size = 10) => ({ name, size, type: 'image/png' }) as File;
const att = (id: string): Attachment => ({ id, name: id, mime: 'image/png', size: 1, image: true });
const flush = () => new Promise((r) => setTimeout(r, 0));
const states = (t: Tray) => t.items.map((i) => i.state);

describe('Tray', () => {
  it(`runs at most ${LIMIT} uploads at once and starts the next when one ends`, async () => {
    const f = fake();
    const t = new Tray(f.up);
    t.add(1, ['a', 'b', 'c', 'd', 'e'].map((n) => file(n)));
    expect(f.calls.map((c) => c.name)).toEqual(['a', 'b', 'c']);
    expect(states(t)).toEqual(['uploading', 'uploading', 'uploading', 'queued', 'queued']);
    f.calls[1].ok(att('b'));
    await flush();
    expect(f.calls.map((c) => c.name)).toEqual(['a', 'b', 'c', 'd']);
    f.calls[0].fail(new TypeError('network'));
    await flush();
    expect(f.calls.map((c) => c.name)).toEqual(['a', 'b', 'c', 'd', 'e']);
    expect(states(t)).toEqual(['failed', 'done', 'uploading', 'uploading', 'uploading']);
    expect(t.busy).toBe(true);
  });

  it('refuses a file over the limit before any upload', () => {
    const f = fake();
    const t = new Tray(f.up);
    t.add(1, [file('big', MAX_UPLOAD + 1), file('ok', MAX_UPLOAD)]);
    expect(f.calls.map((c) => c.name)).toEqual(['ok']);
    expect(t.items[0]).toMatchObject({ state: 'failed', error: UI_MESSAGES.fileTooLarge, retry: false });
    t.retry(t.items[0].key);
    expect(f.calls.length).toBe(1);
  });

  it('cancel aborts a running upload, removes it and starts a queued one', async () => {
    const f = fake();
    const t = new Tray(f.up);
    t.add(1, ['a', 'b', 'c', 'd'].map((n) => file(n)));
    t.remove(t.items[0].key);
    expect(f.calls[0].signal.aborted).toBe(true);
    f.calls[0].fail(new DOMException('x', 'AbortError'));
    await flush();
    expect(t.items.map((i) => i.file.name)).toEqual(['b', 'c', 'd']);
    expect(f.calls.map((c) => c.name)).toEqual(['a', 'b', 'c', 'd']);
  });

  it('shows progress, retries a failure, and Send takes only the finished files', async () => {
    const f = fake();
    const t = new Tray(f.up);
    t.add(1, [file('a'), file('b')]);
    f.calls[0].progress(0.4);
    expect(t.items[0].progress).toBe(0.4);
    f.calls[0].ok(att('a'));
    f.calls[1].fail(new ApiFailure(507, 'quota_full'));
    await flush();
    expect(t.items[1]).toMatchObject({ state: 'failed', error: UI_MESSAGES.uploadQuota, retry: true });
    expect(t.done.map((i) => i.attachment?.id)).toEqual(['a']);
    t.sent();
    expect(t.items.map((i) => i.file.name)).toEqual(['b']);
    t.retry(t.items[0].key);
    expect(f.calls.map((c) => c.name)).toEqual(['a', 'b', 'b']);
    t.clear();
    expect(f.calls[2].signal.aborted).toBe(true);
    expect(t.items).toEqual([]);
  });

  it('words each failure plainly', () => {
    expect(uploadError(new ApiFailure(413, 'too_large'))).toEqual({ error: UI_MESSAGES.fileTooLarge, retry: false });
    expect(uploadError(new ApiFailure(507, 'quota_full')).error).toBe(UI_MESSAGES.uploadQuota);
    expect(uploadError(new ApiFailure(429, 'rate_limited')).error).toBe(UI_MESSAGES.uploadRate);
    expect(uploadError(new TypeError('network')).error).toBe(UI_MESSAGES.uploadNetwork);
  });
});
