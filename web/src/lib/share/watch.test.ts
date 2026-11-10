import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WATCH_GRACE_MS, WatchGate, resetWatches, syncWatches, tabCounts, wantsWatch } from './watch';
import { shares } from './state.svelte';

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

describe('syncWatches', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    shares.shown = new Set();
    shares.popped = null;
    shares.tabVisible = true;
  });
  afterEach(() => {
    resetWatches();
    vi.useRealTimers();
  });

  it('a shown tile in a visible tab watches at once, and stops 10 s after it is gone', () => {
    const send = vi.fn();
    shares.shown = new Set(['a']);
    syncWatches(['a'], send);
    syncWatches(['a'], send);
    expect(send.mock.calls).toEqual([['a', true]]);
    shares.shown = new Set();
    syncWatches(['a'], send);
    vi.advanceTimersByTime(WATCH_GRACE_MS - 1);
    expect(send).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    expect(send.mock.calls).toEqual([['a', true], ['a', false]]);
  });
  it('a popped out share keeps watching with no tile', () => {
    const send = vi.fn();
    shares.popped = 'a';
    syncWatches(['a'], send);
    shares.tabVisible = false;
    syncWatches(['a'], send);
    vi.advanceTimersByTime(WATCH_GRACE_MS);
    expect(send.mock.calls).toEqual([['a', true]]);
  });
  it('a share that ended stops being watched after the grace', () => {
    const send = vi.fn();
    shares.shown = new Set(['a']);
    syncWatches(['a'], send);
    syncWatches([], send);
    vi.advanceTimersByTime(WATCH_GRACE_MS);
    expect(send.mock.calls).toEqual([['a', true], ['a', false]]);
  });
  it('after a reset (a new join) it watches again, and a pending stop is dropped', () => {
    const send = vi.fn();
    shares.shown = new Set(['a']);
    syncWatches(['a'], send);
    shares.shown = new Set();
    syncWatches(['a'], send);
    resetWatches();
    vi.advanceTimersByTime(WATCH_GRACE_MS);
    expect(send.mock.calls).toEqual([['a', true]]);
    shares.shown = new Set(['a']);
    syncWatches(['a'], send);
    expect(send.mock.calls).toEqual([['a', true], ['a', true]]);
  });
});

describe('tabCounts', () => {
  it('a visible tab counts', () => {
    expect(tabCounts('visible', true)).toBe(true);
    expect(tabCounts('visible', false)).toBe(true);
  });
  it('a hidden tab counts only where our pop-out is missing (Firefox: its own pop-out is invisible to the page)', () => {
    expect(tabCounts('hidden', true)).toBe(false);
    expect(tabCounts('hidden', false)).toBe(true);
  });
});
