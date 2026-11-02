import swSource from '../../public/sw.js?raw';
import { describe, expect, it, vi } from 'vitest';

// public/sw.js run against a stand-in `self`: its push handler, with and without the Badging API.
function worker(navigator: object) {
  const on: Record<string, (e: unknown) => void> = {};
  const self = {
    addEventListener: (type: string, f: (e: unknown) => void) => (on[type] = f),
    registration: { showNotification: vi.fn(async () => {}) },
    navigator,
    location: { origin: 'http://x' },
  };
  new Function('self', swSource)(self);
  const push = async (payload: object) => {
    const waits: Promise<unknown>[] = [];
    on.push({ data: { json: () => payload }, waitUntil: (p: Promise<unknown>) => waits.push(p) });
    await Promise.all(waits);
  };
  return { self, push };
}

describe('sw.js badge', () => {
  it('a mention marks the badge without a number; a voice push does not', async () => {
    const nav = { setAppBadge: vi.fn(async () => {}) };
    const { self, push } = worker(nav);
    await push({ kind: 'voice', room_id: 2, title: 'Vesper', body: 'x' });
    expect(nav.setAppBadge).not.toHaveBeenCalled();
    await push({ kind: 'mention', room_id: 2, title: 'Vesper', body: 'x' });
    expect(nav.setAppBadge.mock.calls).toEqual([[]]);
    expect(self.registration.showNotification).toHaveBeenCalledTimes(2);
  });
  it('without the API (Firefox) or with a refusal the notification still shows', async () => {
    for (const nav of [{}, { setAppBadge: vi.fn(() => Promise.reject(new Error('no'))) }]) {
      const { self, push } = worker(nav);
      await push({ kind: 'mention', room_id: 2, title: 'Vesper', body: 'x' });
      expect(self.registration.showNotification).toHaveBeenCalledTimes(1);
    }
  });
});
