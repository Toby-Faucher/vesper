// The Ctrl/Cmd+K switcher's matching and ordering. Plain functions, so the order is tested without a page.
import type { Unread } from '../gen/Unread';

export type Item = { kind: 'room' | 'person' | 'action'; id: string; label: string; hint?: string; run: () => void };
export type RankContext = { unread: Record<number, Unread>; recent: number[] };

export const MAX_RESULTS = 30;
const KINDS: Item['kind'][] = ['room', 'person', 'action'];
export const SCOPES: Record<string, Item['kind']> = { 'r ': 'room', 'p ': 'person', '> ': 'action' };

// Code points, so a letter outside the BMP counts once and is never split.
const chars = (s: string) => Array.from(s.toLowerCase());
const isWordStart = (cs: string[], i: number) => i === 0 || !/[\p{L}\p{N}]/u.test(cs[i - 1]);

/**
 * How well `query` matches `label`, higher is better, null for no match. Case-insensitive. A prefix beats a match at a
 * word start, which beats a match inside a word, which beats a scattered subsequence; within a band shorter labels and
 * earlier matches win. The subsequence ignores spaces in the query and rewards letters at word starts and in runs.
 */
export function score(query: string, label: string): number | null {
  const q = chars(query.trim());
  if (!q.length) return 0;
  const l = chars(label);
  const at = indexOf(l, q);
  if (at === 0) return 3000 - l.length;
  if (at > 0) {
    const start = starts(l, q);
    return start >= 0 ? 2000 - start - l.length / 100 : 1000 - at - l.length / 100;
  }
  // Best placement of each query letter (a small table, labels are short): a letter scores 1, plus 3 at a word start,
  // plus 2 right after the previous letter.
  const want = q.filter((c) => !/\s/u.test(c));
  let row = l.map((c, i) => (c === want[0] ? 1 + (isWordStart(l, i) ? 3 : 0) : -Infinity));
  for (let j = 1; j < want.length; j++) {
    let before = -Infinity; // best of row[0 .. i-2]
    row = l.map((c, i) => {
      if (i >= 2) before = Math.max(before, row[i - 2]);
      if (c !== want[j] || i === 0) return -Infinity;
      return 1 + (isWordStart(l, i) ? 3 : 0) + Math.max(before, row[i - 1] + 2);
    });
  }
  const best = Math.max(...row);
  return best > -Infinity ? best - l.length / 100 : null;
}

function indexOf(l: string[], q: string[], from = 0): number {
  outer: for (let i = from; i + q.length <= l.length; i++) {
    for (let k = 0; k < q.length; k++) if (l[i + k] !== q[k]) continue outer;
    return i;
  }
  return -1;
}
// The first place `q` occurs at a word start, or -1.
function starts(l: string[], q: string[]): number {
  for (let i = indexOf(l, q); i >= 0; i = indexOf(l, q, i + 1)) if (isWordStart(l, i)) return i;
  return -1;
}

/**
 * The switcher's list for `q`, grouped by kind (rooms, people, actions; a group whose best match is higher comes first
 * when there is a query), at most MAX_RESULTS. `r `, `p ` and `> ` scope the query to one kind. The order before
 * matching, and the tie-break after it: rooms with mentions (most first), then other unread rooms (most first), then
 * `ctx.recent` (most recent first), then the order given (the caller passes rooms by position, people in voice first).
 */
export function rank(q: string, items: Item[], ctx: RankContext): Item[] {
  const scope = SCOPES[q.slice(0, 2).toLowerCase()];
  const query = scope ? q.slice(2) : q;
  const prio = (it: Item) => {
    if (it.kind !== 'room') return 0;
    const u = ctx.unread[Number(it.id)];
    if (u?.mentions) return 3e6 + u.mentions;
    if (u?.unread) return 2e6 + u.unread;
    const r = ctx.recent.indexOf(Number(it.id));
    return r >= 0 ? 1e6 - r : 0;
  };
  const scored = items
    .filter((it) => !scope || it.kind === scope)
    .map((it, i) => ({ it, i, p: prio(it), s: score(query, it.label) }))
    .filter((x): x is typeof x & { s: number } => x.s !== null);
  const kindAt = (k: Item['kind']) => KINDS.indexOf(k);
  // No query: kind first. A query: best score first, and the groups follow their best member.
  scored.sort((a, b) => (query.trim() ? b.s - a.s : kindAt(a.it.kind) - kindAt(b.it.kind)) || b.p - a.p || a.i - b.i);
  const top = scored.slice(0, MAX_RESULTS).map((x) => x.it);
  const order = [...new Set(top.map((it) => it.kind))];
  return order.flatMap((k) => top.filter((it) => it.kind === k));
}
