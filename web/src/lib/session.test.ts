import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

class FakeWS {
  static all: FakeWS[] = [];
  readyState = 1;
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  onclose: ((e: { code: number }) => void) | null = null;
  constructor(public url: string) { FakeWS.all.push(this); }
  send() {}
  close() { this.onclose?.({ code: 1000 }); }
  drop(code: number) { this.onclose?.({ code }); }
  hello() {
    const me = { user: { user_id: 'a', username: 'a', display_name: 'A', accent: '#888888', role: 'member', banned: false }, session_id: 1 };
    this.onmessage?.({ data: JSON.stringify({ type: 'hello', me, users: [me.user], rooms: [], voice: [], unread: [] }) });
  }
}

let meStatus = 200;
let calls: string[] = []; // history calls
const fetchMock = vi.fn(async () => new Response(meStatus === 204 ? null : '{"code":"unauthorized"}', { status: meStatus }));

async function load() {
  vi.resetModules();
  FakeWS.all = [];
  vi.stubGlobal('WebSocket', FakeWS);
  vi.stubGlobal('location', { pathname: '/', search: '', hash: '', protocol: 'http:', host: 'x' });
  const go = (kind: string) => (_: unknown, __: string, p: string) => {
    calls.push(`${kind} ${p}`);
    Object.assign(location, { pathname: p, hash: '' });
  };
  calls = [];
  vi.stubGlobal('history', { pushState: go('push'), replaceState: go('replace') });
  vi.stubGlobal('fetch', fetchMock);
  return import('./session.svelte');
}

