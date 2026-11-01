// The one switch point for the look: data-style on <html>, set before mount. Components never read it.
export const STYLES = [{ id: 'vesper', name: 'Vesper dark' }] as const;
export type StyleId = (typeof STYLES)[number]['id'];
export const STYLE_KEY = 'vesper.style';

export function loadStyle(store: Storage | null): StyleId {
  let id: string | null = null;
  try {
    id = store?.getItem(STYLE_KEY) ?? null;
  } catch {
    // storage blocked: use the default
  }
  return STYLES.find((s) => s.id === id)?.id ?? STYLES[0].id;
}

export function applyStyle(id: StyleId, root: HTMLElement = document.documentElement): void {
  root.setAttribute('data-style', id);
}
