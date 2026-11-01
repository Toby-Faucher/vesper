// The keyboard shortcuts: one table, read by the shell's keydown handler, the shortcuts dialog and the switcher's hints.
// Every shortcut but Escape and ? needs a modifier, and those two never fire while typing. Settings refuses a modifier or a
// shortcut's key for push to talk (pttKeyProblem), but an older saved key can be one (ControlLeft, KeyK): so nothing fires
// while push to talk is held, or when the key pressed is its key.

export type Shortcut = 'switcher' | 'help' | 'mute' | 'deafen' | 'prev-room' | 'next-room' | 'next-unread' | 'search' | 'sidebar' | 'escape';
import type { Unread } from '../gen/Unread';

type KeyEvent = Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'shiftKey' | 'altKey'> &
  Partial<Pick<KeyboardEvent, 'getModifierState'>>;

/** `mod` is Cmd on a Mac and Ctrl elsewhere; `ctrl` is Ctrl everywhere. `typing`: it also works in a text field. */
export type Def = { s: Shortcut; key: string; label: string; mod?: true; ctrl?: true; shift?: true; alt?: true; typing?: true };
export const SHORTCUTS: Def[] = [
  { s: 'switcher', mod: true, key: 'k', typing: true, label: 'Switch to a room, a person or an action' },
  { s: 'mute', mod: true, shift: true, key: 'm', typing: true, label: 'Mute or unmute' },
  { s: 'deafen', mod: true, shift: true, key: 'd', typing: true, label: 'Deafen or undeafen' },
  { s: 'search', mod: true, shift: true, key: 'f', typing: true, label: 'Search messages' },
  // Alt+Up/Down move the caret by paragraph in a Mac text field, so in a field they stay the field's; Ctrl+Alt works there.
  { s: 'prev-room', alt: true, key: 'ArrowUp', label: 'Previous room' },
  { s: 'next-room', alt: true, key: 'ArrowDown', label: 'Next room' },
  { s: 'prev-room', ctrl: true, alt: true, key: 'ArrowUp', typing: true, label: 'Previous room, also while typing' },
  { s: 'next-room', ctrl: true, alt: true, key: 'ArrowDown', typing: true, label: 'Next room, also while typing' },
  { s: 'next-unread', alt: true, shift: true, key: 'ArrowDown', label: 'Next unread room' },
  { s: 'sidebar', ctrl: true, key: '\\', typing: true, label: 'Hide or show the sidebar' },
  { s: 'help', shift: true, key: '?', label: 'Show the keyboard shortcuts' },
  { s: 'escape', key: 'Escape', label: 'Close a dialog or a menu' },
];

/** The push-to-talk state the shortcuts step aside for: the key's code (null when push to talk is off) and if it is held. */
export type Ptt = { code: string | null; held: boolean };

/** The shortcut `e` presses, or null. Nothing while Settings waits for a new push-to-talk key, or for push to talk. */
export function shortcut(e: KeyEvent, ctx: { typing: boolean; mac: boolean; capturingPtt: boolean; ptt: Ptt }): Shortcut | null {
  if (ctx.capturingPtt || ctx.ptt.held || (ctx.ptt.code && e.code === ctx.ptt.code)) return null;
  // AltGr types characters (Windows reports it as Ctrl+Alt), so it is never a shortcut. Keys by the character they type:
  // Ctrl+\ is the key that types \, not the key in the US \ position (Ctrl+# on a German layout). Where \ needs AltGr,
  // the sidebar has its button only.
  if (e.getModifierState?.('AltGraph')) return null;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
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
    ...(d.shift && d.key !== '?' ? [mac ? '⇧' : 'Shift'] : []), // ? is Shift+/ on most layouts: the cap says it
    k,
  ];
}
export const keysOf = (s: Shortcut, mac: boolean) => keysFor(SHORTCUTS.find((d) => d.s === s)!, mac);

export const isMac = () => typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.userAgent);

/** True when `el` takes text: a text input, a textarea, a select or an editable element. Push to talk and the shortcuts both use it. */
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

/** Enter without Shift sends (or saves an edit). Not while an IME is composing: Safari reports keyCode 229 for the Enter that ends it. */
export const isSendKey = (e: { key: string; shiftKey: boolean; isComposing: boolean; keyCode: number }) =>
  e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229;

const MODIFIERS = ['Control', 'Alt', 'AltGraph', 'Shift', 'Meta', 'OS', 'Hyper', 'Super', 'Fn'];
// The keys of the table above (k, m, d, f, \, the arrows, ?, Esc), by the character typed or by the key's place.
const SHORTCUT_KEYS = new Set(SHORTCUTS.map((d) => d.key));
const SHORTCUT_CODES = new Set(['KeyK', 'KeyM', 'KeyD', 'KeyF', 'ArrowUp', 'ArrowDown', 'Backslash', 'Slash', 'Escape']);
const MOVE_KEYS = ['Tab', 'Enter', 'Backspace'];
/** Why `e` cannot be the push-to-talk key, or null when it can. A held modifier or a shortcut's key would block the shortcuts. */
export function pttKeyProblem(e: Pick<KeyboardEvent, 'key' | 'code'>): string | null {
  if (MODIFIERS.includes(e.key) || /^(Control|Alt|Shift|Meta|OS)/.test(e.code)) return 'Pick a key that is not Ctrl, Alt, Shift or Meta.';
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  if (SHORTCUT_KEYS.has(key) || SHORTCUT_CODES.has(e.code)) return 'A keyboard shortcut uses that key. Pick another one.';
  if (MOVE_KEYS.includes(e.key) || MOVE_KEYS.includes(e.code) || e.code === 'NumpadEnter') return 'Tab, Enter and Backspace move around the app and type. Pick another one.';
  if (!e.code) return 'That key cannot be read. Pick another one.';
  return null;
}
/** A key code as people read it: KeyQ is Q, Digit1 is 1, BracketLeft is Bracket left, Numpad0 is Numpad 0. */
export const keyName = (code: string) =>
  code.replace(/^(Key|Digit)(?=.)/, '').replace(/(?<=[a-z])(?=[A-Z0-9])/g, ' ').replace(/ (\w)/g, (_, c: string) => ` ${c.toLowerCase()}`);
