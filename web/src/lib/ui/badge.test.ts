import { describe, expect, it, vi } from 'vitest';
import { badge } from './badge';

const u = (room_id: number, mentions: number) => ({ room_id, last_read_id: 0, unread: mentions + 1, mentions });

describe('badge', () => {
  it('shows the sum of unread mentions, clears at 0, and sends only changes', () => {
    const nav = { setAppBadge: vi.fn(async () => {}), clearAppBadge: vi.fn(async () => {}) };
    badge({ 1: u(1, 2), 2: u(2, 1) }, nav);
    badge({ 1: u(1, 2), 2: u(2, 1) }, nav);
    expect(nav.setAppBadge.mock.calls).toEqual([[3]]);
    badge({ 1: u(1, 0) }, nav);
    expect(nav.clearAppBadge).toHaveBeenCalledTimes(1);
  });
  it('does nothing without the API (Firefox) and ignores a refusal', async () => {
    expect(() => badge({ 1: u(1, 4) }, {})).not.toThrow();
    const nav = { setAppBadge: vi.fn(() => Promise.reject(new Error('not installed'))) };
    badge({ 1: u(1, 5) }, nav);
    expect(nav.setAppBadge).toHaveBeenCalledWith(5);
    await Promise.resolve(); // no unhandled rejection
  });
});
