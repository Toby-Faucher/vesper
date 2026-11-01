// The rows of the timeline: day rows, the New row and messages grouped by author. Pure, so it is tested without a DOM.
import type { Message } from '../gen/Message';
import type { Md } from '../markdown';
import { score } from './switcher';

export type Row =
  | { kind: 'day'; label: string; key: string }
  | { kind: 'new'; key: string }
  | { kind: 'msg'; m: Message; head: boolean; key: string };

const GROUP_MS = 5 * 60_000;

const keyFmt = new Map<string, Intl.DateTimeFormat>();
/** The local day of `ms` as YYYY-MM-DD in `tz` (the browser's zone when not given). */
function dayKey(ms: number, tz?: string): string {
  const k = tz ?? '';
  let f = keyFmt.get(k);
  if (!f) keyFmt.set(k, (f = new Intl.DateTimeFormat('en-CA', { year: 'numeric', month: '2-digit', day: '2-digit', timeZone: tz })));
  return f.format(ms);
}

/** The day before a YYYY-MM-DD key, by calendar (a DST day has 23 or 25 hours). */
function prevKey(key: string): string {
  const [y, m, d] = key.split('-').map(Number);
  return new Date(Date.UTC(y, m - 1, d - 1)).toISOString().slice(0, 10);
}

/**
 * "Today", "Yesterday", "Mon 27 Oct", or "Wed 31 Dec 2025" for another year, in the browser's locale (`locale` for tests),
 * so the order matches the search dialog's date fields, which the browser draws ("Mon Oct 27" in en-US).
 */
export function dayLabel(ms: number, now: number, tz?: string, locale?: string): string {
  const k = dayKey(ms, tz);
  const today = dayKey(now, tz);
  if (k === today) return 'Today';
  if (k === prevKey(today)) return 'Yesterday';
  const year = k.slice(0, 4) !== today.slice(0, 4);
  return new Intl.DateTimeFormat(locale, { weekday: 'short', day: 'numeric', month: 'short', year: year ? 'numeric' : undefined, timeZone: tz })
    .format(ms)
    .replace(',', '');
}

/**
 * A day row before the first message of each local day; a New row before the first message after `lastReadId` that
 * is not mine. A message heads a group unless the one before it has the same author, is less than 5 minutes older,
 * and neither is deleted. A reply, a new day and the New row always start a group.
 */
export function rows(messages: Message[], lastReadId: number, myId: string, now: number, tz?: string, locale?: string): Row[] {
  const out: Row[] = [];
  let prev: Message | null = null;
  let day = '';
  let newDone = false;
  for (const m of messages) {
    let head = !prev || prev.author_id !== m.author_id || m.created_at - prev.created_at >= GROUP_MS || prev.deleted || m.deleted || !!m.reply_to;
    const k = dayKey(m.created_at, tz);
    if (k !== day) {
      day = k;
      out.push({ kind: 'day', label: dayLabel(m.created_at, now, tz, locale), key: `d${k}` });
      head = true;
    }
    if (!newDone && m.id > lastReadId && m.author_id !== myId) {
      newDone = true;
      out.push({ kind: 'new', key: 'new' });
      head = true;
    }
    out.push({ kind: 'msg', m, head, key: `m${m.id}` });
    prev = m;
  }
  return out;
}

/**
 * The id of the first unread message from someone else when it is in the loaded page, else null. Ids grow, so with
 * older history not loaded (`moreBefore`) it is only known when nothing unread can be older than the page.
 */
export function firstUnreadInPage(messages: Message[], lastReadId: number, myId: string, moreBefore = true): number | null {
  const first = messages[0];
  if (!first || (moreBefore && first.id > lastReadId + 1)) return null;
  return messages.find((m) => m.id > lastReadId && m.author_id !== myId)?.id ?? null;
}

