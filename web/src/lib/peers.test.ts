import { describe, expect, it } from 'vitest';
import { applyPeerMsg, inRoom } from './peers';
import type { Peer } from './gen/Peer';

const peer = (user_id: string, room_id = 1): Peer => ({
  user_id, name: user_id.toUpperCase(), room_id, muted: false, deafened: false, force_muted: false,
});
const a = peer('a');
const b = peer('b');

describe('applyPeerMsg', () => {
  it('appends; a second join replaces the entry', () => {
    expect(applyPeerMsg([a], { type: 'peer_joined', peer: b })).toEqual([a, b]);
    const moved = { ...a, room_id: 2 };
    expect(applyPeerMsg([a, b], { type: 'peer_joined', peer: moved })).toEqual([moved, b]);
  });
  it('removes on leave', () => {
    expect(applyPeerMsg([a, b], { type: 'peer_left', user_id: 'a' })).toEqual([b]);
  });
  it('peer_state_updates_muted_and_deafened', () => {
    const m = { type: 'peer_state', user_id: 'b', muted: true, deafened: true, force_muted: true } as const;
    expect(applyPeerMsg([a, b], m)).toEqual([a, { ...b, muted: true, deafened: true, force_muted: true }]);
  });
  it('ignores other messages', () => {
    const l = [a];
    expect(applyPeerMsg(l, { type: 'offer', sdp: 'x' })).toBe(l);
  });
});

describe('inRoom', () => {
  it('inRoom_filters_by_room_id', () => {
    const c = peer('c', 2);
    expect(inRoom([a, c, b], 1)).toEqual([a, b]);
    expect(inRoom([a, c, b], 2)).toEqual([c]);
    expect(inRoom([a], 9)).toEqual([]);
  });
});
