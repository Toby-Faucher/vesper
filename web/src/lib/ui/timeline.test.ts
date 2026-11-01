import { describe, expect, it } from 'vitest';
import type { Message } from '../gen/Message';
import { parse } from '../markdown';
import { dayLabel, firstUnreadInPage, isLong, plain, rows } from './timeline';

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
    const r = rows([msg(1), msg(2, { created_at: T + 3 * MIN })], 99, 'me', NOW, TZ);
    expect(shape(r)).toEqual(['[Tue 27 Oct]', '1H', '[Wed 28 Oct]', '2H']);
    // The same instants in New York are both on the 27th (evening), so one day row and a group.
    expect(shape(rows([msg(1), msg(2, { created_at: T + 3 * MIN })], 99, 'me', NOW, 'America/New_York'))).toEqual(['[Tue 27 Oct]', '1H', '2']);
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
    expect(dayLabel(NOW - 2 * 24 * 60 * MIN, NOW, TZ)).toBe('Wed 28 Oct');
    expect(dayLabel(T, NOW, TZ)).toBe('Tue 27 Oct');
    expect(dayLabel(Date.UTC(2025, 11, 31, 12), NOW, TZ)).toBe('Wed 31 Dec 2025');
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
