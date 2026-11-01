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
const fetchMock = vi.fn(async () => new Response(meStatus === 204 ? null : '{"code":"unauthorized"}', { status: meStatus }));

async function load() {
  vi.resetModules();
  FakeWS.all = [];
  vi.stubGlobal('WebSocket', FakeWS);
  vi.stubGlobal('location', { pathname: '/', protocol: 'http:', host: 'x' });
  vi.stubGlobal('history', { pushState: (_: unknown, __: string, p: string) => ((location as { pathname: string }).pathname = p) });
  vi.stubGlobal('fetch', fetchMock);
  return import('./session.svelte');
}

beforeEach(() => {
  vi.useFakeTimers();
  meStatus = 200;
  fetchMock.mockClear();
});
afterEach(() => {
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
  it('other closes reconnect after 1 s, 2 s, 5 s, 10 s, 10 s; hello resets it', async () => {
    const { session } = await load();
    session.connect();
    for (const ms of [1000, 2000, 5000, 10000, 10000]) {
      const n = FakeWS.all.length;
      FakeWS.all[n - 1].drop(1006);
      await vi.advanceTimersByTimeAsync(ms - 1);
      expect(FakeWS.all).toHaveLength(n);
      await vi.advanceTimersByTimeAsync(1);
      expect(FakeWS.all).toHaveLength(n + 1);
    }
    expect(fetchMock).toHaveBeenCalledTimes(5); // asks /api/me before each new socket
    FakeWS.all.at(-1)!.hello();
    FakeWS.all.at(-1)!.drop(1013);
    await vi.advanceTimersByTimeAsync(1000);
    expect(FakeWS.all).toHaveLength(7);
  });
  it('a drop while signed out goes to sign in', async () => {
    const { session, route } = await load();
    session.connect();
    meStatus = 401;
    FakeWS.all[0].drop(1006);
    await vi.advanceTimersByTimeAsync(1000);
    expect(route.path).toBe('/login');
    expect(FakeWS.all).toHaveLength(1);
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
});
