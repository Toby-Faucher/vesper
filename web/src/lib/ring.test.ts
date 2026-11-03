import { describe, expect, it } from 'vitest';
import { applyRing, type Ring } from './ring.svelte';
import type { ServerMsg } from './gen/ServerMsg';

const start = (room_id: number, user_id = 'a'): ServerMsg => ({ type: 'ring_start', room_id, user_id });
const stop = (room_id: number): ServerMsg => ({ type: 'ring_stop', room_id });
const hello = { type: 'hello' } as unknown as ServerMsg;

describe('applyRing', () => {
  it('adds_a_ring_once_per_room', () => {
    const one = applyRing([], start(1));
    expect(one).toEqual([{ room_id: 1, user_id: 'a' }]);
    expect(applyRing(one, start(1, 'b'))).toBe(one);
    expect(applyRing(one, start(2)).map((r) => r.room_id)).toEqual([1, 2]);
  });
  it('stop_removes_only_that_room', () => {
    const list: Ring[] = [{ room_id: 1, user_id: 'a' }, { room_id: 2, user_id: 'b' }];
    expect(applyRing(list, stop(1))).toEqual([{ room_id: 2, user_id: 'b' }]);
    expect(applyRing(list, stop(9)).length).toBe(2);
  });
  it('a_new_hello_forgets_rings_and_other_messages_change_nothing', () => {
    const list: Ring[] = [{ room_id: 1, user_id: 'a' }];
    expect(applyRing(list, { type: 'force_muted', muted: true })).toBe(list);
    expect(applyRing(list, hello)).toEqual([]);
    const empty: Ring[] = [];
    expect(applyRing(empty, hello)).toBe(empty);
  });
});
