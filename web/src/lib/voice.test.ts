import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import type { ServerMsg } from './gen/ServerMsg';

// Fakes for everything that touches audio, WebRTC and the browser. `calls` records the order of the join's steps.
const calls: string[] = [];
const h = vi.hoisted(() => ({
  capture: null as null | { resolve: (c: unknown) => void; reject: (e: unknown) => void },
  playbacks: [] as { close: ReturnType<typeof vi.fn> }[],
}));
vi.mock('./playback', () => ({
  Playback: class {
    constructor() {
      calls.push('playback'); // the real one creates and resumes its AudioContext here
      h.playbacks.push(this as never);
    }
    close = vi.fn();
    setSink = vi.fn(async () => true);
    addLocal = vi.fn();
    addRemote = vi.fn();
    remove = vi.fn();
    setVolume = vi.fn();
    setDeafened = vi.fn();
    speaking = () => new Set<string>();
  },
}));
vi.mock('./audio/capture', () => ({
  createCapture: vi.fn(() => {
    calls.push('capture'); // the real one creates its AudioContext before its first await
    return new Promise((resolve, reject) => (h.capture = { resolve, reject }));
  }),
}));
const joinRoom = vi.fn();
vi.mock('./signaling', () => ({ joinRoom: (...a: unknown[]) => joinRoom(...a) }));

// Counts listeners on window, document and mediaDevices.
class Target {
  n = 0;
  last: (() => void) | undefined; // the newest listener, to fire a devicechange
  addEventListener(_t: string, fn: () => void) { this.n++; this.last = fn; }
  removeEventListener() { this.n--; }
}
const win = new Target();
const doc = Object.assign(new Target(), { visibilityState: 'visible' });
let devices: MediaDeviceInfo[] = [];
const md = Object.assign(new Target(), { enumerateDevices: async () => devices });
vi.stubGlobal('window', win);
vi.stubGlobal('document', doc);
vi.stubGlobal('navigator', { mediaDevices: md, locks: { request: vi.fn() } });
const stored = new Map<string, string>();
vi.stubGlobal('localStorage', { getItem: (k: string) => stored.get(k) ?? null, setItem: () => {}, removeItem: () => {} });
vi.stubGlobal('location', { pathname: '/', search: '', hash: '', protocol: 'http:', host: 'x' });

const { session } = await import('./session.svelte');
const v = await import('./voice.svelte');
const { voice } = v;
const { CALL_ENDED, ROOM_GONE } = await import('./call');
const { SETTINGS_KEY } = await import('./audio/settings');

class FakeSig {
  msg = new Set<(m: ServerMsg) => void>();
  close = new Set<() => void>();
  sent: unknown[] = [];
  leaves = 0;
  onMessage(cb: (m: ServerMsg) => void) { this.msg.add(cb); return () => void this.msg.delete(cb); }
  onClose(cb: () => void) { this.close.add(cb); return () => void this.close.delete(cb); }
  send(m: unknown) { this.sent.push(m); }
  leave() { this.leaves++; }
  emit(m: ServerMsg) { [...this.msg].forEach((cb) => cb(m)); }
  drop() { [...this.close].forEach((cb) => cb()); }
}
const fakeCapture = () => ({ stream: { id: 'mic' }, setLive: vi.fn(), setSender: vi.fn(), setSettings: vi.fn(), onModeChange: vi.fn(), getStats: () => null, close: vi.fn() });
const fakePc = () => ({ getSenders: () => [{ track: { kind: 'audio' } }], close: vi.fn(), getStats: vi.fn(async () => new Map()), iceConnectionState: 'connected' });
const me = 'a'.repeat(32);

let sig: FakeSig;
let stop: () => void;
beforeEach(() => {
  vi.useFakeTimers();
  calls.length = 0;
  h.playbacks.length = 0;
  devices = [];
  stored.clear();
  sig = new FakeSig();
  session.sig = sig as never;
  session.online = true;
  joinRoom.mockReset();
  stop = v.startVoice();
  flushSync();
});
afterEach(() => {
  stop();
  vi.useRealTimers();
});

// Joins room 1 with a working mic and returns the fakes.
async function joined() {
  const cap = fakeCapture();
  const pc = fakePc();
  const off = vi.fn();
  joinRoom.mockResolvedValue({ pc, userId: me, off, peers: [] });
  const p = v.join(1);
  h.capture!.resolve(cap);
  expect(await p).toBe(false);
  expect(voice.joined).toBe(true);
  return { cap, pc, off };
}

describe('join', () => {
  it('creates the playback context and calls createCapture before its first await', () => {
    const p = v.join(1); // not awaited: only the synchronous part has run
    expect(calls).toEqual(['playback', 'capture']);
    expect(voice.joining).toBe(true);
    h.capture!.reject(new Error('stop'));
    return p;
  });

  it('a failed mic ends the attempt without telling the server, and asks for focus on the error', async () => {
    const p = v.join(1);
    h.capture!.reject(new DOMException('no', 'NotAllowedError'));
    expect(await p).toBe(true);
    expect(sig.leaves).toBe(0);
    expect(voice.error).not.toBe('');
    expect(voice.joining).toBe(false);
    expect(h.playbacks[0].close).toHaveBeenCalled();
  });

  it('refuses while offline', async () => {
    session.online = false;
    expect(await v.join(1)).toBe(false);
    expect(voice.error).toBe('Not connected to the server. Wait a moment and try again.');
    expect(calls).toEqual([]);
  });
});

