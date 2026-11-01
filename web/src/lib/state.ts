import type { Me } from './gen/Me';
import type { Peer } from './gen/Peer';
import type { RoomInfo } from './gen/RoomInfo';
import type { ServerMsg } from './gen/ServerMsg';
import type { UserInfo } from './gen/UserInfo';
import { applyPeerMsg } from './peers';

export type AppState = { me: Me | null; users: Record<string, UserInfo>; rooms: RoomInfo[]; voice: Peer[] };

export const emptyState = (): AppState => ({ me: null, users: {}, rooms: [], voice: [] });

export function applyServerMsg(s: AppState, m: ServerMsg): AppState {
  switch (m.type) {
    case 'hello':
      return { me: m.me, users: Object.fromEntries(m.users.map((u) => [u.user_id, u])), rooms: m.rooms, voice: m.voice };
    case 'rooms_changed': {
      // The server sends no peer_left for rooms we can no longer see, so drop their peers here.
      const ids = new Set(m.rooms.map((r) => r.id));
      return { ...s, rooms: m.rooms, voice: s.voice.filter((p) => ids.has(p.room_id)) };
    }
    case 'user_updated': {
      const u = m.user;
      const me = s.me && s.me.user.user_id === u.user_id ? { ...s.me, user: u } : s.me;
      return { ...s, me, users: { ...s.users, [u.user_id]: u } };
    }
    default: {
      const voice = applyPeerMsg(s.voice, m);
      return voice === s.voice ? s : { ...s, voice };
    }
  }
}