beforeEach(() => {
  vi.useFakeTimers();
  meStatus = 200;
  fetchMock.mockClear();
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe('session', () => {
  it('opens one socket with the cookie (no key in the URL) and reads hello', async () => {
    const { session } = await load();
    session.connect();
    session.connect();
    expect(FakeWS.all).toHaveLength(1);
    expect(FakeWS.all[0].url).toBe('ws://x/ws');
    FakeWS.all[0].hello();
    expect(session.online).toBe(true);
    expect(session.state.me?.user.display_name).toBe('A');
  });
  it('4001 goes to sign in and clears the state', async () => {
    const { session, route } = await load();
    session.connect();
    FakeWS.all[0].hello();
    FakeWS.all[0].drop(4001);
    expect(route.path).toBe('/login');
    expect(session.state.me).toBeNull();
    expect(session.notice).toMatch(/session ended/);
    await vi.advanceTimersByTimeAsync(30000);
    expect(FakeWS.all).toHaveLength(1);
  });
  it('a session that ends on a room page signs in back to that room', async () => {
    const { session, route } = await load();
    Object.assign(location, { pathname: '/r/12' });
    session.connect();
    FakeWS.all[0].drop(4001);
    expect(route.path).toBe('/login');
    expect(calls.at(-1)).toBe('replace /login?next=%2Fr%2F12');
  });
  it('4003 says a moderator disconnected you and does not reconnect', async () => {
    const { session, route } = await load();
    session.connect();
    FakeWS.all[0].drop(4003);
    expect(session.notice).toBe('A moderator disconnected you.');
    expect(route.path).toBe('/');
    await vi.advanceTimersByTimeAsync(30000);
    expect(FakeWS.all).toHaveLength(1);
    session.connect(); // the Reconnect button
    expect(FakeWS.all).toHaveLength(2);
  });
  it('backoff doubles from 1 to 2 s up to 15 to 30 s, with jitter', async () => {
    const { backoffMs } = await load();
    vi.spyOn(Math, 'random').mockReturnValue(0);
    expect([0, 1, 2, 3, 4, 9].map(backoffMs)).toEqual([1000, 2000, 4000, 8000, 15000, 15000]);
    vi.spyOn(Math, 'random').mockReturnValue(0.999999);
    expect(Math.round(backoffMs(0))).toBe(2000);
    expect(Math.round(backoffMs(9))).toBe(30000);
  });
  it('other closes reconnect with growing waits; hello alone does not reset them', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { session } = await load();
    session.connect();
    const drop = async (ms: number) => {
      const n = FakeWS.all.length;
      FakeWS.all[n - 1].hello(); // accepted, then dropped at once
      FakeWS.all[n - 1].drop(1006);
      await vi.advanceTimersByTimeAsync(ms - 1);
      expect(FakeWS.all).toHaveLength(n);
      await vi.advanceTimersByTimeAsync(1);
      expect(FakeWS.all).toHaveLength(n + 1);
    };
    for (const ms of [1000, 2000, 4000, 8000, 15000, 15000]) await drop(ms);
    expect(fetchMock).toHaveBeenCalledTimes(6); // asks /api/me before each new socket
    // Up for 10 s before the drop: the next wait starts at 1 s again.
    FakeWS.all.at(-1)!.hello();
    await vi.advanceTimersByTimeAsync(10000);
    FakeWS.all.at(-1)!.drop(1013);
    await vi.advanceTimersByTimeAsync(1000);
    expect(FakeWS.all).toHaveLength(8);
  });
  it('logout while a retry asks /api/me opens no socket afterwards', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { session } = await load();
    session.connect();
    let answer!: (r: Response) => void;
    fetchMock.mockImplementationOnce(() => new Promise<Response>((r) => (answer = r)));
    FakeWS.all[0].drop(1006);
    await vi.advanceTimersByTimeAsync(1000); // the /api/me call is in flight
    const out = session.logout();
    answer(new Response(null, { status: 204 }));
    await out;
    await vi.advanceTimersByTimeAsync(30000);
    expect(FakeWS.all).toHaveLength(1);
  });
  it('redirects to sign in replace the history entry', async () => {
    const { session } = await load();
    session.connect();
    FakeWS.all[0].drop(4001);
    expect(calls).toEqual(['replace /login']);
  });
  it('takeInvite returns the code and clears the fragment', async () => {
    const { takeInvite } = await load();
    Object.assign(location, { pathname: '/invite', search: '?a=1', hash: '#' + 'ab'.repeat(32) });
    expect(takeInvite()).toBe('ab'.repeat(32));
    expect(calls).toEqual(['replace /invite?a=1']);
    expect(location.hash).toBe('');
  });
  it('a drop while signed out goes to sign in', async () => {
    const { session, route } = await load();
    session.connect();
    meStatus = 401;
    FakeWS.all[0].drop(1006);
    await vi.advanceTimersByTimeAsync(2000);
    expect(route.path).toBe('/login');
    expect(FakeWS.all).toHaveLength(1);
  });
  it('retryAt says when the next try starts; retryNow skips the wait once', async () => {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    vi.setSystemTime(100000);
    const { session } = await load();
    session.connect();
    expect(session.retryAt).toBeNull();
    FakeWS.all[0].drop(1006);
    expect(session.retryAt).toBe(101000);
    session.retryNow();
    expect(session.retryAt).toBeNull();
    session.retryNow(); // the try is already running: no second /api/me
    await vi.advanceTimersByTimeAsync(0);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(FakeWS.all).toHaveLength(2);
    await vi.advanceTimersByTimeAsync(30000); // the cleared timer does not fire a second socket
    expect(FakeWS.all).toHaveLength(2);
    FakeWS.all[1].drop(1006);
    expect(session.retryAt).toBe(Date.now() + 2000);
    await session.logout();
    expect(session.retryAt).toBeNull();
  });
  it('logout and a lost session (4001) stop and clear the uploads of every room', async () => {
    for (const end of ['logout', '4001']) {
      const { session } = await load();
      const { tray } = await import('./ui/tray.svelte');
      const ctrls = [1, 2].map((room) => {
        const ctrl = new AbortController();
        tray.items.push({ key: room, room, file: {} as File, state: 'uploading', progress: 0, error: '', retry: true, attachment: null, ctrl });
        return ctrl;
      });
      session.connect();
      if (end === 'logout') {
        meStatus = 204;
        await session.logout();
      } else FakeWS.all[0].drop(4001);
      expect(ctrls.map((c) => c.signal.aborted)).toEqual([true, true]);
      expect(tray.items).toEqual([]);
    }
  });
  it('logout posts, closes the socket and stops reconnecting', async () => {
    const { session, route } = await load();
    session.connect();
    meStatus = 204;
    await session.logout();
    expect(fetchMock).toHaveBeenCalledWith('/api/logout', expect.objectContaining({ method: 'POST' }));
    expect(route.path).toBe('/login');
    expect(session.sig).toBeNull();
    await vi.advanceTimersByTimeAsync(30000);
    expect(FakeWS.all).toHaveLength(1);
  });
  it('logout forgets the last and recent rooms of this account only', async () => {
    const store = new Map([['vesper.lastRoom.a', '5'], ['vesper.lastRoom.b', '7'], ['vesper.recent.a', '[5]'], ['vesper.recent.b', '[7]']]);
    vi.stubGlobal('localStorage', { getItem: (k: string) => store.get(k) ?? null, removeItem: (k: string) => store.delete(k) });
    const { session, lastRoomKey } = await load();
    session.connect();
    FakeWS.all[0].hello(); // user a
    meStatus = 204;
    await session.logout();
    expect(store.has(lastRoomKey('a'))).toBe(false);
    expect(store.get(lastRoomKey('b'))).toBe('7');
    expect([...store.keys()]).toEqual(['vesper.lastRoom.b', 'vesper.recent.b']);
    expect(calls.at(-1)).toBe('replace /login'); // signing out on purpose keeps no page to return to
  });
});

