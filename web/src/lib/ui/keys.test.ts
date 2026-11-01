import { describe, expect, it } from 'vitest';
import type { Unread } from '../gen/Unread';
import { SHORTCUTS, isTyping, keysFor, nextUnread, shortcut, stepRoom, type Def } from './keys';

const un = (room_id: number, unread: number): Unread => ({ room_id, last_read_id: 0, unread, mentions: 0 });

type Ev = Parameters<typeof shortcut>[0];
const ev = (key: string, mods: Partial<Ev> = {}): Ev => ({
  key, code: key === '\\' ? 'Backslash' : key.length === 1 ? `Key${key.toUpperCase()}` : key,
  ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...mods,
});
const linux = { typing: false, mac: false, capturingPtt: false };
const mac = { ...linux, mac: true };
const typing = { ...linux, typing: true };
// The event that presses `d` on this platform.
const press = (d: Def, isMac: boolean) =>
  ev(d.shift && d.key.length === 1 ? d.key.toUpperCase() : d.key, {
    ctrlKey: !!d.ctrl || (!!d.mod && !isMac), metaKey: !!d.mod && isMac, shiftKey: !!d.shift, altKey: !!d.alt,
  });

describe('shortcut', () => {
  it('maps each key', () => {
    expect(shortcut(ev('k', { ctrlKey: true }), linux)).toBe('switcher');
    expect(shortcut(ev('M', { ctrlKey: true, shiftKey: true }), linux)).toBe('mute');
    expect(shortcut(ev('D', { ctrlKey: true, shiftKey: true }), linux)).toBe('deafen');
    expect(shortcut(ev('F', { ctrlKey: true, shiftKey: true }), linux)).toBe('search');
    expect(shortcut(ev('ArrowUp', { altKey: true }), linux)).toBe('prev-room');
    expect(shortcut(ev('ArrowDown', { altKey: true }), linux)).toBe('next-room');
    expect(shortcut(ev('ArrowDown', { altKey: true, shiftKey: true }), linux)).toBe('next-unread');
    expect(shortcut(ev('\\', { ctrlKey: true }), linux)).toBe('sidebar');
    expect(shortcut(ev('Escape'), linux)).toBe('escape');
  });

  it('uses Cmd on a Mac and Ctrl elsewhere; the sidebar is Ctrl on both', () => {
    expect(shortcut(ev('k', { metaKey: true }), mac)).toBe('switcher');
    expect(shortcut(ev('k', { ctrlKey: true }), mac)).toBe(null);
    expect(shortcut(ev('k', { metaKey: true }), linux)).toBe(null);
    expect(shortcut(ev('M', { metaKey: true, shiftKey: true }), mac)).toBe('mute');
    expect(shortcut(ev('\\', { ctrlKey: true }), mac)).toBe('sidebar');
    expect(shortcut(ev('\\', { metaKey: true }), mac)).toBe(null);
  });

  it('needs the exact modifiers (AltGr is Ctrl plus Alt, extra Shift is another key)', () => {
    expect(shortcut(ev('k', { ctrlKey: true, altKey: true }), linux)).toBe(null);
    expect(shortcut(ev('K', { ctrlKey: true, shiftKey: true }), linux)).toBe(null);
    expect(shortcut(ev('m', { ctrlKey: true }), linux)).toBe(null);
    expect(shortcut(ev('ArrowUp', { ctrlKey: true }), linux)).toBe(null);
    expect(shortcut(ev('Escape', { shiftKey: true }), linux)).toBe(null);
  });

  it('a key without a modifier is never a shortcut, apart from Escape (push to talk is a single key)', () => {
    for (const k of [' ', 'f', 'k', 'm', 'd', 'ArrowUp', 'ArrowDown', '\\', 'Enter', 'Tab', 'F1', 'Control', 'Alt'])
      for (const ctx of [linux, mac]) expect(shortcut(ev(k), ctx)).toBe(null);
    for (const k of ['F', 'K', 'M']) expect(shortcut(ev(k, { shiftKey: true }), linux)).toBe(null);
    for (const d of SHORTCUTS) if (d.s !== 'escape') expect(d.mod || d.ctrl || d.alt).toBeTruthy();
  });

  it('nothing while Settings captures the push-to-talk key', () => {
    const cap = { ...linux, capturingPtt: true };
    for (const d of SHORTCUTS) expect(shortcut(press(d, false), cap)).toBe(null);
  });

  it('while typing: modifier shortcuts and Alt+Up/Down work; Escape and Alt+Shift+Down do not', () => {
    expect(shortcut(ev('k', { ctrlKey: true }), typing)).toBe('switcher');
    expect(shortcut(ev('M', { ctrlKey: true, shiftKey: true }), typing)).toBe('mute');
    expect(shortcut(ev('\\', { ctrlKey: true }), typing)).toBe('sidebar');
    expect(shortcut(ev('ArrowUp', { altKey: true }), typing)).toBe('prev-room');
    expect(shortcut(ev('ArrowDown', { altKey: true }), typing)).toBe('next-room');
    expect(shortcut(ev('ArrowDown', { altKey: true, shiftKey: true }), typing)).toBe(null);
    expect(shortcut(ev('Escape'), typing)).toBe(null);
  });

  it('has no two shortcuts on one key, on either platform, and each entry presses itself', () => {
    for (const isMac of [false, true]) {
      const seen = new Set(SHORTCUTS.map((d) => keysFor(d, isMac).join('+')));
      expect(seen.size).toBe(SHORTCUTS.length);
      for (const d of SHORTCUTS) expect(shortcut(press(d, isMac), isMac ? mac : linux)).toBe(d.s);
    }
  });

  it('finds Backslash by position when the layout types something else', () => {
    expect(shortcut({ ...ev('|', { ctrlKey: true }), code: 'Backslash' }, linux)).toBe('sidebar');
  });
});

