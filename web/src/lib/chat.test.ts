import { afterEach, describe, expect, it, vi } from 'vitest';
import { anchorTop, applyChatMsg, marks, mergeLatest, typers, type ChatState } from './chat.svelte';
import type { Message } from './gen/Message';

const msg = (id: number, over: Partial<Message> = {}): Message => ({
  id, room_id: 1, author_id: 'a', body: `m${id}`, created_at: id, edited_at: null, deleted: false, reply_to: null,
  reactions: [], attachments: [], mentions: [], pinned: false, ...over,
});
const loaded = (ids: number[], moreAfter = false): ChatState => ({
  rooms: { 1: { messages: ids.map((i) => msg(i)), moreBefore: true, moreAfter } }, typing: {},
});
const ids = (s: ChatState) => s.rooms[1].messages.map((m) => m.id);

afterEach(() => vi.useRealTimers());

describe('applyChatMsg', () => {
  it('message_new appends in id order and ignores duplicates', () => {
    let s = applyChatMsg(loaded([1, 3]), { type: 'message_new', message: msg(4) });
    s = applyChatMsg(s, { type: 'message_new', message: msg(2) });
    s = applyChatMsg(s, { type: 'message_new', message: msg(4) });
    expect(ids(s)).toEqual([1, 2, 3, 4]);
  });

  it('message_new is not added to an older slice or an unloaded room', () => {
    expect(ids(applyChatMsg(loaded([1], true), { type: 'message_new', message: msg(9) }))).toEqual([1]);
    expect(applyChatMsg(loaded([1]), { type: 'message_new', message: msg(9, { room_id: 2 }) }).rooms[2]).toBeUndefined();
  });

  it('message_updated_replaces_in_place', () => {
    const s = applyChatMsg(loaded([1, 2, 3]), { type: 'message_updated', message: msg(2, { body: 'new', edited_at: 10 }) });
    expect(ids(s)).toEqual([1, 2, 3]);
    expect(s.rooms[1].messages[1].body).toBe('new');
    // An update for a message outside the loaded slice is not inserted.
    expect(ids(applyChatMsg(s, { type: 'message_updated', message: msg(7) }))).toEqual([1, 2, 3]);
  });

  it('a stale message_updated never replaces a newer copy', () => {
    let s = applyChatMsg(loaded([1]), { type: 'message_updated', message: msg(1, { body: 'second', edited_at: 20 }) });
    s = applyChatMsg(s, { type: 'message_updated', message: msg(1, { body: 'first', edited_at: 10 }) });
    expect(s.rooms[1].messages[0].body).toBe('second');
    s = applyChatMsg(s, { type: 'message_updated', message: msg(1, { body: '', deleted: true, edited_at: 20 }) });
    s = applyChatMsg(s, { type: 'message_updated', message: msg(1, { body: 'second', edited_at: 20, pinned: true }) });
    expect(s.rooms[1].messages[0].deleted).toBe(true);
  });

  it('typing_expires_after_5s', () => {
    vi.useFakeTimers();
    let s = applyChatMsg(loaded([1]), { type: 'typing', room_id: 1, user_id: 'b' });
    expect(typers(s, 1)).toEqual(['b']);
    vi.advanceTimersByTime(4900);
    expect(typers(s, 1)).toEqual(['b']);
    vi.advanceTimersByTime(100);
    expect(typers(s, 1)).toEqual([]);
    // A message from the typer ends the line at once.
    s = applyChatMsg(s, { type: 'typing', room_id: 1, user_id: 'b' });
    s = applyChatMsg(s, { type: 'message_new', message: msg(2, { author_id: 'b' }) });
    expect(typers(s, 1)).toEqual([]);
  });
});

describe('mergeLatest (refetch after a reconnect)', () => {
  const page = (ms: Message[], more_before = true) => ({ messages: ms, more_before, more_after: false });

  it('keeps older held messages and takes the fetched copies', () => {
    const held = loaded([1, 2, 3]).rooms[1];
    const r = mergeLatest(held, page([msg(2, { body: 'edited', edited_at: 5 }), msg(3), msg(4)]));
    expect(r.messages.map((m) => m.id)).toEqual([1, 2, 3, 4]);
    expect(r.messages[1].body).toBe('edited');
    expect(r.moreBefore).toBe(true);
  });

  it('a held copy newer than the fetched one wins', () => {
    const held = { messages: [msg(2, { deleted: true, body: '' })], moreBefore: false, moreAfter: false };
    expect(mergeLatest(held, page([msg(2)])).messages[0].deleted).toBe(true);
  });

  it('a gap or an older slice is replaced by the page', () => {
    const r = mergeLatest(loaded([1, 2]).rooms[1], page([msg(60), msg(61)]));
    expect(r.messages.map((m) => m.id)).toEqual([60, 61]);
    expect(mergeLatest(loaded([1, 70], true).rooms[1], page([msg(60)])).messages.map((m) => m.id)).toEqual([60]);
    expect(mergeLatest(undefined, page([], false))).toEqual({ messages: [], moreBefore: false, moreAfter: false });
  });
});

describe('helpers', () => {
  it('marks splits a snippet at the markers, as text', () => {
    expect(marks('...say \u0002hello\u0003 to <b>\u0002hel\u0003...')).toEqual([
      { text: '...say ', mark: false }, { text: 'hello', mark: true }, { text: ' to <b>', mark: false },
      { text: 'hel', mark: true }, { text: '...', mark: false },
    ]);
    expect(marks('a\u0003b\u0002c')).toEqual([{ text: 'a', mark: false }, { text: 'b', mark: false }, { text: 'c', mark: true }]);
  });

  it('anchorTop keeps the viewport stable when older rows are prepended', () => {
    // 300 px of older messages arrived above; the reader stays on the same row.
    expect(anchorTop(40, 1000, 1300)).toBe(340);
    expect(anchorTop(0, 500, 500)).toBe(0);
  });
});
