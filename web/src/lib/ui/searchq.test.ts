import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiFailure } from '../api';
import type { RoomInfo } from '../gen/RoomInfo';
import type { SearchPage } from '../gen/SearchPage';
import type { SearchQuery } from '../gen/SearchQuery';
import type { UserInfo } from '../gen/UserInfo';
import { Searcher } from './search.svelte';
import { complete, keyAt, keyValue, nothingToSearch, parseSearch, setKey, shortLast } from './searchq';

const user = (user_id: string, username: string, display_name: string): UserInfo =>
  ({ user_id, username, display_name, accent: '', role: 'member', banned: false });
const room = (id: number, name: string): RoomInfo => ({ id, name, position: id, restricted: false, members: [] });
const ctx = {
  users: [user('a1', 'alice', 'Alice'), user('s1', 'sam', 'Sam Smith'), user('b1', 'bob', 'sam')],
  rooms: [room(1, 'general'), room(2, 'game night')],
  today: new Date(2026, 10, 1, 15, 30),
};
const p = (s: string) => parseSearch(s, ctx);
const local = (y: number, m: number, d: number) => new Date(y, m - 1, d).getTime();

describe('parseSearch', () => {
  it('reads each key and leaves the rest as text, in any order', () => {
    const a = p('from:alice in:general hello after:2026-10-01 before:2026-10-31 there');
    expect(a).toMatchObject({ q: 'hello there', author_id: 'a1', room_id: 1, from: local(2026, 10, 1), to: local(2026, 11, 1) - 1, unknown: [] });
    expect(a.chips.map((c) => c.key)).toEqual(['from', 'in', 'after', 'before']);
    expect(p('hello before:2026-10-31 in:general from:alice after:2026-10-01').chips).toEqual(a.chips);
  });
  it('turns dates into local days, both included', () => {
    expect(p('after:2026-03-29').from).toBe(new Date(2026, 2, 29, 0, 0, 0, 0).getTime());
    expect(p('before:2026-03-29').to).toBe(new Date(2026, 2, 29, 23, 59, 59, 999).getTime());
    expect(p('after:today').from).toBe(local(2026, 11, 1));
    expect(p('before:yesterday').to).toBe(local(2026, 11, 1) - 1);
  });
  it('says which names and dates it does not know', () => {
    const a = p('from:carl in:x after:2026-02-30 before:soon hi');
    expect(a).toMatchObject({ q: 'hi', author_id: null, room_id: null, from: null, to: null, chips: [] });
    expect(a.unknown).toEqual(['No one called carl.', 'No room called x.', '2026-02-30 is not a date. Use YYYY-MM-DD.', 'soon is not a date. Use YYYY-MM-DD.']);
  });
  it('keeps a quoted name with spaces together, and matches names without case', () => {
    expect(p('from:"Sam Smith" in:"Game Night" plans')).toMatchObject({ author_id: 's1', room_id: 2, q: 'plans' });
    expect(p('FROM:ALICE').author_id).toBe('a1');
    expect(p('from:sam').author_id).toBe('s1'); // a username wins over another person's display name
    expect(p('"see you" soon').q).toBe('see you soon');
  });
  it('allows filters with no words, and ignores a key with no value yet', () => {
    expect(p('from:alice')).toMatchObject({ q: '', author_id: 'a1' });
    expect(p('hello from:')).toMatchObject({ q: 'hello', author_id: null, unknown: [], chips: [] });
  });
  it('treats other words with a colon, and search syntax, as text', () => {
    expect(p('to:bob http://x.y has:link').q).toBe('to:bob http://x.y has:link');
    expect(p('a OR b NEAR(c d) * "x" -- ; DROP TABLE messages').q).toBe('a OR b NEAR(c d) * x -- ; DROP TABLE messages');
    expect(p('<img src=x onerror=alert(1)>').q).toBe('<img src=x onerror=alert(1)>');
  });
  it('lets the last of a repeated key count', () => {
    const a = p('from:carl from:alice');
    expect(a).toMatchObject({ author_id: 'a1', unknown: [] });
    expect(a.chips).toEqual([{ key: 'from', text: 'Alice' }]);
  });
});

