// Color scheme: dark, light, or follow the system. Per device (vesper.scheme), default dark so the look people have stays.
// The result is data-scheme="dark|light" on <html>; tokens.css reads only that. index.html sets it before the first paint with
// the same rules (scheme.test.ts keeps the two in step). A style that is dark or light by nature (STYLES[].scheme) ignores the choice.
import { applyAccent, loadAccent, type Scheme } from './accent';
import { STYLES } from './style';

export const SCHEME_KEY = 'vesper.scheme';
export const CHOICES = [
  { id: 'dark', name: 'Dark' },
  { id: 'light', name: 'Light' },
  { id: 'system', name: 'System' },
] as const;
export type Choice = (typeof CHOICES)[number]['id'];
const QUERY = '(prefers-color-scheme: light)';

export function loadScheme(store: Storage | null): Choice {
  let v: string | null = null;
  try {
    v = store?.getItem(SCHEME_KEY) ?? null;
  } catch {
    // storage blocked: dark
  }
  return CHOICES.find((c) => c.id === v)?.id ?? 'dark';
}

/** The scheme to draw: the style's own when it has one, else the choice (System asks the browser). */
export function resolveScheme(choice: Choice, fixed: 'both' | Scheme, systemLight: boolean): Scheme {
  if (fixed !== 'both') return fixed;
  return choice === 'system' ? (systemLight ? 'light' : 'dark') : choice;
}

/** Sets data-scheme for the style now on <html>, the browser's tab colour, and repaints the accent for the scheme. */
export function applyScheme(choice: Choice, root: HTMLElement = document.documentElement): Scheme {
  const fixed = STYLES.find((s) => s.id === root.getAttribute('data-style'))?.scheme ?? 'both';
  const scheme = resolveScheme(choice, fixed, typeof matchMedia === 'function' && matchMedia(QUERY).matches);
  root.setAttribute('data-scheme', scheme);
  applyAccent(loadAccent(), root);
  const doc = root.ownerDocument;
  const colour = doc && getComputedStyle(root).getPropertyValue('--bg-app').trim();
  if (colour) doc.querySelector('meta[name="theme-color"]')?.setAttribute('content', colour);
  return scheme;
}

/** Follows the system while the choice is System. */
export function watchScheme(store: Storage | null): void {
  if (typeof matchMedia === 'function') matchMedia(QUERY).addEventListener('change', () => applyScheme(loadScheme(store)));
}