describe('keysFor', () => {
  it('names the caps per platform', () => {
    const find = (s: string) => SHORTCUTS.find((d) => d.s === s)!;
    expect(keysFor(find('mute'), false)).toEqual(['Ctrl', 'Shift', 'M']);
    expect(keysFor(find('mute'), true)).toEqual(['⌘', '⇧', 'M']);
    expect(keysFor(find('next-unread'), false)).toEqual(['Alt', 'Shift', '↓']);
    expect(keysFor(find('sidebar'), true)).toEqual(['Ctrl', '\\']);
    expect(keysFor(find('escape'), false)).toEqual(['Esc']);
  });
});

describe('isTyping', () => {
  // A stand-in for Element.closest: matches when the element is one of `kinds` (a tag, maybe with a type).
  const el = (tag: string, attrs: Record<string, string> = {}) => ({
    closest(sel: string) {
      const parts = sel.split(/,(?![^(]*\))/);
      const hit = parts.some((p) => {
        const [base, not] = p.split(':not(');
        if (base === '[contenteditable]') return 'contenteditable' in attrs && attrs.contenteditable !== 'false';
        if (base !== tag) return false;
        return !not?.includes(`[type=${attrs.type}]`);
      });
      return hit ? this : null;
    },
  }) as unknown as Element;

  it('text fields count, buttons and sliders do not', () => {
    expect(isTyping(el('input'))).toBe(true);
    expect(isTyping(el('input', { type: 'text' }))).toBe(true);
    expect(isTyping(el('input', { type: 'search' }))).toBe(true);
    expect(isTyping(el('textarea'))).toBe(true);
    expect(isTyping(el('select'))).toBe(true);
    expect(isTyping(el('div', { contenteditable: 'true' }))).toBe(true);
    expect(isTyping(el('div', { contenteditable: 'false' }))).toBe(false);
    expect(isTyping(el('input', { type: 'range' }))).toBe(false);
    expect(isTyping(el('input', { type: 'checkbox' }))).toBe(false);
    expect(isTyping(el('button'))).toBe(false);
    expect(isTyping(null)).toBe(false);
  });
});

describe('room steps', () => {
  const order = [5, 2, 9];
  it('steps without wrapping', () => {
    expect(stepRoom(order, 2, 1)).toBe(9);
    expect(stepRoom(order, 2, -1)).toBe(5);
    expect(stepRoom(order, 9, 1)).toBe(null);
    expect(stepRoom(order, 5, -1)).toBe(null);
    expect(stepRoom(order, null, 1)).toBe(5);
    expect(stepRoom(order, null, -1)).toBe(9);
    expect(stepRoom([], null, 1)).toBe(null);
  });
  it('next unread wraps and skips the current room', () => {
    expect(nextUnread(order, 2, { 5: un(5, 1) })).toBe(5);
    expect(nextUnread(order, 5, { 5: un(5, 1), 9: un(9, 1) })).toBe(9);
    expect(nextUnread(order, 5, { 5: un(5, 1) })).toBe(null);
    expect(nextUnread(order, null, { 9: un(9, 2) })).toBe(9);
    expect(nextUnread(order, 2, { 9: un(9, 0) })).toBe(null);
  });
});
