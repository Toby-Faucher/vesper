import type { Peer } from './gen/Peer';
import type { ServerMsg } from './gen/ServerMsg';

export function applyPeerMsg(peers: Peer[], m: ServerMsg): Peer[] {
  switch (m.type) {
    case 'peer_joined': {
      const id = m.peer.user_id;
      // A user is in one room at a time: a join elsewhere replaces the old entry.
      return peers.some((p) => p.user_id === id) ? peers.map((p) => (p.user_id === id ? m.peer : p)) : [...peers, m.peer];
    }
    case 'peer_left':
      return peers.filter((p) => p.user_id !== m.user_id);
    case 'peer_state':
      return peers.map((p) =>
        p.user_id === m.user_id ? { ...p, muted: m.muted, deafened: m.deafened, force_muted: m.force_muted } : p,
      );
    default:
      return peers;
  }
}

export function inRoom(peers: Peer[], roomId: number): Peer[] {
  return peers.filter((p) => p.room_id === roomId);
}
