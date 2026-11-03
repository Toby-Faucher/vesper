import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '../gen/Message';
import type { ServerMsg } from '../gen/ServerMsg';
import { applyServerMsg, emptyState } from '../state';

// notify.ts reaches the session (location) and reads localStorage, neither of which node has.
const store = new Map<string, string>();
vi.stubGlobal('location', { pathname: '/', search: '', hash: '', protocol: 'http:', host: 'x' });
vi.stubGlobal('localStorage', {
  getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v), clear: () => store.clear(),
});
const { DEFAULTS, NOTIFY_KEY, loadPrefs, mentionCount, savePrefs, shouldChime, shouldNotify, titleFor } = await import('./notify');

const msg = (o: Partial<Message> = {}): Message => ({
  id: 5, room_id: 2, author_id: 'b', body: 'hi @alice', created_at: 0, edited_at: null, updated_at: 0, deleted: false,
  reply_to: null, reactions: [], attachments: [], mentions: ['a'], pinned: false, ...o,
});
const on = { mentions: true, dms: true, sounds: true, call: true, muteCues: true, connectCues: true, volume: 50 };
const nctx = { myId: 'a', activeRoom: 1, hidden: false, prefs: on, permission: 'granted' as NotificationPermission };

describe('shouldNotify', () => {
  it('fires for a mention of me in another room', () => expect(shouldNotify(msg(), nctx)).toBe(true));
  it('fires in the open room when the tab is hidden', () => expect(shouldNotify(msg(), { ...nctx, activeRoom: 2, hidden: true })).toBe(true));
  it('any message in a DM fires, without a mention, but not my own or in the room I am looking at', () => {
    const dm = { ...nctx, dm: true };
    expect(shouldNotify(msg({ mentions: [] }), dm)).toBe(true);
    expect(shouldNotify(msg({ mentions: [], author_id: 'a' }), dm)).toBe(false);
    expect(shouldNotify(msg({ mentions: [] }), { ...dm, activeRoom: 2 })).toBe(false);
    expect(shouldNotify(msg({ mentions: [] }), nctx)).toBe(false);
    expect(shouldNotify(msg({ mentions: [] }), { ...dm, prefs: { ...on, dms: false } })).toBe(false);
    expect(shouldNotify(msg({ mentions: [] }), { ...dm, prefs: { ...on, mentions: false } })).toBe(true);
  });
  it('each condition alone turns it off', () => {
    expect(shouldNotify(msg({ mentions: [] }), nctx)).toBe(false);
    expect(shouldNotify(msg({ mentions: ['c'] }), nctx)).toBe(false);
    expect(shouldNotify(msg({ author_id: 'a' }), nctx)).toBe(false);
    expect(shouldNotify(msg({ deleted: true }), nctx)).toBe(false);
    expect(shouldNotify(msg(), { ...nctx, prefs: { ...on, mentions: false } })).toBe(false);
    expect(shouldNotify(msg(), { ...nctx, permission: 'default' })).toBe(false);
    expect(shouldNotify(msg(), { ...nctx, permission: 'denied' })).toBe(false);
    expect(shouldNotify(msg(), { ...nctx, activeRoom: 2 })).toBe(false); // looking at that room
  });
});

