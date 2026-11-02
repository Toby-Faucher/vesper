import { tick } from 'svelte';
import { slide } from './motion';
// Whether the sidebar is hidden, kept in localStorage['vesper.sidebar']. The shell and the page headers read it.
const KEY = 'vesper.sidebar';
const storage = (() => { try { return localStorage; } catch { return null; } })();

// `sliding`: a hide is under way; the shell keeps the sidebar shown (fixed, inert) until the slide ends. The saved state at
// boot is read here, not set through setCollapsed(), so it never slides.
export const sidebar = $state({ collapsed: storage?.getItem(KEY) === 'collapsed', sliding: false });
let n = 0; // the latest toggle: an older slide that ends late does not hide the sidebar

export function setCollapsed(on: boolean): void {
  sidebar.collapsed = on;
  sidebar.sliding = on;
  const at = ++n;
  try {
    if (on) storage?.setItem(KEY, 'collapsed');
    else storage?.removeItem(KEY);
  } catch {
    // storage full or blocked: the choice lasts until reload
  }
  // The toggle that was pressed is gone with the sidebar (or with the header button): give focus to the one now shown,
  // unless focus is somewhere else on purpose (Ctrl+\ from the composer keeps it there).
  void tick().then(() => {
    const a = document.activeElement;
    if (!a || a === document.body || !a.isConnected || !a.checkVisibility() || a.closest('[inert]'))
      document.querySelector<HTMLElement>('[aria-controls="sidebar"]:not([hidden] *, [inert] *)')?.focus();
    const side = document.getElementById('sidebar');
    const main = document.getElementById('main');
    const done = () => void (at === n && (sidebar.sliding = false));
    if (side && main) void slide(side, main, !on).then(done);
    else done();
  });
}
