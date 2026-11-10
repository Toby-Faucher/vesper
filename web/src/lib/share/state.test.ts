import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { WATCH_GRACE_MS } from './watch';
import { resetWatches, shares, syncWatches } from './state.svelte';

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
