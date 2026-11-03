// The one switch point for the look: data-style on <html>, set before mount. Components never read it.
// `scheme`: both = the Color scheme choice applies; dark or light = the style is that by nature and the choice is ignored.
// `own`: the style brings its own accent, used while the account accent is the default amber.
// `load`: the style's CSS, a lazy chunk (Vesper is in the first load, so it has none).
export type StyleDef = { id: string; name: string; blurb: string; scheme: 'both' | 'dark' | 'light'; own?: boolean; load?: () => Promise<unknown> };
export const STYLES = [{ id: 'vesper', name: 'Vesper', blurb: 'The default look, dark or light.', scheme: 'both' }] as const satisfies readonly StyleDef[];
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
  if ((STYLES.find((s) => s.id === id) as StyleDef | undefined)?.own) root.setAttribute('data-own-accent', '');
  else root.removeAttribute('data-own-accent');
}

/** Loads the style's CSS, then applies it, so the page never shows a style without its tokens. A failed load keeps the old look. */
export async function useStyle(id: StyleId, root: HTMLElement = document.documentElement): Promise<void> {
  try {
    await (STYLES.find((s) => s.id === id) as StyleDef | undefined)?.load?.();
  } catch {
    return;
  }
  applyStyle(id, root);
}
