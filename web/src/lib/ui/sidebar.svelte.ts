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
}