/** The text a reader sees in rendered Markdown, on one line: for screen reader announcements. */
export function plain(nodes: Md[]): string {
  const walk = (list: Md[]): string =>
    list
      .map((n) => {
        switch (n.t) {
          case 'text': case 'code': return n.v;
          case 'pre': return ` ${n.v} `;
          case 'br': case 'hr': return ' ';
          case 'link': return n.href;
          case 'mention': return `@${n.username}`;
          case 'quote': case 'h': case 'li': return ` ${walk(n.c)} `;
          default: return walk(n.c);
        }
      })
      .join('');
  return walk(nodes).replace(/\s+/g, ' ').trim();
}

/** Bodies above about 20 lines collapse behind "Show more". Counted from the text, so no row is measured. */
export const isLong = (body: string) => (body.match(/\n/g)?.length ?? 0) > 20 || body.length > 2000;

/**
 * The `@` mention being typed at `caret`: where the `@` is and the text after it. Null when the `@` does not start a
 * word (as in an email address), when a space comes after it, or inside inline code or a code block.
 */
export function mentionQuery(text: string, caret: number): { start: number; q: string } | null {
  const before = text.slice(0, caret);
  const at = before.lastIndexOf('@');
  if (at < 0) return null;
  const q = before.slice(at + 1);
  if (q.length > 32 || /\s|@/u.test(q)) return null;
  const prev = before[at - 1];
  if (prev !== undefined && !/[\s([{"'>]/u.test(prev)) return null;
  const lines = before.slice(0, at).split('\n');
  const fences = lines.slice(0, -1).filter((l) => l.startsWith('```')).length;
  if (fences % 2 || (lines.at(-1)!.split('`').length - 1) % 2) return null;
  return { start: at, q };
}

type Person = { user_id: string; username: string; display_name: string; role: string; banned: boolean };
type Seen = { restricted: boolean; members: string[] };

/** True when the person can read the room: anyone for an open room; the owner, moderators and listed members for a restricted one. */
export const canSee = (u: Person, room: Seen) => !room.restricted || u.role === 'owner' || u.role === 'moderator' || room.members.includes(u.user_id);

/**
 * People to offer for `@q` in `room`, best match first: only those who can see the room, not banned, not me. Matches the
 * username and the display name with the switcher's scorer (code points, so any script works). At most `max`.
 */
export function mentionable<U extends Person>(q: string, users: U[], room: Seen, myId: string, max = 8): U[] {
  const out: { u: U; s: number }[] = [];
  for (const u of users) {
    if (u.banned || u.user_id === myId || !canSee(u, room)) continue;
    const s = Math.max(score(q, u.username) ?? -Infinity, score(q, u.display_name) ?? -Infinity);
    if (s > -Infinity) out.push({ u, s });
  }
  out.sort((a, b) => b.s - a.s || a.u.display_name.localeCompare(b.u.display_name));
  return out.slice(0, max).map((x) => x.u);
}

/** "Sam is typing", "Sam and Alex are typing", "Several people are typing", or '' for nobody. */
export function typingLine(names: string[]): string {
  if (!names.length) return '';
  if (names.length === 1) return `${names[0]} is typing`;
  if (names.length === 2) return `${names[0]} and ${names[1]} are typing`;
  return 'Several people are typing';
}

/**
 * Whether the timeline follows the bottom after a scroll event. The position decides: within 8 px of the bottom it
 * follows, anywhere else it does not (a reader moved off the bottom by anything, a scroll or a change above, is left
 * there). One exception: `resized`, the list changed height since its ResizeObserver last ran (the upload tray grew),
 * and the observer is about to put it back at the bottom, so it keeps what it was.
 */
export const follows = (was: boolean, gap: number, resized: boolean) => gap < 8 || (resized && was);

/**
 * The messages that rise in after the list changed: live arrivals only, three at most. None on a room open (`since` is
 * null), none on "Load older" (nothing newer than `since`), none for a page of newer messages (more than three).
 */
export function arrivals(since: number | null, ids: number[]): number[] {
  if (since === null) return [];
  const fresh = ids.filter((id) => id > since);
  return fresh.length <= 3 ? fresh : [];
}
