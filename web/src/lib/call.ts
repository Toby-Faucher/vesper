import type { ServerMsg } from './gen/ServerMsg';

/** The call's own voice flags. A new call and an ended one both start from `idleCall()`. */
export type CallFlags = { muted: boolean; deafened: boolean; forceMuted: boolean };
// forceMuted resets too: the server sends force_muted again on join if it still holds.
export const idleCall = (): CallFlags => ({ muted: false, deafened: false, forceMuted: false });

export const CALL_ENDED = 'Your call ended. You may have joined from another tab, or a moderator removed you.';
export const ROOM_GONE = 'You no longer have access to that room.';

/**
 * What a socket message does to the call. `userId` and `room` are empty or null when not in a call.
 * `end` is the text to show when the call is over; `leave` says to tell the server too.
 */
export function callEvent(
  m: ServerMsg,
  c: { userId: string; room: number | null; muted: boolean },
): { end?: string; leave?: boolean; forceMuted?: boolean; muted?: boolean } {
  switch (m.type) {
    case 'peer_left':
      // Our own id: another tab took the call, a moderator kicked us, or access was lost.
      return c.userId && m.user_id === c.userId ? { end: CALL_ENDED } : {};
    case 'force_muted':
      return { forceMuted: m.muted, muted: c.muted || m.muted }; // lifting a force-mute does not unmute
    case 'rooms_changed':
      return c.room !== null && !m.rooms.some((r) => r.id === c.room) ? { end: ROOM_GONE, leave: true } : {};
    default:
      return {};
  }
}
