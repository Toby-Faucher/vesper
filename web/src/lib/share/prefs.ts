export type Prefer = 'sharp' | 'smooth';
export type Layout = 'grid' | 'focus' | 'dock';
/** kbps null means use the suggestion. */
export type ShareSettings = { fps: 30 | 60; kbps: number | null; prefer: Prefer };

export const SHARE_KEY = 'vesper.share', LAYOUT_KEY = 'vesper.shareLayout';

const storage = (() => { try { return localStorage; } catch { return null; } })();
const DEFAULT: ShareSettings = { fps: 60, kbps: null, prefer: 'sharp' };

export function loadShareSettings(): ShareSettings {
  try {
    const v = JSON.parse(storage?.getItem(SHARE_KEY) ?? 'null');
    const kbps = v?.kbps;
    if ((v?.fps !== 30 && v?.fps !== 60) || (v.prefer !== 'sharp' && v.prefer !== 'smooth')) return { ...DEFAULT };
    if (kbps !== null && !(typeof kbps === 'number' && kbps >= 500 && kbps <= 30_000)) return { ...DEFAULT };
    return { fps: v.fps, kbps, prefer: v.prefer };
  } catch {
    return { ...DEFAULT };
  }
}
export function saveShareSettings(s: ShareSettings): void {
  try { storage?.setItem(SHARE_KEY, JSON.stringify(s)); } catch { /* storage blocked: the setting lasts this page */ }
}

export function loadLayout(): Layout {
  try {
    const v = storage?.getItem(LAYOUT_KEY);
    return v === 'focus' || v === 'dock' ? v : 'grid';
  } catch {
    return 'grid';
  }
}
export function saveLayout(l: Layout): void {
  try { storage?.setItem(LAYOUT_KEY, l); } catch { /* storage blocked: the setting lasts this page */ }
}