describe('shouldChime', () => {
  const peer = (user_id: string, room_id: number) => ({ user_id, name: user_id, room_id, muted: false, deafened: false, force_muted: false });
  const joined = (id: string, room: number): ServerMsg => ({ type: 'peer_joined', peer: peer(id, room) });
  const left: ServerMsg = { type: 'peer_left', user_id: 'b' };
  const c = { joinedRoom: 3, myId: 'a', online: true, sinceHelloMs: 60000, prefs: on, leftRoom: 3 };
  it('joins and leaves in my call room', () => {
    expect(shouldChime(joined('b', 3), c)).toBe('join');
    expect(shouldChime(left, c)).toBe('leave');
  });
  it('is quiet right after a hello (a reconnect)', () => {
    expect(shouldChime(joined('b', 3), { ...c, sinceHelloMs: 2999 })).toBe(null);
    expect(shouldChime(left, { ...c, sinceHelloMs: 100 })).toBe(null);
  });
  it('is quiet for another room, for me, offline, not in a call, and with sounds off', () => {
    expect(shouldChime(joined('b', 4), c)).toBe(null);
    expect(shouldChime(left, { ...c, leftRoom: 4 })).toBe(null);
    expect(shouldChime(joined('a', 3), c)).toBe(null);
    expect(shouldChime({ type: 'peer_left', user_id: 'a' }, c)).toBe(null);
    expect(shouldChime(joined('b', 3), { ...c, online: false })).toBe(null);
    expect(shouldChime(joined('b', 3), { ...c, joinedRoom: null })).toBe(null);
    expect(shouldChime(joined('b', 3), { ...c, prefs: { ...on, sounds: false } })).toBe(null);
    expect(shouldChime(joined('b', 3), { ...c, prefs: { ...on, call: false } })).toBe(null); // the master switch
    expect(shouldChime(joined('b', 3), { ...c, deafened: true })).toBe(null);
    expect(shouldChime({ type: 'typing', room_id: 3, user_id: 'b' }, c)).toBe(null);
  });
});

describe('title count', () => {
  it('titleFor', () => {
    expect(titleFor(0)).toBe('Vesper');
    expect(titleFor(2)).toBe('(2) Vesper');
  });
  it('sums mentions across rooms and drops when a room is read', () => {
    const me = { user: { user_id: 'a', username: 'a', display_name: 'A', accent: '#888888', role: 'member', banned: false }, session_id: 1 };
    let s = applyServerMsg(emptyState(), { type: 'hello', me, users: [me.user], rooms: [], voice: [], unread: [] } as unknown as ServerMsg);
    s = applyServerMsg(s, { type: 'message_new', message: msg({ id: 1, room_id: 1 }) });
    s = applyServerMsg(s, { type: 'message_new', message: msg({ id: 2, room_id: 2 }) });
    s = applyServerMsg(s, { type: 'message_new', message: msg({ id: 3, room_id: 2, mentions: [] }) });
    expect(titleFor(mentionCount(s))).toBe('(2) Vesper');
    s = applyServerMsg(s, { type: 'read', room_id: 2, last_read_id: 3 });
    expect(titleFor(mentionCount(s))).toBe('(1) Vesper');
    s = applyServerMsg(s, { type: 'read', room_id: 1, last_read_id: 1 });
    expect(titleFor(mentionCount(s))).toBe('Vesper');
  });
});

describe('prefs', () => {
  beforeEach(() => localStorage.clear());
  it('defaults: mentions and join/leave off, my call cues on at volume 50; saved', () => {
    expect(loadPrefs()).toEqual({ mentions: false, dms: false, sounds: false, call: true, muteCues: true, connectCues: true, volume: 50 });
    savePrefs({ ...DEFAULTS, mentions: true, connectCues: false, volume: 20 });
    expect(loadPrefs()).toEqual({ ...DEFAULTS, mentions: true, connectCues: false, volume: 20 });
    localStorage.setItem(NOTIFY_KEY, 'not json');
    expect(loadPrefs()).toEqual(DEFAULTS);
  });
  it('an old saved value keeps its join and leave choice', () => {
    localStorage.setItem(NOTIFY_KEY, JSON.stringify({ mentions: true, sounds: true }));
    expect(loadPrefs()).toEqual({ ...DEFAULTS, mentions: true, dms: true, sounds: true });
    localStorage.setItem(NOTIFY_KEY, JSON.stringify({ mentions: true, dms: false }));
    expect(loadPrefs().dms).toBe(false); // once set, dms stands alone
  });
  it('a bad volume falls back or is clamped', () => {
    for (const [v, want] of [['70', 50], [null, 50], [-5, 0], [140, 100], [33.4, 33]] as const) {
      localStorage.setItem(NOTIFY_KEY, JSON.stringify({ volume: v }));
      expect(loadPrefs().volume).toBe(want);
    }
  });
});
