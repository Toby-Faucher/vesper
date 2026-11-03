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
  /** Your direct messages. Each one's `name` is the other person's display name, set here. */
  dms: RoomInfo[];
  voice: Peer[];
  /** By room id. A room missing here has nothing unread. */
  unread: Record<number, Unread>;
};

export const emptyState = (): AppState => ({ me: null, users: {}, rooms: [], archived: [], dms: [], voice: [], unread: {} });

/** The other person of a DM. */
export const dmPeer = (r: RoomInfo, myId: string) => r.members.find((m) => m !== myId) ?? myId;

/** DMs named after the other person (the stored name is generated), so every place that shows a room name shows theirs. */
export const titled = (dms: RoomInfo[], users: Record<string, UserInfo>, myId: string): RoomInfo[] =>
  dms.map((r) => ({ ...r, name: users[dmPeer(r, myId)]?.display_name ?? 'Someone' }));

/** A room list from the server, split into the three lists of `AppState`. */
export const splitRooms = (list: RoomInfo[], users: Record<string, UserInfo> = {}, myId = '') => ({
  rooms: list.filter((r) => r.kind === 'room' && !r.archived),
  archived: list.filter((r) => r.kind === 'room' && r.archived),
  dms: titled(list.filter((r) => r.kind === 'dm'), users, myId),
});

/** A room or a DM by id. */
export const findRoom = (s: AppState, id: number | null | undefined) => (id == null ? undefined : s.rooms.find((r) => r.id === id) ?? s.dms.find((r) => r.id === id));

export function applyServerMsg(s: AppState, m: ServerMsg): AppState {
  switch (m.type) {
    case 'hello': {
      const users = Object.fromEntries(m.users.map((u) => [u.user_id, u]));
      return {
        me: m.me,
        users,
        ...splitRooms(m.rooms, users, m.me.user.user_id),
        voice: m.voice,
        unread: Object.fromEntries(m.unread.map((u) => [u.room_id, u])),
      };
    }
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
      const split = splitRooms(m.rooms, s.users, s.me?.user.user_id ?? '');
      const ids = new Set([...split.rooms, ...split.dms].map((r) => r.id));
      return { ...s, ...split, voice: s.voice.filter((p) => ids.has(p.room_id)) };
    }
    case 'user_created': // a signup: without it they showed as 'Someone' until a reload
    case 'user_updated': {
      const u = m.user;
      const me = s.me && s.me.user.user_id === u.user_id ? { ...s.me, user: u } : s.me;
      const users = { ...s.users, [u.user_id]: u };
      return { ...s, me, users, dms: titled(s.dms, users, s.me?.user.user_id ?? '') };
    }
    default: {
      const voice = applyPeerMsg(s.voice, m);
      return voice === s.voice ? s : { ...s, voice };
    }
  }
}
