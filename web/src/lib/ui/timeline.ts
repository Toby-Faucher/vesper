// The rows of the timeline: day rows, the New row and messages grouped by author. Pure, so it is tested without a DOM.
import type { Message } from '../gen/Message';
import type { Md } from '../markdown';

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

/** "Today", "Yesterday", "Mon 27 Oct", or "Wed 31 Dec 2025" for another year. */
export function dayLabel(ms: number, now: number, tz?: string): string {
  const k = dayKey(ms, tz);
  const today = dayKey(now, tz);
  if (k === today) return 'Today';
  if (k === prevKey(today)) return 'Yesterday';
  const year = k.slice(0, 4) !== today.slice(0, 4);
  return new Intl.DateTimeFormat('en-GB', { weekday: 'short', day: 'numeric', month: 'short', year: year ? 'numeric' : undefined, timeZone: tz })
    .format(ms)
    .replace(',', '');
}

/**
 * A day row before the first message of each local day; a New row before the first message after `lastReadId` that
 * is not mine. A message heads a group unless the one before it has the same author, is less than 5 minutes older,
 * and neither is deleted. A reply, a new day and the New row always start a group.
 */
export function rows(messages: Message[], lastReadId: number, myId: string, now: number, tz?: string): Row[] {
  const out: Row[] = [];
  let prev: Message | null = null;
  let day = '';
  let newDone = false;
  for (const m of messages) {
    let head = !prev || prev.author_id !== m.author_id || m.created_at - prev.created_at >= GROUP_MS || prev.deleted || m.deleted || !!m.reply_to;
    const k = dayKey(m.created_at, tz);
    if (k !== day) {
      day = k;
      out.push({ kind: 'day', label: dayLabel(m.created_at, now, tz), key: `d${k}` });
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
          case 'br': return ' ';
          case 'link': return n.href;
          case 'mention': return `@${n.username}`;
          case 'quote': return ` ${walk(n.c)} `;
          default: return walk(n.c);
        }
      })
      .join('');
  return walk(nodes).replace(/\s+/g, ' ').trim();
}

/** Bodies above about 20 lines collapse behind "Show more". Counted from the text, so no row is measured. */
export const isLong = (body: string) => (body.match(/\n/g)?.length ?? 0) > 20 || body.length > 2000;
