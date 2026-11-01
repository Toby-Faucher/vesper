import type { RoomInfo } from '../gen/RoomInfo';
import type { UserInfo } from '../gen/UserInfo';

// The search box's inline syntax: `from:sam in:general after:2026-10-01 before:2026-10-31 hello`. A value with a space
// goes in quotes (`from:"Sam Smith"`, `in:"game night"`). Anything else, other `word:` tokens included, is text for the
// server, which quotes every term itself, so no input here can become search syntax.

export type Key = 'from' | 'in' | 'after' | 'before';
export type Chip = { key: Key; text: string };
export type Parsed = {
  q: string;
  room_id: number | null;
  author_id: string | null;
  from: number | null;
  to: number | null;
  chips: Chip[];
  unknown: string[];
};
type Token = { start: number; end: number; key: Key | null; value: string; text: string };

const KEYS = new Set<string>(['from', 'in', 'after', 'before']);
// A run of quoted text or of anything but spaces and quotes, glued together: `from:"Sam Smith"`, `"two words"`, `x"y`.
const TOKEN = /(?:"[^"]*"?|[^\s"]+)+/g;
const unquote = (s: string) => s.replace(/"/g, '');

function tokens(input: string): Token[] {
  return [...input.matchAll(TOKEN)].map((m) => {
    const text = m[0];
    const c = text.indexOf(':');
    const key = c > 0 ? text.slice(0, c).toLowerCase() : '';
    const isKey = KEYS.has(key);
    return { start: m.index, end: m.index + text.length, key: isKey ? (key as Key) : null, value: isKey ? unquote(text.slice(c + 1)) : unquote(text), text };
  });
}

/** Local midnight at the start of a YYYY-MM-DD day, or of the day after it; null for anything else. */
function day(v: string, today: Date, next = false): number | null {
  const at = (y: number, m: number, d: number) => new Date(y, m, d + (next ? 1 : 0)).getTime();
  const w = v.toLowerCase();
  if (w === 'today' || w === 'yesterday') return at(today.getFullYear(), today.getMonth(), today.getDate() - (w === 'today' ? 0 : 1));
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v);
  if (!m) return null;
  const [y, mo, d] = [Number(m[1]), Number(m[2]) - 1, Number(m[3])];
  const t = new Date(y, mo, d);
  return t.getFullYear() === y && t.getMonth() === mo && t.getDate() === d ? at(y, mo, d) : null; // 2026-02-30 is not a day
}

const dayText = (ms: number) => new Date(ms).toLocaleDateString([], { day: 'numeric', month: 'short', year: 'numeric' });

/**
 * The query the server gets for `input`. Dates are whole local days, both included: `after:` starts at that day's
 * midnight, `before:` ends at the last ms of that day (the server compares unix ms and knows no time zone). A
 * repeated key: the last one counts. A key with no value yet (`from:` while typing) is left out.
 */
export function parseSearch(input: string, ctx: { users: UserInfo[]; rooms: RoomInfo[]; today: Date }): Parsed {
  const out: Parsed = { q: '', room_id: null, author_id: null, from: null, to: null, chips: [], unknown: [] };
  const text: string[] = [];
  const chip = new Map<Key, string>();
  const bad = new Map<Key, string>();
  for (const t of tokens(input)) {
    if (!t.key) {
      if (t.value) text.push(t.value);
      continue;
    }
    const v = t.value.trim();
    if (!v) continue;
    const low = v.toLowerCase();
    bad.delete(t.key);
    chip.delete(t.key);
    if (t.key === 'from') {
      const u = ctx.users.find((x) => x.username.toLowerCase() === low) ?? ctx.users.find((x) => x.display_name.toLowerCase() === low);
      out.author_id = u?.user_id ?? null;
      if (u) chip.set('from', u.display_name);
      else bad.set('from', `No one called ${v}.`);
    } else if (t.key === 'in') {
      const r = ctx.rooms.find((x) => x.name.toLowerCase() === low);
      out.room_id = r?.id ?? null;
      if (r) chip.set('in', r.name);
      else bad.set('in', `No room called ${v}.`);
    } else {
      const ms = day(v, ctx.today, t.key === 'before');
      if (t.key === 'after') out.from = ms;
      else out.to = ms === null ? null : ms - 1;
      if (ms !== null) chip.set(t.key, dayText(t.key === 'after' ? ms : ms - 1));
      else bad.set(t.key, `${v} is not a date. Use YYYY-MM-DD.`);
    }
  }
  out.q = text.join(' ');
  out.chips = (['from', 'in', 'after', 'before'] as Key[]).filter((k) => chip.has(k)).map((key) => ({ key, text: chip.get(key)! }));
  out.unknown = [...bad.values()];
  return out;
}

const quote = (v: string) => (/[\s"]/.test(v) ? `"${unquote(v)}"` : v);

/** `input` with every `key:` token removed, then `key:value` put first when a value is given. */
export function setKey(input: string, key: Key, value: string | null): string {
  let rest = input;
  for (const t of tokens(input).reverse()) if (t.key === key) rest = rest.slice(0, t.start) + rest.slice(t.end);
  rest = rest.replace(/\s+/g, ' ').trim();
  return value ? `${key}:${quote(value)} ${rest}` : rest; // a trailing space after a lone filter, ready for the words
}

/** The raw value of the last `key:` token, for the date fields (YYYY-MM-DD or ''). */
export function keyValue(input: string, key: Key): string {
  return tokens(input).filter((t) => t.key === key).at(-1)?.value ?? '';
}

/** A `from:` or `in:` token the caret is in, for the suggestions: its place in `input` and the text after the colon. */
export function keyAt(input: string, caret: number): { key: 'from' | 'in'; q: string; start: number; end: number } | null {
  const t = tokens(input).find((x) => x.start < caret && caret <= x.end);
  return t && (t.key === 'from' || t.key === 'in') ? { key: t.key, q: t.value, start: t.start, end: t.end } : null;
}

/** `input` with the token at `start..end` replaced by `key:value` and a space, and where the caret goes after it. */
export function complete(input: string, at: { key: Key; start: number; end: number }, value: string): { text: string; caret: number } {
  const put = `${at.key}:${quote(value)} `;
  const after = input.slice(at.end).replace(/^\s+/, '');
  return { text: input.slice(0, at.start) + put + after, caret: at.start + put.length };
}

/** True when `q` has nothing the server can search (no letter or digit): it would answer with no hits. */
export const nothingToSearch = (q: string) => !/[\p{L}\p{N}]/u.test(q);

/**
 * True when the last word ends in a single letter or digit. The server matches that as a whole word, not as the
 * start of one (a one-letter prefix is its slow case), so "hello w" finds "w" only: the dialog says to type more.
 */
export function shortLast(q: string): boolean {
  const last = q.trim().split(/\s+/).at(-1) ?? '';
  const run = last.split(/[^\p{L}\p{N}]+/u).filter(Boolean).at(-1) ?? '';
  return [...run].length === 1;
}
