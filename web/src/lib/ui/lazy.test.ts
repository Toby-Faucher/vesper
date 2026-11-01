import { describe, expect, it } from 'vitest';
import { lazy } from './lazy';

describe('lazy', () => {
  it('loads once and keeps the result', async () => {
    let n = 0;
    const get = lazy(async () => ++n);
    expect(await get()).toBe(1);
    expect(await get()).toBe(1);
    expect(n).toBe(1);
  });

  it('a failed import is not kept: the next call tries again', async () => {
    let n = 0;
    // A dynamic import of a chunk that is not there (offline, or a deploy removed it).
    const get = lazy(() => (++n === 1 ? import(/* @vite-ignore */ `./missing-chunk-${n}.js`) : Promise.resolve('loaded')));
    await expect(get()).rejects.toThrow();
    expect(await get()).toBe('loaded');
    expect(await get()).toBe('loaded');
    expect(n).toBe(2);
  });
});
