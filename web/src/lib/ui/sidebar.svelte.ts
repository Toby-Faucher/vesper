import { tick } from 'svelte';
// Whether the sidebar is hidden, kept in localStorage['vesper.sidebar']. The shell and the page headers read it.
const KEY = 'vesper.sidebar';
const storage = (() => { try { return localStorage; } catch { return null; } })();

export const sidebar = $state({ collapsed: storage?.getItem(KEY) === 'collapsed' });

export function setCollapsed(on: boolean): void {
  sidebar.collapsed = on;
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
    if (!a || a === document.body || !a.isConnected || !a.checkVisibility())
      document.querySelector<HTMLElement>('[aria-controls="sidebar"]:not([hidden] *)')?.focus();
  });
}