describe('in a call', () => {
  it('leave cleans up: capture and playback closed, pc closed, handlers dropped, timers cleared', async () => {
    const { cap, pc, off } = await joined();
    expect(vi.getTimerCount()).toBe(2); // the speaking poll and the stats poll
    v.leave();
    expect(sig.leaves).toBe(1);
    expect(cap.close).toHaveBeenCalled(); // stops the mic pipeline and closes its AudioContext
    expect(h.playbacks[0].close).toHaveBeenCalled();
    expect(pc.close).toHaveBeenCalled();
    expect(off).toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
    expect(voice).toMatchObject({ joined: false, joinedRoom: null, capture: undefined, pc: undefined, quality: 'unknown', error: '' });
  });

  it('polls stats every 2 s into quality', async () => {
    const { pc } = await joined();
    await vi.advanceTimersByTimeAsync(2000);
    expect(pc.getStats).toHaveBeenCalledTimes(1);
    expect(voice.quality).toBe('unknown'); // an empty report has no values
  });

  it('force_muted mutes and blocks unmute; lifting it keeps the mute', async () => {
    const { cap } = await joined();
    sig.emit({ type: 'force_muted', muted: true });
    expect(voice).toMatchObject({ forceMuted: true, muted: true });
    expect(cap.setLive).toHaveBeenLastCalledWith(false);
    v.toggleMute();
    expect(voice.muted).toBe(true);
    sig.emit({ type: 'force_muted', muted: false });
    expect(voice).toMatchObject({ forceMuted: false, muted: true });
  });

  it('peer_left with our own id ends the call; another id does not', async () => {
    await joined();
    sig.emit({ type: 'peer_left', user_id: 'b'.repeat(32) });
    expect(voice.joined).toBe(true);
    sig.emit({ type: 'peer_left', user_id: me });
    expect(voice.joined).toBe(false);
    expect(voice.error).toBe(CALL_ENDED);
    expect(sig.leaves).toBe(0);
  });

  it('a room that vanished from the list ends the call and tells the server', async () => {
    await joined();
    sig.emit({ type: 'rooms_changed', rooms: [{ id: 2, name: 'r', position: 0, restricted: false, members: [] }] } as ServerMsg);
    expect(voice.joined).toBe(false);
    expect(voice.error).toBe(ROOM_GONE);
    expect(sig.leaves).toBe(1);
  });

  it('a dropped socket ends the call; the new socket after a reconnect is subscribed and the old one is not', async () => {
    await joined();
    const old = sig;
    old.drop();
    expect(voice.joined).toBe(false);
    expect(voice.error).toBe('Disconnected from the server.');
    sig = new FakeSig();
    session.sig = sig as never;
    flushSync();
    expect(old.msg.size + old.close.size).toBe(0);
    expect(sig.msg.size).toBe(1);
    expect(sig.close.size).toBe(1);
    await joined();
    expect(joinRoom.mock.calls.at(-1)![0]).toBe(sig);
  });
});

// Logout and a 4001 close end the voice session (stop) and a new sign-in starts it again: nothing may carry over.
describe('a new session starts fresh', () => {
  const restart = () => {
    stop();
    stop = v.startVoice();
    flushSync();
  };

  it('a failed join leaves no error alert for the next sign-in', async () => {
    const p = v.join(1);
    h.capture!.reject(new DOMException('no', 'NotAllowedError'));
    await p;
    expect(voice.error).not.toBe('');
    restart();
    expect(voice.error).toBe('');
  });

  it('a 4001 close in a call leaves no "Disconnected from the server." whichever close handler runs first', async () => {
    // The session's handler (logout, stop) ran after the call's handler: the call already wrote its text.
    await joined();
    sig.drop();
    expect(voice.error).toBe('Disconnected from the server.');
    restart();
    expect(voice.error).toBe('');
    // The session's handler ran first: the call's handler still runs in the same loop, after stop.
    stop();
    sig = new FakeSig();
    session.sig = sig as never;
    sig.close.add(() => stop());
    stop = v.startVoice();
    flushSync();
    await joined();
    sig.drop();
    expect(voice.error).toBe('');
    stop = v.startVoice();
    flushSync();
    expect(voice.error).toBe('');
  });

  it('Diagnostics shows deepfilter again after a call that fell back', async () => {
    const { cap } = await joined();
    const onMode = cap.onModeChange.mock.calls[0][0] as (m: string, r: string) => void;
    onMode('browser', 'denoiser failed');
    expect(voice.mode).toBe('browser');
    restart();
    expect(voice.mode).toBe('deepfilter');
    expect(voice).toMatchObject({ volumes: {}, userId: '', joinedBitrate: 64 });
  });

  it('a mic plugged in while signed out is not offered later', async () => {
    const mic = (id: string) => ({ kind: 'audioinput', deviceId: id, label: id, groupId: '' }) as MediaDeviceInfo;
    devices = [mic('a')];
    restart();
    await vi.advanceTimersByTimeAsync(0); // the first enumerateDevices
    stop(); // signed out
    devices = [mic('a'), mic('b')];
    stop = v.startVoice();
    flushSync();
    await vi.advanceTimersByTimeAsync(0);
    await joined();
    md.last!();
    await vi.advanceTimersByTimeAsync(0);
    expect(voice.offered).toBeNull();
  });

  it('reads the audio settings from storage again', async () => {
    stop();
    stored.set(SETTINGS_KEY, JSON.stringify({ bitrateKbps: 32 }));
    stop = v.startVoice();
    expect(voice.settings.bitrateKbps).toBe(32);
  });
});

it('20 joins and leaves leak no listeners, timers or socket handlers', async () => {
  stop();
  for (let i = 0; i < 20; i++) {
    stop = v.startVoice();
    flushSync();
    await joined();
    v.leave();
    stop();
    expect([win.n, doc.n, md.n, sig.msg.size, sig.close.size, vi.getTimerCount()]).toEqual([0, 0, 0, 0, 0, 0]);
  }
  stop = v.startVoice();
});
