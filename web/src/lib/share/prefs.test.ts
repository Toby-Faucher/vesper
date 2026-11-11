import { afterEach, describe, expect, it, vi } from 'vitest';

function fakeStorage(init: Record<string, string> = {}) {
  const m = new Map(Object.entries(init));
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
  };
}

async function load(storage: unknown) {
  vi.resetModules();
  vi.stubGlobal('localStorage', storage);
  return import('./prefs');
}

afterEach(() => vi.unstubAllGlobals());

const DEFAULT = { fps: 60, kbps: null, prefer: 'sharp' };

describe('share settings', () => {
  it('defaults when empty', async () => {
    const p = await load(fakeStorage());
    expect(p.loadShareSettings()).toEqual(DEFAULT);
    expect(p.loadLayout()).toBe('grid');
  });
  it('defaults on corrupt JSON', async () => {
    const p = await load(fakeStorage({ 'vesper.share': '{nope', 'vesper.shareLayout': 'wat' }));
    expect(p.loadShareSettings()).toEqual(DEFAULT);
    expect(p.loadLayout()).toBe('grid');
  });
  it('defaults out of range values', async () => {
    const p = await load(fakeStorage({ 'vesper.share': JSON.stringify({ fps: 45, kbps: 99, prefer: 'x' }) }));
    expect(p.loadShareSettings()).toEqual(DEFAULT);
  });
  it('round-trips', async () => {
    const p = await load(fakeStorage());
    p.saveShareSettings({ fps: 30, kbps: 4000, prefer: 'smooth' });
    expect(p.loadShareSettings()).toEqual({ fps: 30, kbps: 4000, prefer: 'smooth' });
    p.saveLayout('dock');
    expect(p.loadLayout()).toBe('dock');
  });
  it('survives storage that throws', async () => {
    const boom = () => { throw new Error('blocked'); };
    const p = await load({ getItem: boom, setItem: boom });
    expect(() => p.saveShareSettings(DEFAULT as never)).not.toThrow();
    expect(() => p.saveLayout('focus')).not.toThrow();
    expect(p.loadShareSettings()).toEqual(DEFAULT);
    expect(p.loadLayout()).toBe('grid');
  });
});
