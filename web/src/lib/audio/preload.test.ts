import { describe, it, expect, vi, beforeEach } from 'vitest';
import { preloadDenoiserAssets, resetPreload } from './preload';

const URLS = ['/dsp/vesper_dsp.js', '/dsp/vesper_dsp_bg.wasm', '/dsp/model.tar.gz'];

function nav(saveData?: boolean) {
  vi.stubGlobal('navigator', { connection: saveData === undefined ? undefined : { saveData } });
}

describe('preloadDenoiserAssets', () => {
  beforeEach(() => {
    resetPreload();
    nav();
  });

  it('fetches all three assets with default cache and same-origin credentials', async () => {
    const f = vi.fn().mockResolvedValue(new Response('x'));
    await preloadDenoiserAssets(f);
    expect(f.mock.calls.map((c) => c[0])).toEqual(URLS);
    for (const c of f.mock.calls) expect(c[1]).toEqual({ cache: 'default', credentials: 'same-origin' });
  });

  it('never rejects when fetch rejects or throws', async () => {
    await expect(preloadDenoiserAssets(vi.fn().mockRejectedValue(new Error('offline')))).resolves.toBeUndefined();
    resetPreload();
    await expect(preloadDenoiserAssets(() => { throw new Error('sync'); })).resolves.toBeUndefined();
  });

  it('does nothing when saveData is on', async () => {
    nav(true);
    const f = vi.fn();
    await preloadDenoiserAssets(f);
    expect(f).not.toHaveBeenCalled();
  });

  it('is idempotent', async () => {
    const f = vi.fn().mockResolvedValue(new Response('x'));
    await preloadDenoiserAssets(f);
    await preloadDenoiserAssets(f);
    expect(f).toHaveBeenCalledTimes(3);
  });
});
