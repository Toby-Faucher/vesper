import { describe, expect, it } from 'vitest';
import { DOWNLOADS, desktopOs } from './desktop';

describe('desktopOs', () => {
  it('picks the build for this system', () => {
    expect(desktopOs('Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/130.0')).toBe('windows');
    expect(desktopOs('Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0')).toBe('linux');
  });
  it('offers none where there is no build', () => {
    expect(desktopOs('Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 Safari/605.1.15')).toBeNull();
    expect(desktopOs('Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/130.0 Mobile')).toBeNull();
    expect(desktopOs('Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15')).toBeNull();
  });
  it('links each build by its fixed release name', () => {
    for (const d of Object.values(DOWNLOADS)) expect(d.url.endsWith(`/latest/download/${d.file}`)).toBe(true);
  });
});
