import { expect, it } from 'vitest';
import { loadRecent, pushRecent, recentKey } from './recent';

const store = () => {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v), m };
};

it('keeps the last ten, most recent first, without repeats, per account', () => {
  const s = store();
  for (let i = 1; i <= 12; i++) pushRecent(s, 'u1', i);
  pushRecent(s, 'u1', 5);
  expect(loadRecent(s, 'u1')).toEqual([5, 12, 11, 10, 9, 8, 7, 6, 4, 3]);
  expect(loadRecent(s, 'u2')).toEqual([]);
});

it('survives junk and blocked storage', () => {
  const s = store();
  s.m.set(recentKey('u'), '{"no":1}');
  expect(loadRecent(s, 'u')).toEqual([]);
  s.m.set(recentKey('u'), '[1,"x",2.5,3]');
  expect(loadRecent(s, 'u')).toEqual([1, 3]);
  s.m.set(recentKey('u'), 'not json');
  expect(loadRecent(s, 'u')).toEqual([]);
  expect(pushRecent(null, 'u', 4)).toEqual([4]);
  const full = { getItem: () => null, setItem: () => { throw new Error('full'); } };
  expect(pushRecent(full, 'u', 4)).toEqual([4]);
});