describe('editing the box', () => {
  it('sets, replaces and removes a key', () => {
    expect(setKey('hello', 'in', 'general')).toBe('in:general hello');
    expect(setKey('in:general hello in:x', 'in', 'game night')).toBe('in:"game night" hello');
    expect(setKey('from:"Sam Smith" hello', 'from', null)).toBe('hello');
    expect(setKey('', 'after', '2026-10-01')).toBe('after:2026-10-01 ');
    expect(keyValue('after:2026-10-01 hi', 'after')).toBe('2026-10-01');
  });
  it('finds the key at the caret and completes it', () => {
    const s = 'hi from:sa there';
    const at = keyAt(s, 10)!;
    expect(at).toEqual({ key: 'from', q: 'sa', start: 3, end: 10 });
    expect(keyAt(s, 2)).toBe(null);
    expect(keyAt('after:2', 7)).toBe(null);
    expect(complete(s, at, 'sam')).toEqual({ text: 'hi from:sam there', caret: 12 });
    expect(complete('in:ga', keyAt('in:ga', 5)!, 'game night')).toEqual({ text: 'in:"game night" ', caret: 16 });
  });
});

describe('hints', () => {
  it('knows when there is nothing to search', () => {
    expect(nothingToSearch('')).toBe(true);
    expect(nothingToSearch('?!. --')).toBe(true);
    expect(nothingToSearch('é')).toBe(false);
  });
  it('spots a one-letter last word', () => {
    expect(shortLast('h')).toBe(true);
    expect(shortLast('hello w')).toBe(true);
    expect(shortLast('x-a')).toBe(true);
    expect(shortLast('a.')).toBe(true);
    expect(shortLast('hello wo')).toBe(false);
    expect(shortLast('é')).toBe(true);
  });
});

describe('Searcher', () => {
  afterEach(() => vi.useRealTimers());
  const q = (text: string) => ({ q: text, room_id: null, author_id: null, from: null, to: null });
  const page = (ids: number[], next: number | null = null): SearchPage =>
    ({ hits: ids.map((id) => ({ snippet: '', message: { id } as SearchPage['hits'][0]['message'] })), next_before: next });

  it('waits 300 ms after the last change and sends only the last query', async () => {
    vi.useFakeTimers();
    const run = vi.fn(async (_: SearchQuery) => page([1]));
    const s = new Searcher(run);
    s.set(q('he'));
    await vi.advanceTimersByTimeAsync(200);
    s.set(q('hel'));
    await vi.advanceTimersByTimeAsync(299);
    expect(run).not.toHaveBeenCalled();
    expect(s.status).toBe('loading');
    await vi.advanceTimersByTimeAsync(1);
    expect(run).toHaveBeenCalledTimes(1);
    expect(run.mock.calls[0][0]).toEqual({ ...q('hel'), before: null });
    expect(s.status).toBe('done');
    s.set(q('hel')); // the same query: nothing new
    await vi.advanceTimersByTimeAsync(400);
    expect(run).toHaveBeenCalledTimes(1);
  });
  it('drops an answer to an older query', async () => {
    vi.useFakeTimers();
    let first!: (p: SearchPage) => void;
    const run = vi.fn((x: SearchQuery) => (x.q === 'old' ? new Promise<SearchPage>((r) => (first = r)) : Promise.resolve(page([2]))));
    const s = new Searcher(run);
    s.set(q('old'));
    await vi.advanceTimersByTimeAsync(300);
    s.set(q('new'));
    await vi.advanceTimersByTimeAsync(300);
    first(page([1]));
    await vi.advanceTimersByTimeAsync(0);
    expect(s.hits.map((h) => h.message.id)).toEqual([2]);
  });
  it('says slow on a 429 and tries again a second later', async () => {
    vi.useFakeTimers();
    let n = 0;
    const run = vi.fn(async () => (n++ ? page([1]) : Promise.reject(new ApiFailure(429, 'rate_limited'))));
    const s = new Searcher(run);
    s.set(q('hello'));
    await vi.advanceTimersByTimeAsync(300);
    expect(s.status).toBe('slow');
    await vi.advanceTimersByTimeAsync(999);
    expect(run).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(s.status).toBe('done');
  });
  it('shows other errors, clears on null, and pages with the cursor', async () => {
    vi.useFakeTimers();
    const run = vi.fn(async (x: SearchQuery) => (x.q === 'bad' ? Promise.reject(new ApiFailure(400, 'bad_request')) : x.before ? page([3]) : page([5, 4], 4)));
    const s = new Searcher(run);
    s.set(q('bad'));
    await vi.advanceTimersByTimeAsync(300);
    expect(s.status).toBe('error');
    expect(s.error).toMatch(/did not accept/);
    s.set(q('good'));
    await vi.advanceTimersByTimeAsync(300);
    s.more();
    await vi.advanceTimersByTimeAsync(0);
    expect(run.mock.calls.at(-1)![0].before).toBe(4);
    expect(s.hits.map((h) => h.message.id)).toEqual([5, 4, 3]);
    expect(s.next).toBe(null);
    s.set(null);
    expect([s.status, s.hits.length]).toEqual(['idle', 0]);
  });
});
