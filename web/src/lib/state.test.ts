import { describe, expect, it } from 'vitest';
import { applyServerMsg, emptyState, findRoom, sortDms, type AppState } from './state';
import type { Peer } from './gen/Peer';
import type { RoomInfo } from './gen/RoomInfo';
import type { UserInfo } from './gen/UserInfo';
import type { Message } from './gen/Message';

const user = (user_id: string, display_name = user_id): UserInfo => ({
  user_id, username: user_id, display_name, accent: '#888888', role: 'member', banned: false, bio: '', avatar: null, ui_accent: null,
});
const room = (id: number): RoomInfo => ({ id, kind: 'room', name: `room${id}`, position: id, restricted: false, members: [], archived: false });
const dm = (id: number, a: string, b: string): RoomInfo => ({ id, kind: 'dm', name: `dm-${a}-${b}`, position: 0, restricted: true, members: [a, b], archived: false });
const peer = (user_id: string, room_id: number): Peer => ({
  user_id, name: user_id, room_id, muted: false, deafened: false, force_muted: false, share: null,
});
const server = { version: '1.4.0', build: 'b1' };
const me = { user: user('a'), session_id: 1, server, upload_max: 1 << 20 };
const unread1 = { room_id: 1, last_read_id: 5, unread: 2, mentions: 0 };

describe('applyServerMsg', () => {
  it('hello_replaces_state', () => {
    const old: AppState = { me: null, users: { x: user('x') }, rooms: [room(9)], archived: [], dms: [], voice: [peer('x', 9)], unread: {}, activity: {} };
    const s = applyServerMsg(old, {
      type: 'hello', me, users: [user('a'), user('b')], rooms: [room(1)], voice: [peer('b', 1)], unread: [unread1],
    });
    expect(s).toEqual({ me, users: { a: user('a'), b: user('b') }, rooms: [room(1)], archived: [], dms: [], voice: [peer('b', 1)], unread: { 1: unread1 }, activity: {} });
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
  it('archived rooms go to their own list, and their peers are dropped', () => {
    const gone = { ...room(2), archived: true };
    const s: AppState = { ...emptyState(), rooms: [room(1), room(2)], voice: [peer('a', 1), peer('b', 2)] };
    const next = applyServerMsg(s, { type: 'rooms_changed', rooms: [room(1), gone] });
    expect(next.rooms).toEqual([room(1)]);
    expect(next.archived).toEqual([gone]);
    expect(next.voice).toEqual([peer('a', 1)]);
    const hello = applyServerMsg(s, { type: 'hello', me, users: [], rooms: [gone, room(1)], voice: [], unread: [] });
    expect([hello.rooms, hello.archived]).toEqual([[room(1)], [gone]]);
  });
  it('user_updated_merges', () => {
    const s: AppState = { ...emptyState(), me, users: { a: user('a'), b: user('b') } };
    const other = applyServerMsg(s, { type: 'user_updated', user: user('b', 'Bee') });
    expect(other.users).toEqual({ a: user('a'), b: user('b', 'Bee') });
    expect(other.me).toBe(me);
    const self = applyServerMsg(s, { type: 'user_updated', user: user('a', 'Ay') });
    expect(self.me).toEqual({ user: user('a', 'Ay'), session_id: 1, server, upload_max: 1 << 20 });
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
  it('dms_are_kept_apart_and_named_after_the_other_person', () => {
    const s = applyServerMsg(emptyState(), {
      type: 'hello', me, users: [user('a'), user('b', 'Bea')], rooms: [room(1), dm(7, 'a', 'b')], voice: [], unread: [],
    });
    expect(s.rooms.map((r) => r.id)).toEqual([1]);
    expect(s.dms.map((r) => [r.id, r.name])).toEqual([[7, 'Bea']]);
    expect(findRoom(s, 7)?.name).toBe('Bea');
    expect(findRoom(s, 1)?.name).toBe('room1');
    // A rename shows at once, and a new DM from rooms_changed keeps the voice of both kinds.
    const renamed = applyServerMsg(s, { type: 'user_updated', user: user('b', 'Bee') });
    expect(renamed.dms[0].name).toBe('Bee');
    const withVoice = { ...s, voice: [peer('b', 7), peer('b', 1)] };
    const changed = applyServerMsg(withVoice, { type: 'rooms_changed', rooms: [room(1), dm(7, 'a', 'b'), dm(8, 'a', 'c')] });
    expect(changed.voice.map((p) => p.room_id)).toEqual([7, 1]);
    expect(changed.dms.map((r) => r.name)).toEqual(['Bea', 'Someone']);
  });
});

describe('sortDms', () => {
  const a = { ...dm(1, 'a', 'x'), name: 'Amy' };
  const b = { ...dm(2, 'a', 'y'), name: 'Bea' };
  const c = { ...dm(3, 'a', 'z'), name: 'Cy' };
  const ids = (l: RoomInfo[]) => l.map((r) => r.id);
  it('puts the newest message first, then name when nothing is known', () => {
    expect(ids(sortDms([a, b, c], {}, { 1: 5, 2: 9 }))).toEqual([2, 1, 3]);
    expect(ids(sortDms([c, b, a], {}, {}))).toEqual([1, 2, 3]);
  });
  it('keeps unread DMs above the rest', () => {
    const unread = { 1: { room_id: 1, last_read_id: 0, unread: 1, mentions: 0 } };
    expect(ids(sortDms([b, a], unread, { 2: 9, 1: 2 }))).toEqual([1, 2]);
  });
  it('moves a DM up when a message arrives', () => {
    let s: AppState = { ...emptyState(), me, dms: [a, b], activity: { 1: 5, 2: 9 } };
    s = applyServerMsg(s, { type: 'message_new', message: { id: 12, room_id: 1, author_id: 'a' } as Message });
    expect(ids(sortDms(s.dms, s.unread, s.activity))).toEqual([1, 2]);
  });
});
