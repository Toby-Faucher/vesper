import { beforeEach, describe, expect, it, vi } from 'vitest';
import { badge, resetBadge } from './badge';

const u = (room_id: number, mentions: number) => ({ room_id, last_read_id: 0, unread: mentions + 1, mentions });
const fake = () => ({ setAppBadge: vi.fn(async () => {}), clearAppBadge: vi.fn(async () => {}) });

describe('badge', () => {
  beforeEach(() => resetBadge({}));

  it('clears on the first count after the app opens, even at 0 (the worker may have set a mark)', async () => {
    vi.resetModules();
    const fresh = await import('./badge');
    const nav = fake();
    fresh.badge({}, nav);
    expect(nav.clearAppBadge).toHaveBeenCalledTimes(1);
    expect(nav.setAppBadge).not.toHaveBeenCalled();
  });
  it('shows the sum of unread mentions, clears at 0, and sends only changes', () => {
    const nav = fake();
    badge({ 1: u(1, 2), 2: u(2, 1) }, nav);
    badge({ 1: u(1, 2), 2: u(2, 1) }, nav);
    badge({ 1: u(1, 3), 2: u(2, 1) }, nav);
    expect(nav.setAppBadge.mock.calls).toEqual([[3], [4]]);
    badge({ 1: u(1, 0) }, nav);
    badge({}, nav);
    expect(nav.clearAppBadge).toHaveBeenCalledTimes(1);
  });
  it('sign-out clears even when the count was already 0, and the next account always sends', () => {
    const nav = fake();
    badge({}, nav);
    resetBadge(nav);
    expect(nav.clearAppBadge).toHaveBeenCalledTimes(2);
    badge({}, nav);
    expect(nav.clearAppBadge).toHaveBeenCalledTimes(3);
  });
  it('does nothing without the API (Firefox) and ignores a refusal', async () => {
    expect(() => badge({ 1: u(1, 4) }, {})).not.toThrow();
    const nav = { setAppBadge: vi.fn(() => Promise.reject(new Error('not installed'))) };
    badge({ 1: u(1, 5) }, nav);
    expect(nav.setAppBadge).toHaveBeenCalledWith(5);
    await Promise.resolve(); // no unhandled rejection
  });
});
