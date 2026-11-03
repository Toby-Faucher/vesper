import { describe, expect, it } from 'vitest';
import { CALL_ENDED, ROOM_GONE, callEvent, idleCall } from './call';

const me = 'a'.repeat(32);
const inCall = { userId: me, room: 1, muted: false };
const room = (id: number) => ({ id, kind: 'room' as const, name: 'r', position: 0, restricted: false, members: [], archived: false });

describe('callEvent', () => {
  it('peer_left with our own id ends the call; another id does not', () => {
    expect(callEvent({ type: 'peer_left', user_id: me }, inCall)).toEqual({ end: CALL_ENDED });
    expect(callEvent({ type: 'peer_left', user_id: 'b'.repeat(32) }, inCall)).toEqual({});
    expect(callEvent({ type: 'peer_left', user_id: me }, { userId: '', room: null, muted: false })).toEqual({});
  });
  it('force_muted mutes and disables Mute; lifting it does not unmute', () => {
    expect(callEvent({ type: 'force_muted', muted: true }, inCall)).toEqual({ forceMuted: true, muted: true });
    expect(callEvent({ type: 'force_muted', muted: false }, { ...inCall, muted: true })).toEqual({ forceMuted: false, muted: true });
  });
  it('a room that left the list ends the call and tells the server', () => {
    expect(callEvent({ type: 'rooms_changed', rooms: [room(2)] }, inCall)).toEqual({ end: ROOM_GONE, leave: true });
    expect(callEvent({ type: 'rooms_changed', rooms: [room(1)] }, inCall)).toEqual({});
    // The owner still gets an archived room, but the call in it is over.
    expect(callEvent({ type: 'rooms_changed', rooms: [{ ...room(1), archived: true }] }, inCall)).toEqual({ end: ROOM_GONE, leave: true });
    expect(callEvent({ type: 'rooms_changed', rooms: [] }, { userId: '', room: null, muted: false })).toEqual({});
  });
  it('an ended call is not force-muted on rejoin until the server says so', () => {
    let flags = { ...idleCall(), ...callEvent({ type: 'force_muted', muted: true }, inCall) };
    expect(flags.forceMuted).toBe(true);
    flags = idleCall(); // what teardown() and join() set
    expect(flags).toEqual({ muted: false, deafened: false, forceMuted: false });
  });
});
