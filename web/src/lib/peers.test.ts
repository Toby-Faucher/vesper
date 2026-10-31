import { describe, expect, it } from 'vitest';
import { applyPeerMsg } from './peers';

const a = { user_id: 'a', name: 'A', muted: false };
const b = { user_id: 'b', name: 'B', muted: false };

describe('applyPeerMsg', () => {
  it('appends without duplicating', () => {
    expect(applyPeerMsg([a], { type: 'peer_joined', peer: b })).toEqual([a, b]);
    expect(applyPeerMsg([a], { type: 'peer_joined', peer: a })).toEqual([a]);
  });
  it('removes on leave', () => {
    expect(applyPeerMsg([a, b], { type: 'peer_left', user_id: 'a' })).toEqual([b]);
  });
  it('updates muted', () => {
    expect(applyPeerMsg([a, b], { type: 'peer_muted', user_id: 'b', muted: true })).toEqual([
      a, { ...b, muted: true },
    ]);
  });
  it('ignores other messages', () => {
    const l = [a];
    expect(applyPeerMsg(l, { type: 'offer', sdp: 'x' })).toBe(l);
  });
});