describe('navigate and closePage', () => {
  // A history with entries and state, enough for push, replace and back.
  async function nav(start: string, startState: unknown = null) {
    const mod = await load();
    const h = { entries: [{ path: start, state: startState }], i: 0 };
    const at = (p: string) => Object.assign(location, { pathname: p, search: '', hash: '' });
    at(start);
    vi.stubGlobal('history', {
      get state() { return h.entries[h.i].state; },
      get length() { return h.entries.length; },
      pushState(state: unknown, _: string, p: string) { h.entries.splice(++h.i, Infinity, { path: p, state }); at(p); },
      replaceState(state: unknown, _: string, p: string) { h.entries[h.i] = { path: p, state }; at(p); },
      back() { at(h.entries[--h.i].path); },
    });
    return { ...mod, paths: () => h.entries.map((e) => e.path), h };
  }

  it('a click in the app pushes an entry marked app; a deep link is not', async () => {
    const { navigate, cameInApp } = await nav('/r/1');
    expect(cameInApp()).toBe(false);
    navigate('/r/2');
    expect(cameInApp()).toBe(true);
  });

  it('moves inside Settings replace and keep the mark, so Close goes Back to the room', async () => {
    const { navigate, closePage, paths, route } = await nav('/r/1');
    navigate('/settings/voice'); // the sidebar button
    navigate('/settings/diagnostics'); // a section link or Call details
    navigate('/settings/voice'); // the sidebar button again, already in Settings
    expect(paths()).toEqual(['/r/1', '/settings/voice']);
    closePage();
    expect(location.pathname).toBe('/r/1');
    navigate('/admin/invites');
    navigate('/admin/members');
    navigate('/settings/voice'); // another page: a new entry
    expect(paths()).toEqual(['/r/1', '/admin/members', '/settings/voice']);
    expect(route.path).toBe('/settings/voice');
  });

  it('closePage after a deep link replaces with / and adds no entry', async () => {
    const { closePage, paths } = await nav('/settings/voice');
    closePage();
    expect(paths()).toEqual(['/']);
  });
});
