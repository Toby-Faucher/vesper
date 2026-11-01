import { describe, expect, it } from 'vitest';
import type { Message } from '../gen/Message';
import { parse } from '../markdown';
import { arrivals, canSee, dayLabel, firstUnreadInPage, follows, isLong, mentionQuery, mentionable, plain, rows, typingLine } from './timeline';

const TZ = 'Europe/London';
// 2026-10-27 is a Tuesday. 23:58 London time (GMT, after the October change) is 23:58 UTC.
const T = Date.UTC(2026, 9, 27, 23, 58);
const MIN = 60_000;
const msg = (id: number, over: Partial<Message> = {}): Message => ({
  id, room_id: 1, author_id: 'a', body: `m${id}`, created_at: T, edited_at: null, updated_at: 1, deleted: false, reply_to: null,
  reactions: [], attachments: [], mentions: [], pinned: false, ...over,
});
const shape = (r: ReturnType<typeof rows>) => r.map((x) => (x.kind === 'msg' ? `${x.m.id}${x.head ? 'H' : ''}` : x.kind === 'day' ? `[${x.label}]` : 'NEW'));
const NOW = T + 3 * 24 * 60 * MIN; // Friday 30 Oct

describe('rows', () => {
  it('puts a day row before the first message of each local day, across midnight', () => {
    const r = rows([msg(1), msg(2, { created_at: T + 3 * MIN })], 99, 'me', NOW, TZ, 'en-GB');
    expect(shape(r)).toEqual(['[Tue 27 Oct]', '1H', '[Wed 28 Oct]', '2H']);
    // The same instants in New York are both on the 27th (evening), so one day row and a group.
    expect(shape(rows([msg(1), msg(2, { created_at: T + 3 * MIN })], 99, 'me', NOW, 'America/New_York', 'en-GB'))).toEqual(['[Tue 27 Oct]', '1H', '2']);
  });

  it('groups by author within 5 minutes', () => {
    const t = T - 60 * MIN;
    const list = [
      msg(1, { created_at: t }),
      msg(2, { created_at: t + 5 * MIN - 1 }), // just inside
      msg(3, { created_at: t + 10 * MIN - 1 }), // 5 min after 2: a new group
      msg(4, { created_at: t + 10 * MIN, author_id: 'b' }), // another author
    ];
    expect(shape(rows(list, 99, 'me', NOW, TZ)).slice(1)).toEqual(['1H', '2', '3H', '4H']);
  });

  it('a deleted message breaks a group, and so does a reply', () => {
    const t = T - 60 * MIN;
    const list = [msg(1, { created_at: t }), msg(2, { created_at: t, deleted: true }), msg(3, { created_at: t }),
      msg(4, { created_at: t, reply_to: { id: 1, author_id: 'a', excerpt: 'm1' } })];
    expect(shape(rows(list, 99, 'me', NOW, TZ)).slice(1)).toEqual(['1H', '2H', '3H', '4H']);
  });

  it('puts the New row before the first unread message from someone else, and starts a group there', () => {
    const t = T - 60 * MIN;
    const list = [msg(1, { created_at: t }), msg(2, { created_at: t, author_id: 'me' }), msg(3, { created_at: t, author_id: 'me' }), msg(4, { created_at: t, author_id: 'me' })];
    list.push(msg(5, { created_at: t, author_id: 'me' }));
    expect(shape(rows(list, 1, 'me', NOW, TZ)).slice(1)).toEqual(['1H', '2H', '3', '4', '5']); // my own unread: no row
    const mixed = [msg(1, { created_at: t }), msg(2, { created_at: t, author_id: 'me' }), msg(3, { created_at: t }), msg(4, { created_at: t })];
    expect(shape(rows(mixed, 1, 'me', NOW, TZ)).slice(1)).toEqual(['1H', '2H', 'NEW', '3H', '4']);
  });

  it('keys are unique and stable', () => {
    const r = rows([msg(1), msg(2, { created_at: T + 3 * MIN, author_id: 'b' })], 1, 'me', NOW, TZ);
    expect(new Set(r.map((x) => x.key)).size).toBe(r.length);
  });
});

describe('dayLabel', () => {
  it('says Today, Yesterday, a date, and the year when it is not this year', () => {
    expect(dayLabel(NOW, NOW, TZ)).toBe('Today');
    expect(dayLabel(NOW - 24 * 60 * MIN, NOW, TZ)).toBe('Yesterday');
    expect(dayLabel(NOW - 2 * 24 * 60 * MIN, NOW, TZ, 'en-GB')).toBe('Wed 28 Oct');
    expect(dayLabel(T, NOW, TZ, 'en-GB')).toBe('Tue 27 Oct');
    expect(dayLabel(Date.UTC(2025, 11, 31, 12), NOW, TZ, 'en-GB')).toBe('Wed 31 Dec 2025');
  });

  it('follows the locale, like the date fields the browser draws', () => {
    expect(dayLabel(T, NOW, TZ, 'en-US')).toBe('Tue Oct 27');
    expect(dayLabel(Date.UTC(2025, 11, 31, 12), NOW, TZ, 'en-US')).toBe('Wed Dec 31, 2025');
  });
});

