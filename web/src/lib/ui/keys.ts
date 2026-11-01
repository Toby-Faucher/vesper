// The keyboard shortcuts: one table, read by the shell's keydown handler, the shortcuts dialog and the switcher's hints.
// Every shortcut but Escape needs a modifier, so none can fight push to talk (a single key) or typing.

export type Shortcut = 'switcher' | 'mute' | 'deafen' | 'prev-room' | 'next-room' | 'next-unread' | 'search' | 'sidebar' | 'escape';
import type { Unread } from '../gen/Unread';

type KeyEvent = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'>;

/** `mod` is Cmd on a Mac and Ctrl elsewhere; `ctrl` is Ctrl everywhere. `typing`: it also works in a text field. */
export type Def = { s: Shortcut; key: string; label: string; mod?: true; ctrl?: true; shift?: true; alt?: true; typing?: true };
export const SHORTCUTS: Def[] = [
  { s: 'switcher', mod: true, key: 'k', typing: true, label: 'Switch to a room, a person or an action' },
  { s: 'mute', mod: true, shift: true, key: 'm', typing: true, label: 'Mute or unmute' },
  { s: 'deafen', mod: true, shift: true, key: 'd', typing: true, label: 'Deafen or undeafen' },
  { s: 'search', mod: true, shift: true, key: 'f', typing: true, label: 'Search messages' },
  { s: 'prev-room', alt: true, key: 'ArrowUp', typing: true, label: 'Previous room' },
  { s: 'next-room', alt: true, key: 'ArrowDown', typing: true, label: 'Next room' },
  { s: 'next-unread', alt: true, shift: true, key: 'ArrowDown', label: 'Next unread room' },
  { s: 'sidebar', ctrl: true, key: '\\', typing: true, label: 'Hide or show the sidebar' },
  { s: 'escape', key: 'Escape', label: 'Close a dialog or a menu' },
];

/** The shortcut `e` presses, or null. Nothing while Settings waits for a new push-to-talk key. */
export function shortcut(e: KeyEvent, ctx: { typing: boolean; mac: boolean; capturingPtt: boolean }): Shortcut | null {
  if (ctx.capturingPtt) return null;
  // Backslash by its position: Shift or the layout can change the character. Letters by the character they type.
  const key = e.code === 'Backslash' ? '\\' : e.key.length === 1 ? e.key.toLowerCase() : e.key;
  for (const d of SHORTCUTS) {
    const ctrl = !!d.ctrl || (!!d.mod && !ctx.mac);
    const meta = !!d.mod && ctx.mac;
    if (d.key === key && e.ctrlKey === ctrl && e.metaKey === meta && e.shiftKey === !!d.shift && e.altKey === !!d.alt)
      return ctx.typing && !d.typing ? null : d.s;
  }
  return null;
}

const NAMES: Record<string, string> = { ArrowUp: '↑', ArrowDown: '↓', Escape: 'Esc' };
/** The keys to show for `d`, one string per key cap. */
export function keysFor(d: Def, mac: boolean): string[] {
  const k = NAMES[d.key] ?? d.key.toUpperCase();
  return [
    ...(d.ctrl ? ['Ctrl'] : []),
    ...(d.mod ? [mac ? '⌘' : 'Ctrl'] : []),
    ...(d.alt ? [mac ? '⌥' : 'Alt'] : []),
    ...(d.shift ? [mac ? '⇧' : 'Shift'] : []),
    k,
  ];
}
export const keysOf = (s: Shortcut, mac: boolean) => keysFor(SHORTCUTS.find((d) => d.s === s)!, mac);

export const isMac = () => typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent);

/** True when `el` takes text: a text input, a textarea, a select or an editable element. */
export function isTyping(el: Element | null): boolean {
  return !!el?.closest?.(
    'input:not([type=checkbox],[type=radio],[type=range],[type=button],[type=submit],[type=reset],[type=color],[type=file]),textarea,select,[contenteditable]:not([contenteditable=false])',
  );
}

// Alt+Up/Down and Alt+Shift+Down. Here, not in switcher.ts, so the switcher's code stays out of the first load.
/** The room `dir` steps from `current` in the sidebar order (no wrap), or null at either end. */
export function stepRoom(ids: number[], current: number | null, dir: 1 | -1): number | null {
  const i = current === null ? -1 : ids.indexOf(current);
  if (i < 0) return ids[dir > 0 ? 0 : ids.length - 1] ?? null;
  return ids[i + dir] ?? null;
}

/** The next room with unread messages after `current` in the sidebar order, wrapping, or null when none other has any. */
export function nextUnread(ids: number[], current: number | null, unread: Record<number, Unread>): number | null {
  const i = current === null ? -1 : ids.indexOf(current);
  for (let k = 1; k <= ids.length; k++) {
    const id = ids[(i + k + ids.length) % ids.length];
    if (id !== current && unread[id]?.unread) return id;
  }
  return null;
}
