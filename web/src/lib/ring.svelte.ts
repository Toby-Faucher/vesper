import type { ServerMsg } from './gen/ServerMsg';

// Incoming DM calls: who is ringing me, by room. The server sends ring_start to my sockets and ring_stop when it ends
// (caller left, someone joined, declined, timed out). A new hello forgets them: the server's timer ends a ring on its
// own, and a ring missed while the socket was down is not worth showing late.
export type Ring = { room_id: number; user_id: string };

export const ring = $state<{ list: Ring[] }>({ list: [] });

export function applyRing(list: Ring[], m: ServerMsg): Ring[] {
  switch (m.type) {
    case 'ring_start':
      return list.some((r) => r.room_id === m.room_id) ? list : [...list, { room_id: m.room_id, user_id: m.user_id }];
    case 'ring_stop':
      return list.filter((r) => r.room_id !== m.room_id);
    case 'hello':
      return list.length ? [] : list;
    default:
      return list;
  }
}

/** Called for every server message. */
export function onRingMsg(m: ServerMsg): void {
  const next = applyRing(ring.list, m);
  if (next !== ring.list) ring.list = next;
}

/** My own join of that room ends its ring here at once, without waiting for the server's ring_stop. */
export const stopRing = (roomId: number) => (ring.list = ring.list.filter((r) => r.room_id !== roomId));

/** The ring sound repeats this often while the prompt shows. */
export const RING_EVERY_MS = 2500;