describe('firstUnreadInPage', () => {
  const list = [msg(10), msg(11, { author_id: 'me' }), msg(12), msg(13)];
  it('finds the first unread message from someone else', () => {
    expect(firstUnreadInPage(list, 10, 'me', true)).toBe(12);
    expect(firstUnreadInPage(list, 13, 'me', true)).toBeNull();
  });
  it('is null when the first unread may be older than the page', () => {
    expect(firstUnreadInPage(list, 5, 'me', true)).toBeNull();
    expect(firstUnreadInPage(list, 5, 'me', false)).toBe(10); // the page starts the room
    expect(firstUnreadInPage([msg(11, { author_id: 'me' }), msg(12)], 10, 'me', true)).toBe(12); // 11 is mine and the read mark is 10
  });
});

describe('plain', () => {
  it('reads the rendered text, not the Markdown', () => {
    expect(plain(parse('hello **bob** _x_ `code` @alice https://example.test/a'))).toBe('hello bob x code @alice https://example.test/a');
    expect(plain(parse('> quoted\nline two'))).toBe('quoted line two');
  });
});

describe('isLong', () => {
  it('collapses above 20 lines or a very long body', () => {
    expect(isLong('a\n'.repeat(20))).toBe(false);
    expect(isLong('a\n'.repeat(21))).toBe(true);
    expect(isLong('x'.repeat(2001))).toBe(true);
  });
});

describe('mentionQuery', () => {
  const at = (t: string) => mentionQuery(t.replace('|', ''), t.indexOf('|'));
  it('finds an @ at a word start before the caret', () => {
    expect(at('@|')).toEqual({ start: 0, q: '' });
    expect(at('hi @al|')).toEqual({ start: 3, q: 'al' });
    expect(at('hi @al| there')).toEqual({ start: 3, q: 'al' });
    expect(at('line\n(@bo|')).toEqual({ start: 6, q: 'bo' });
    expect(at('@Zoë|')).toEqual({ start: 0, q: 'Zoë' });
    expect(at('@🙂x|')).toEqual({ start: 0, q: '🙂x' });
  });
  it('is null after a space, inside a word, or with the caret before the @', () => {
    expect(at('@al ex|')).toBeNull();
    expect(at('mail@host|')).toBeNull();
    expect(at('|@al')).toBeNull();
    expect(at('no mention|')).toBeNull();
  });
  it('is null inside inline code and code blocks', () => {
    expect(at('`@al|')).toBeNull();
    expect(at('`x` @al|')).toEqual({ start: 4, q: 'al' });
    expect(at('```\n@al|')).toBeNull();
    expect(at('```\ncode\n```\n@al|')).toEqual({ start: 13, q: 'al' });
  });
});

describe('mentionable', () => {
  const u = (user_id: string, username: string, display_name = username, role = 'member', banned = false) => ({ user_id, username, display_name, role, banned });
  const people = [u('1', 'alice'), u('2', 'alan', 'Alan Wake'), u('3', 'zoe', 'Zoë'), u('4', 'mod', 'Mod', 'moderator'), u('5', 'albert', 'Al', 'member', true), u('9', 'me')];
  const open = { restricted: false, members: [] as string[] };
  const names = (q: string, room = open) => mentionable(q, people, room, '9').map((x) => x.username);
  it('offers only people who can see the room, never me or the banned', () => {
    expect(names('')).toEqual(['alan', 'alice', 'mod', 'zoe']);
    expect(names('', { restricted: true, members: ['3'] })).toEqual(['mod', 'zoe']);
    expect(names('al', { restricted: true, members: ['1'] })).toEqual(['alice']);
    expect(canSee(people[0], { restricted: true, members: [] })).toBe(false);
    expect(canSee(people[3], { restricted: true, members: [] })).toBe(true);
  });
  it('filters by username or display name, best first, any script', () => {
    expect(names('al')).toEqual(['alan', 'alice']);
    expect(names('wake')).toEqual(['alan']);
    expect(names('zoë')).toEqual(['zoe']);
    expect(names('ZO')).toEqual(['zoe']);
    expect(names('xq')).toEqual([]);
    expect(mentionable('', people, open, '9', 2)).toHaveLength(2);
  });
});

describe('typingLine', () => {
  it('names one or two, then says several', () => {
    expect(typingLine([])).toBe('');
    expect(typingLine(['Sam'])).toBe('Sam is typing');
    expect(typingLine(['Sam', 'Alex'])).toBe('Sam and Alex are typing');
    expect(typingLine(['Sam', 'Alex', 'Kim'])).toBe('Several people are typing');
  });
});

describe('follows', () => {
  it('follows within 8 px of the bottom, from any position before', () => {
    expect(follows(false, 0, false)).toBe(true);
    expect(follows(false, 7, false)).toBe(true);
  });
  it('stops following anywhere else, also without a move up (a change above pushed the reader off)', () => {
    expect(follows(true, 8, false)).toBe(false);
    expect(follows(true, 300, false)).toBe(false);
  });
  it('keeps following while the list changed height and its ResizeObserver has not run yet', () => {
    expect(follows(true, 64, true)).toBe(true);
    expect(follows(false, 64, true)).toBe(false);
  });
});

describe('arrivals', () => {
  it('animates live arrivals only, three at most', () => {
    expect(arrivals(null, [1, 2, 3])).toEqual([]); // a room open: the history appears at once
    expect(arrivals(10, [4, 5, 6, 7, 8, 9, 10])).toEqual([]); // Load older: nothing newer than before
    expect(arrivals(10, [9, 10, 11])).toEqual([11]);
    expect(arrivals(10, [10, 11, 12, 13])).toEqual([11, 12, 13]);
    expect(arrivals(10, [10, 11, 12, 13, 14])).toEqual([]); // Load newer: a page, not a live burst
  });
});
