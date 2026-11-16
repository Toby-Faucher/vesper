import { describe, expect, it } from 'vitest';
import { DOWNLOADS, PORTABLE, desktopOs, inviteLink } from './desktop';

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
    for (const d of [...Object.values(DOWNLOADS), PORTABLE]) expect(d.url.endsWith(`/latest/download/${d.file}`)).toBe(true);
    expect(DOWNLOADS.windows.file).toBe('vesper-desktop-windows-x64-setup.exe');
    expect(PORTABLE.file).toBe('vesper-desktop-windows-x64.zip');
  });
  it('makes the desktop invite link with this server in it', () => {
    const code = 'ab'.repeat(32);
    expect(inviteLink(code, 'https://v.example:8443')).toBe(`vesper://invite/${code}?server=https%3A%2F%2Fv.example%3A8443`);
  });
});
