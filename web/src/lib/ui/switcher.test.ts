import { describe, expect, it } from 'vitest';
import type { Unread } from '../gen/Unread';
import { MAX_RESULTS, rank, score, type Item } from './switcher';

const run = () => {};
const room = (id: number, label: string): Item => ({ kind: 'room', id: String(id), label, run });
const person = (id: string, label: string): Item => ({ kind: 'person', id, label, run });
const action = (id: string, label: string): Item => ({ kind: 'action', id, label, run });
const un = (room_id: number, unread: number, mentions = 0): Unread => ({ room_id, last_read_id: 0, unread, mentions });
const ids = (l: Item[]) => l.map((i) => i.id);
const none = { unread: {}, recent: [] };

describe('score', () => {
  it('is null when the letters are not there in order', () => {
    expect(score('xyz', 'general')).toBe(null);
    expect(score('lareneg', 'general')).toBe(null);
  });
  it('ignores case, also outside ASCII', () => {
    expect(score('GEN', 'general')).not.toBe(null);
    expect(score('éTÉ', 'Été plans')).toBe(score('été', 'été plans'));
    expect(score('straße', 'STRASSE')).toBe(null); // no folding beyond lowercase, and no crash
  });
  it('counts a letter outside the BMP once', () => {
    expect(score('🎵m', '🎵 music')).not.toBe(null);
    expect(score('🎵', 'music')).toBe(null);
  });
  it('ranks prefix over word start over inside a word over scattered letters', () => {
    const prefix = score('gen', 'general')!;
    const word = score('gen', 'the general')!;
    const inside = score('gen', 'regenerate')!;
    const scattered = score('gen', 'gaming evening')!;
    expect(prefix).toBeGreaterThan(word);
    expect(word).toBeGreaterThan(inside);
    expect(inside).toBeGreaterThan(scattered);
  });
  it('prefers word starts and runs in a subsequence, and ignores spaces in the query', () => {
    expect(score('ms', 'music stuff')!).toBeGreaterThan(score('ms', 'maps')!);
    expect(score('join gen', 'Join voice in general')).not.toBe(null);
    expect(score('sv', 'Settings: Voice')!).toBeGreaterThan(score('sv', 'saved')!);
  });
  it('prefers the shorter label in the same band', () => {
    expect(score('mu', 'music')!).toBeGreaterThan(score('mu', 'music and more')!);
  });
});

describe('rank', () => {
  const rooms = [room(1, 'general'), room(2, 'music'), room(3, 'lobby'), room(4, 'games')];
  const items = [...rooms, person('a', 'alice'), person('b', 'bob'), action('mute', 'Mute'), action('gen', 'Settings: General')];

  it('empty query: mentions first, then unread by count, then recent, then the given order; people, then actions', () => {
    const unread = { 2: un(2, 1), 3: un(3, 9), 4: un(4, 1, 1) };
    expect(ids(rank('', items, { unread, recent: [] }))).toEqual(['4', '3', '2', '1', 'a', 'b', 'mute', 'gen']);
    expect(ids(rank('', rooms, { unread: {}, recent: [3, 4] }))).toEqual(['3', '4', '1', '2']);
    // Unread beats recent: 1 was opened last but 2 has news.
    expect(ids(rank('', rooms, { unread: { 2: un(2, 1) }, recent: [1] }))).toEqual(['2', '1', '3', '4']);
    // More mentions first.
    expect(ids(rank('', rooms, { unread: { 1: un(1, 1, 1), 3: un(3, 2, 2) }, recent: [] })).slice(0, 2)).toEqual(['3', '1']);
  });

  it('a query sorts by score; unread, then recent break ties', () => {
    const two = [room(1, 'music one'), room(2, 'music two'), room(3, 'music six')];
    expect(ids(rank('mus', two, none))).toEqual(['1', '2', '3']); // equal scores keep the given order
    expect(ids(rank('music', [room(1, 'music a'), room(2, 'music b')], { unread: { 2: un(2, 1) }, recent: [] }))).toEqual(['2', '1']);
    expect(ids(rank('music', [room(1, 'music a'), room(2, 'music b')], { unread: {}, recent: [2] }))).toEqual(['2', '1']);
  });

  it('groups by kind, the group with the best match first', () => {
    const got = rank('gen', items, none);
    expect(ids(got)).toEqual(['1', 'gen']); // games has no n
    expect(got.map((i) => i.kind)).toEqual(['room', 'action']);
    const act = rank('mute', [room(9, 'mute room'), action('mute', 'Mute')], none);
    expect(act.map((i) => i.kind)).toEqual(['action', 'room']);
  });

  it('prefixes scope the list to one kind', () => {
    expect(ids(rank('r ', items, none))).toEqual(['1', '2', '3', '4']);
    expect(ids(rank('p b', items, none))).toEqual(['b']);
    expect(ids(rank('> ', items, none))).toEqual(['mute', 'gen']);
    expect(ids(rank('> gen', items, none))).toEqual(['gen']);
    expect(ids(rank('R mus', items, none))).toEqual(['2']);
    // Without the space it is an ordinary query.
    expect(ids(rank('rm', [room(1, 'rm'), action('x', 'r m')], none))).toEqual(['1', 'x']);
  });

  it('keeps at most 30', () => {
    const many = Array.from({ length: 50 }, (_, i) => room(i + 1, `room ${i + 1}`));
    expect(rank('', many, none)).toHaveLength(MAX_RESULTS);
    expect(rank('room', many, none)).toHaveLength(MAX_RESULTS);
    // The cap keeps the best: the unread one is in.
    expect(ids(rank('', many, { unread: { 50: un(50, 1) }, recent: [] }))[0]).toBe('50');
  });

  it('no match gives an empty list', () => {
    expect(rank('zzz', items, none)).toEqual([]);
  });
});
