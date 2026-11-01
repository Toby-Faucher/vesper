import { describe, expect, it } from 'vitest';
import { applyServerMsg, emptyState, type AppState } from './state';
import type { Peer } from './gen/Peer';
import type { RoomInfo } from './gen/RoomInfo';
import type { UserInfo } from './gen/UserInfo';
import type { Message } from './gen/Message';

const user = (user_id: string, display_name = user_id): UserInfo => ({
  user_id, username: user_id, display_name, accent: '#888888', role: 'member', banned: false, bio: '', avatar: null,
});
const room = (id: number): RoomInfo => ({ id, name: `room${id}`, position: id, restricted: false, members: [] });
const peer = (user_id: string, room_id: number): Peer => ({
  user_id, name: user_id, room_id, muted: false, deafened: false, force_muted: false,
});
const me = { user: user('a'), session_id: 1 };
const unread1 = { room_id: 1, last_read_id: 5, unread: 2, mentions: 0 };

describe('applyServerMsg', () => {
  it('hello_replaces_state', () => {
    const old: AppState = { me: null, users: { x: user('x') }, rooms: [room(9)], voice: [peer('x', 9)], unread: {} };
    const s = applyServerMsg(old, {
      type: 'hello', me, users: [user('a'), user('b')], rooms: [room(1)], voice: [peer('b', 1)], unread: [unread1],
    });
    expect(s).toEqual({ me, users: { a: user('a'), b: user('b') }, rooms: [room(1)], voice: [peer('b', 1)], unread: { 1: unread1 } });
  });
  it('rooms_changed_keeps_voice', () => {
    const s: AppState = { ...emptyState(), rooms: [room(1), room(2)], voice: [peer('a', 1), peer('b', 2)] };
    const next = applyServerMsg(s, { type: 'rooms_changed', rooms: [room(2), room(1), room(3)] });
    expect(next.rooms.map((r) => r.id)).toEqual([2, 1, 3]);
    expect(next.voice).toEqual(s.voice);
  });
  it('rooms_changed drops peers of rooms that disappeared', () => {
    const s: AppState = { ...emptyState(), rooms: [room(1), room(2)], voice: [peer('a', 1), peer('b', 2)] };
    expect(applyServerMsg(s, { type: 'rooms_changed', rooms: [room(1)] }).voice).toEqual([peer('a', 1)]);
  });
  it('user_updated_merges', () => {
    const s: AppState = { ...emptyState(), me, users: { a: user('a'), b: user('b') } };
    const other = applyServerMsg(s, { type: 'user_updated', user: user('b', 'Bee') });
    expect(other.users).toEqual({ a: user('a'), b: user('b', 'Bee') });
    expect(other.me).toBe(me);
    const self = applyServerMsg(s, { type: 'user_updated', user: user('a', 'Ay') });
    expect(self.me).toEqual({ user: user('a', 'Ay'), session_id: 1 });
    expect(applyServerMsg(s, { type: 'user_updated', user: user('c') }).users.c).toEqual(user('c'));
  });
  it('user_created adds the new person', () => {
    const s: AppState = { ...emptyState(), me, users: { a: user('a') } };
    const next = applyServerMsg(s, { type: 'user_created', user: user('n', 'Newbie') });
    expect(next.users).toEqual({ a: user('a'), n: user('n', 'Newbie') });
    expect(next.me).toBe(me);
  });
  it('voice messages go through applyPeerMsg; others keep the state', () => {
    const s: AppState = { ...emptyState(), voice: [peer('a', 1)] };
    expect(applyServerMsg(s, { type: 'peer_left', user_id: 'a' }).voice).toEqual([]);
    expect(applyServerMsg(s, { type: 'offer', sdp: 'x' })).toBe(s);
  });
  it('unread counts new messages from others and mentions, and read clears it', () => {
    const msg = (id: number, author_id: string, mentions: string[] = []): Message => ({
      id, room_id: 1, author_id, body: 'x', created_at: 0, edited_at: null, updated_at: 0, deleted: false, reply_to: null,
      reactions: [], attachments: [], mentions, pinned: false,
    });
    let s: AppState = { ...emptyState(), me, unread: { 1: unread1 } };
    s = applyServerMsg(s, { type: 'message_new', message: msg(6, 'b') });
    s = applyServerMsg(s, { type: 'message_new', message: msg(7, 'b', ['a']) });
    s = applyServerMsg(s, { type: 'message_new', message: msg(8, 'a') }); // mine
    s = applyServerMsg(s, { type: 'message_new', message: msg(4, 'b') }); // already read
    expect(s.unread[1]).toEqual({ room_id: 1, last_read_id: 5, unread: 4, mentions: 1 });
    s = applyServerMsg(s, { type: 'message_new', message: { ...msg(9, 'b'), room_id: 2 } });
    expect(s.unread[2]).toEqual({ room_id: 2, last_read_id: 0, unread: 1, mentions: 0 });
    s = applyServerMsg(s, { type: 'read', room_id: 1, last_read_id: 8 });
    expect(s.unread[1]).toEqual({ room_id: 1, last_read_id: 8, unread: 0, mentions: 0 });
  });
});
