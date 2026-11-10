import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WatchGate, wantsWatch } from './watch';

describe('wantsWatch', () => {
  it('is true when popped out', () => {
    expect(wantsWatch({ inPanel: false, tabVisible: false, poppedOut: true })).toBe(true);
  });
  it('is true in the panel with a visible tab', () => {
    expect(wantsWatch({ inPanel: true, tabVisible: true, poppedOut: false })).toBe(true);
  });
  it('is false otherwise', () => {
    expect(wantsWatch({ inPanel: true, tabVisible: false, poppedOut: false })).toBe(false);
    expect(wantsWatch({ inPanel: false, tabVisible: true, poppedOut: false })).toBe(false);
  });
});

describe('WatchGate', () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('sends true once', () => {
    const send = vi.fn();
    const g = new WatchGate(send);
    g.set(true);
    g.set(true);
    expect(send.mock.calls).toEqual([[true]]);
  });
  it('waits 10 s before sending false', () => {
    const send = vi.fn();
    const g = new WatchGate(send);
    g.set(true);
    g.set(false);
    vi.advanceTimersByTime(9_000);
    expect(send).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1_000);
    expect(send.mock.calls).toEqual([[true], [false]]);
  });
  it('sends nothing more when wanted again inside the grace', () => {
    const send = vi.fn();
    const g = new WatchGate(send);
    g.set(true);
    g.set(false);
    vi.advanceTimersByTime(5_000);
    g.set(true);
    vi.advanceTimersByTime(30_000);
    expect(send.mock.calls).toEqual([[true]]);
  });
  it('dispose cancels the timer and sends nothing', () => {
    const send = vi.fn();
    const g = new WatchGate(send);
    g.set(true);
    g.set(false);
    g.dispose();
    vi.advanceTimersByTime(30_000);
    expect(send.mock.calls).toEqual([[true]]);
  });
});
