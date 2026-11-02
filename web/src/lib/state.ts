import type { Me } from './gen/Me';
import type { Peer } from './gen/Peer';
import type { RoomInfo } from './gen/RoomInfo';
import type { ServerMsg } from './gen/ServerMsg';
import type { Unread } from './gen/Unread';
import type { UserInfo } from './gen/UserInfo';
import { applyPeerMsg } from './peers';

export type AppState = {
  me: Me | null;
  users: Record<string, UserInfo>;
  /** Rooms that are not archived: the sidebar, switcher, search and every other room list. */
  rooms: RoomInfo[];
  /** Archived rooms. The server sends them to the owner only, for Admin > Rooms. */
  archived: RoomInfo[];
  voice: Peer[];
  /** By room id. A room missing here has nothing unread. */
  unread: Record<number, Unread>;
};

export const emptyState = (): AppState => ({ me: null, users: {}, rooms: [], archived: [], voice: [], unread: {} });

/** A room list from the server, split into the two lists of `AppState`. */
export const splitRooms = (list: RoomInfo[]) => ({ rooms: list.filter((r) => !r.archived), archived: list.filter((r) => r.archived) });

export function applyServerMsg(s: AppState, m: ServerMsg): AppState {
  switch (m.type) {
    case 'hello':
      return {
        me: m.me,
        users: Object.fromEntries(m.users.map((u) => [u.user_id, u])),
        ...splitRooms(m.rooms),
        voice: m.voice,
        unread: Object.fromEntries(m.unread.map((u) => [u.room_id, u])),
      };
    case 'message_new': {
      const msg = m.message;
      const u = s.unread[msg.room_id] ?? { room_id: msg.room_id, last_read_id: 0, unread: 0, mentions: 0 };
      const mine = msg.author_id === s.me?.user.user_id;
      if (mine || msg.id <= u.last_read_id) return s;
      const mention = s.me && msg.mentions.includes(s.me.user.user_id) ? 1 : 0;
      return { ...s, unread: { ...s.unread, [msg.room_id]: { ...u, unread: u.unread + 1, mentions: u.mentions + mention } } };
    }
    case 'read':
      // shortcut: a marker older than the newest message clears the count too; we only send the newest id we show.
      return { ...s, unread: { ...s.unread, [m.room_id]: { room_id: m.room_id, last_read_id: m.last_read_id, unread: 0, mentions: 0 } } };
    case 'rooms_changed': {
      // The server sends no peer_left for rooms we can no longer see, so drop their peers here.
      const split = splitRooms(m.rooms);
      const ids = new Set(split.rooms.map((r) => r.id));
      return { ...s, ...split, voice: s.voice.filter((p) => ids.has(p.room_id)) };
    }
    case 'user_created': // a signup: without it they showed as 'Someone' until a reload
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
