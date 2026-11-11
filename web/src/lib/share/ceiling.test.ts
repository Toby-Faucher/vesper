import { describe, expect, it } from 'vitest';
import { ceilingKbps, engineOf, suggestKbps } from './ceiling';

describe('ceilingKbps', () => {
  it('follows the Firefox size table', () => {
    expect(ceilingKbps('firefox', 1920, 1080)).toBe(5000);
    expect(ceilingKbps('firefox', 1728, 1080)).toBe(5000);
    expect(ceilingKbps('firefox', 1920, 1200)).toBe(10000);
    expect(ceilingKbps('firefox', 2560, 1440)).toBe(10000);
    expect(ceilingKbps('firefox', 1280, 720)).toBe(5000);
    expect(ceilingKbps('firefox', 1024, 576)).toBe(2500);
    expect(ceilingKbps('firefox', 640, 360)).toBe(2000);
  });
  it('has no engine ceiling elsewhere', () => {
    expect(ceilingKbps('other', 1920, 1080)).toBe(30000);
  });
});

describe('suggestKbps', () => {
  it('is half the upload, rounded to 500, within the limits', () => {
    expect(suggestKbps(20_000, 30_000)).toBe(10_000);
    expect(suggestKbps(500_000, 5_000)).toBe(5_000);
    expect(suggestKbps(500_000, 30_000)).toBe(10_000); // four 10 Mbps shares fit the room cap
    expect(suggestKbps(700, 30_000)).toBe(500);
    expect(suggestKbps(9_300, 30_000)).toBe(4_500);
  });
  it('falls back to 8 Mbps under the ceiling when unmeasured', () => {
    expect(suggestKbps(null, 30_000)).toBe(8_000);
    expect(suggestKbps(null, 5_000)).toBe(5_000);
  });
});

describe('engineOf', () => {
  it('spots Firefox and Zen', () => {
    expect(engineOf('Mozilla/5.0 (X11; Linux x86_64; rv:130.0) Gecko/20100101 Firefox/130.0')).toBe('firefox');
    expect(engineOf('Mozilla/5.0 (X11; Linux x86_64; rv:154.0) Gecko/20100101 Firefox/154.0 Zen/1.0')).toBe('firefox');
  });
  it('calls Chromium browsers other', () => {
    const chrome = 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36';
    expect(engineOf(chrome)).toBe('other');
    expect(engineOf(chrome + ' Edg/130.0.0.0')).toBe('other');
    expect(engineOf(chrome)).toBe('other'); // Brave reports plain Chrome
  });
});
