import type { Peer } from './gen/Peer';
import type { ServerMsg } from './gen/ServerMsg';

export function applyPeerMsg(peers: Peer[], m: ServerMsg): Peer[] {
  switch (m.type) {
    case 'peer_joined':
      return peers.some((p) => p.user_id === m.peer.user_id) ? peers : [...peers, m.peer];
    case 'peer_left':
      return peers.filter((p) => p.user_id !== m.user_id);
    case 'peer_muted':
      return peers.map((p) => (p.user_id === m.user_id ? { ...p, muted: m.muted } : p));
    default:
      return peers;
  }
}
