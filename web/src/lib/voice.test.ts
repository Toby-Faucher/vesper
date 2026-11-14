import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync } from 'svelte';
import type { ServerMsg } from './gen/ServerMsg';

// Fakes for everything that touches audio, WebRTC and the browser. `calls` records the order of the join's steps.
const calls: string[] = [];
const chime = vi.hoisted(() => vi.fn());
vi.mock('./ui/chime', () => ({ chime }));
const h = vi.hoisted(() => ({
  capture: null as null | { resolve: (c: unknown) => void; reject: (e: unknown) => void },
  playbacks: [] as { close: ReturnType<typeof vi.fn>; setSink: ReturnType<typeof vi.fn>; setLevelling: ReturnType<typeof vi.fn> }[],
  sinkFail: null as unknown,
  ctx: undefined as unknown, // the AudioContext the last Playback was given (undefined: it made its own)
}));
vi.mock('./playback', () => ({
  Playback: class {
    constructor(_volumes: unknown, ctx?: unknown) {
      calls.push('playback'); // the real one creates and resumes its AudioContext here, unless it is given one
      h.ctx = ctx;
      h.playbacks.push(this as never);
    }
    close = vi.fn();
    setSink = vi.fn(async () => { if (h.sinkFail) throw h.sinkFail; return true; });
    addLocal = vi.fn();
    addRemote = vi.fn();
    remove = vi.fn();
    setVolume = vi.fn();
    setDeafened = vi.fn();
    setLevelling = vi.fn();
    speaking = () => new Set<string>();
  },
}));
vi.mock('./audio/capture', () => ({
  createCapture: vi.fn(() => {
    calls.push('capture'); // the real one creates its AudioContext before its first await
    return new Promise((resolve, reject) => (h.capture = { resolve, reject }));
  }),
}));
vi.mock('../routes/CallBar.svelte', () => ({ default: {} })); // in the call chunk; its styles need a real document
vi.mock('../routes/SharePanel.svelte', () => ({ default: {} })); // the same
const joinRoom = vi.fn();
vi.mock('./join', () => ({ joinRoom: (...a: unknown[]) => joinRoom(...a) }));
const send = vi.hoisted(() => ({ capture: vi.fn(), applyShare: vi.fn() }));
vi.mock('./share/send', () => send);

// Counts listeners on window, document and mediaDevices.
class Target {
  n = 0;
  last: (() => void) | undefined; // the newest listener, to fire a devicechange
  addEventListener(_t: string, fn: () => void) { this.n++; this.last = fn; }
  removeEventListener() { this.n--; }
}
const idleDefaults = { requestIdleCallback: () => 0, cancelIdleCallback: () => {} }; // never runs: tests load the chunk themselves
const win = Object.assign(new Target(), idleDefaults);
const doc = Object.assign(new Target(), { visibilityState: 'visible' });
let devices: MediaDeviceInfo[] = [];
const md = Object.assign(new Target(), { enumerateDevices: async () => devices });
vi.stubGlobal('window', win);
vi.stubGlobal('document', doc);
vi.stubGlobal('navigator', { mediaDevices: md, locks: { request: vi.fn() } });
const stored = new Map<string, string>();
const writes: string[] = [];
vi.stubGlobal('localStorage', { getItem: (k: string) => stored.get(k) ?? null, setItem: (k: string, x: string) => void writes.push(`${k}=${x}`), removeItem: () => {} });
vi.stubGlobal('location', { pathname: '/', search: '', hash: '', protocol: 'http:', host: 'x' });

const { session, runBeforeLogout } = await import('./session.svelte');
const v = await import('./voice.svelte');
const { voice } = v;
const { CALL_ENDED, ROOM_GONE } = await import('./call');
const { DISCONNECTED, REJOIN_MS } = v;
const { SETTINGS_KEY } = await import('./audio/settings');
const { createCapture } = await import('./audio/capture');
const { MIC_FALLBACK, SPEAKER_FALLBACK } = await import('./devices');
const { shares } = await import('./share/state.svelte');

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
let cold = true; // the first test runs before the call chunk has loaded; every later one after
beforeEach(async () => {
  if (!cold) await v.loadCall();
  vi.useFakeTimers();
  calls.length = 0;
  h.playbacks.length = 0;
  devices = [];
  stored.clear();
  h.sinkFail = null;
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
  const video = { replaceTrack: vi.fn(async (_t: unknown) => {}) };
  joinRoom.mockResolvedValue({ pc, userId: me, off, peers: [], video });
  const p = v.join(1);
  h.capture!.resolve(cap);
  expect(await p).toBe(false);
  expect(voice.joined).toBe(true);
  return { cap, pc, off, video };
}

// Must stay the first test: it needs the call chunk not loaded yet.
it('a cold page (no hover or focus first): both AudioContexts are made and resumed inside the click, then handed over', async () => {
  const made: { opts?: unknown; resumed: boolean }[] = [];
  const g = globalThis as { AudioContext?: unknown };
  g.AudioContext = class {
    resumed = false;
    constructor(public opts?: unknown) { made.push(this); }
    resume() { this.resumed = true; return Promise.resolve(); }
    close() { return Promise.resolve(); }
  };
  try {
    const p = v.join(1); // not awaited: only the synchronous part has run
    expect(made.map((c) => [c.opts, c.resumed])).toEqual([[undefined, true], [{ sampleRate: 48000, latencyHint: 'interactive' }, true]]);
    expect(calls).toEqual([]); // the call code is still loading
    expect(voice.joining).toBe(true);
    await vi.waitFor(() => expect(calls).toEqual(['playback', 'capture']));
    expect(h.ctx).toBe(made[0]);
    expect(vi.mocked(createCapture).mock.calls.at(-1)![1]!.createContext!()).toBe(made[1]);
    h.capture!.reject(new Error('stop'));
    expect(await p).toBe(true);
  } finally {
    delete g.AudioContext;
    cold = false;
  }
});

describe('join', () => {
  it('even out voices: off by default, passed to each new call, saved, and applied to a running call', async () => {
    expect(voice.levelling).toBe(false);
    await joined();
    expect(h.playbacks[0].setLevelling).toHaveBeenLastCalledWith(false);
    v.setLevelling(true);
    expect(h.playbacks[0].setLevelling).toHaveBeenLastCalledWith(true);
    expect(writes).toContain('vesper.level=on');
    v.leave();
    await joined();
    expect(h.playbacks[1].setLevelling).toHaveBeenLastCalledWith(true); // a fresh graph: starts from 0 dB, levelling on
    v.setLevelling(false);
  });

  it('with the call chunk loaded (a hover or focus came first): creates the playback context and calls createCapture before its first await', () => {
    const p = v.join(1); // not awaited: only the synchronous part has run
    expect(calls).toEqual(['playback', 'capture']);
    expect(h.ctx).toBeUndefined(); // Playback makes its own, inside the click
    expect(voice.joining).toBe(true);
    h.capture!.reject(new Error('stop'));
    return p;
  });

  it('a second join while the first is still starting does nothing (the switcher has no busy state)', async () => {
    const p = v.join(1);
    expect(await v.join(2)).toBe(false);
    expect(h.playbacks).toHaveLength(1); // no second Playback, capture or mic
    h.capture!.reject(new Error('stop'));
    await p;
  });

  it('startVoice loads the call chunk when the browser is idle, and its cleanup cancels that', () => {
    const ric = vi.fn((_cb: () => void, _o: unknown) => 7);
    const cic = vi.fn();
    Object.assign(win, { requestIdleCallback: ric, cancelIdleCallback: cic });
    try {
      const stop2 = v.startVoice();
      expect(ric).toHaveBeenCalledWith(expect.any(Function), { timeout: 5000 });
      stop2();
      expect(cic).toHaveBeenCalledWith(7);
    } finally {
      Object.assign(win, idleDefaults);
    }
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

  it('a mic that would not start shows a sentence with the way out, and the browser words as details', async () => {
    const p = v.join(1);
    h.capture!.reject({ name: 'OverconstrainedError', message: 'Constraints could not be satisfied.' }); // Firefox: no DOMException
    expect(await p).toBe(true);
    expect(voice.error).toMatch(/^Vesper could not find a microphone\. .*Settings > Voice > Microphone/);
    expect(voice.error).toMatch(/Details: OverconstrainedError: Constraints could not be satisfied\.$/);
  });

  it('a saved mic that is gone: the join goes on with the default, the id is forgotten and stored, one notice', async () => {
    stop();
    stored.set(SETTINGS_KEY, JSON.stringify({ micId: 'gone' }));
    stop = v.startVoice();
    const p = joined();
    const deps = vi.mocked(createCapture).mock.calls.at(-1)![1]!;
    expect(vi.mocked(createCapture).mock.calls.at(-1)![0].micId).toBe('gone');
    writes.length = 0;
    deps.onMicFallback!(); // the capture got the default mic after the saved one failed
    await p;
    expect(voice.settings.micId).toBeNull();
    expect(JSON.parse(writes.at(-1)!.split('=')[1]).micId).toBeNull();
    expect(voice.notice).toBe(MIC_FALLBACK);
    v.leave();
    expect(voice.notice).toBe('');
  });

  it('a saved speaker that is gone: the join goes on, the id is forgotten, one notice', async () => {
    stop();
    stored.set(SETTINGS_KEY, JSON.stringify({ speakerId: 'gone' }));
    stop = v.startVoice();
    h.sinkFail = new DOMException('not found', 'NotFoundError');
    await joined();
    expect(h.playbacks[0].setSink).toHaveBeenCalledWith('gone');
    expect(voice.settings.speakerId).toBeNull();
    expect(voice.notice).toBe(SPEAKER_FALLBACK);
    expect(voice.error).toBe('');
  });

  it('a saved speaker that fails for another reason is kept, without a notice', async () => {
    stop();
    stored.set(SETTINGS_KEY, JSON.stringify({ speakerId: 'usb' }));
    stop = v.startVoice();
    for (const name of ['AbortError', 'NotAllowedError']) {
      h.sinkFail = new DOMException('x', name);
      await joined();
      expect(voice.settings.speakerId).toBe('usb');
      expect(voice.notice).toBe('');
      v.leave();
    }
  });

  it('a failing default speaker changes nothing', async () => {
    h.sinkFail = new DOMException('x', 'AbortError');
    await joined();
    expect(voice.notice).toBe('');
  });

  it('in a call: a picked speaker that is gone goes back to the default with the notice; another failure says so', async () => {
    await joined();
    h.sinkFail = new DOMException('x', 'NotFoundError');
    v.updateSettings({ ...voice.settings, speakerId: 'gone' });
    await vi.advanceTimersByTimeAsync(0);
    expect(voice.settings.speakerId).toBeNull();
    expect(voice.notice).toBe(SPEAKER_FALLBACK);
    expect(h.playbacks[0].setSink).toHaveBeenLastCalledWith(null);
    h.sinkFail = new DOMException('x', 'AbortError');
    v.updateSettings({ ...voice.settings, speakerId: 'usb' });
    await vi.advanceTimersByTimeAsync(0);
    expect(voice.settings.speakerId).toBe('usb');
    expect(voice.error).toBe('could not switch speaker');
  });

  it('the fallback notice goes after NOTICE_MS', async () => {
    stop();
    stored.set(SETTINGS_KEY, JSON.stringify({ micId: 'gone' }));
    stop = v.startVoice();
    const p = joined();
    vi.mocked(createCapture).mock.calls.at(-1)![1]!.onMicFallback!();
    await p;
    await vi.advanceTimersByTimeAsync(v.NOTICE_MS - 100);
    expect(voice.notice).toBe(MIC_FALLBACK);
    await vi.advanceTimersByTimeAsync(200);
    expect(voice.notice).toBe('');
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

  it('my own join and leave play connect and disconnect', async () => {
    chime.mockClear();
    await joined();
    flushSync();
    expect(chime).toHaveBeenLastCalledWith('connect', null, expect.any(Number));
    v.leave();
    flushSync();
    expect(chime).toHaveBeenLastCalledWith('disconnect', null, expect.any(Number));
  });

  it('the end of a call sends one summary to your own list; a failed join sends none', async () => {
    const fetch = vi.fn(async () => ({ ok: true, status: 204 }));
    vi.stubGlobal('fetch', fetch);
    try {
      joinRoom.mockRejectedValueOnce(new Error('refused'));
      const p = v.join(1);
      h.capture!.resolve(fakeCapture());
      await p;
      expect(fetch).not.toHaveBeenCalled();
      await joined();
      await vi.advanceTimersByTimeAsync(4000);
      v.leave();
      v.leave();
      expect(fetch).toHaveBeenCalledTimes(1);
      const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
      expect([url, init.method]).toEqual(['/api/me/calls', 'POST']);
      expect(JSON.parse(init.body as string)).toMatchObject({ duration_s: 4, mode: 'deepfilter', browser_ns_loading: false });
    } finally {
      vi.stubGlobal('fetch', undefined);
    }
  });

  it('sign out ends the call and waits for its summary before /api/logout', async () => {
    let done!: () => void;
    const fetch = vi.fn(() => new Promise((r) => (done = () => r({ ok: true, status: 204 }))));
    vi.stubGlobal('fetch', fetch);
    try {
      await joined();
      let finished = false;
      const p = runBeforeLogout().then(() => (finished = true));
      expect(voice.joined).toBe(false);
      expect(fetch).toHaveBeenCalledWith('/api/me/calls', expect.objectContaining({ method: 'POST' }));
      await Promise.resolve();
      expect(finished).toBe(false); // logout() waits for the post (up to 1.5 s)
      done();
      await p;
      expect(finished).toBe(true);
    } finally {
      vi.stubGlobal('fetch', undefined);
    }
  });

  it('Retry microphone: one retry at a time, in the same call, the reason comes from the capture', async () => {
    const { cap } = await joined();
    let done!: (ok: boolean) => void;
    const retry = vi.fn(() => new Promise<boolean>((r) => (done = r)));
    Object.assign(cap, { retry });
    const onMode = cap.onModeChange.mock.calls[0][0] as (m: string, r: string) => void;
    onMode('off', 'microphone lost: ended');
    expect(voice.deviceError).toMatch(/^The microphone stopped/);
    const p = v.retryMic();
    void v.retryMic(); // a second press while it runs does nothing
    expect(retry).toHaveBeenCalledTimes(1);
    expect(retry).toHaveBeenCalledWith(false); // a lost mic: the swap is not blamed
    expect(voice.retrying).toBe(true);
    onMode('browser', '');
    done(true);
    await p;
    expect(voice).toMatchObject({ retrying: false, deviceError: '', mode: 'browser', joined: true });
    expect(sig.leaves).toBe(0);
    voice.micWarning = 'mic'; // "no sound": the retry also keeps the browser filter (no reopened mic)
    const q = v.retryMic();
    expect(retry).toHaveBeenLastCalledWith(true);
    done(true);
    await q;
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
    sig.emit({ type: 'rooms_changed', rooms: [{ id: 2, kind: 'room', name: 'r', position: 0, restricted: false, members: [], archived: false }] } as ServerMsg);
    expect(voice.joined).toBe(false);
    expect(voice.error).toBe(ROOM_GONE);
    expect(sig.leaves).toBe(1);
  });

  // What session.svelte does on a drop and on the next hello.
  const lose = () => { sig.drop(); session.online = false; session.sig = null; flushSync(); };
  const regain = () => { sig = new FakeSig(); session.sig = sig as never; flushSync(); session.online = true; flushSync(); };

  it('a dropped socket ends the media but not the call: it joins the same room again on the next socket', async () => {
    await joined();
    const old = sig;
    lose();
    expect(voice.joined).toBe(false);
    expect(voice.error).toBe('');
    expect(voice.rejoin).toBe(1);
    expect(joinRoom).toHaveBeenCalledTimes(1);
    regain();
    expect(old.msg.size + old.close.size).toBe(0);
    expect(sig.msg.size).toBe(1);
    expect(sig.close.size).toBe(1);
    expect(voice.rejoin).toBe(null);
    h.capture!.resolve(fakeCapture());
    await vi.waitFor(() => expect(voice.joined).toBe(true));
    expect(joinRoom).toHaveBeenCalledTimes(2);
    expect(joinRoom.mock.calls.at(-1)![0]).toBe(sig);
    expect(joinRoom.mock.calls.at(-1)![1]).toMatchObject({ roomId: 1 });
    vi.advanceTimersByTime(REJOIN_MS); // the give-up timer went with the rejoin
    expect(voice.error).toBe('');
    expect(voice.joined).toBe(true);
  });

  it('no socket back within a minute: it gives up and says so', async () => {
    await joined();
    lose();
    vi.advanceTimersByTime(REJOIN_MS);
    expect(voice.rejoin).toBe(null);
    expect(voice.error).toBe(DISCONNECTED);
    regain();
    expect(joinRoom).toHaveBeenCalledTimes(1);
  });

  it('joining another room yourself as the socket comes back replaces the rejoin', async () => {
    await joined();
    lose();
    sig = new FakeSig();
    session.sig = sig as never;
    session.online = true; // the hello is in, and the rejoin effect has not run yet
    void v.join(2);
    expect(voice.rejoin).toBe(null);
    flushSync();
    h.capture!.resolve(fakeCapture());
    await vi.waitFor(() => expect(voice.joined).toBe(true));
    expect(joinRoom).toHaveBeenCalledTimes(2);
    expect(joinRoom.mock.calls.at(-1)![1]).toMatchObject({ roomId: 2 });
    vi.advanceTimersByTime(REJOIN_MS);
    expect(voice.error).toBe('');
  });

  it('a click on Join while still offline keeps the rejoin', async () => {
    await joined();
    lose();
    expect(await v.join(2)).toBe(false);
    expect(voice.rejoin).toBe(1);
  });

  it('a drop while not in a call rejoins nothing', async () => {
    lose();
    expect(voice.rejoin).toBe(null);
    regain();
    expect(joinRoom).not.toHaveBeenCalled();
  });
});

describe('screen share', () => {
  const bob = 'b'.repeat(32);
  // A captured screen track: `fire('ended')` is the browser's own Stop sharing.
  const screen = (width = 1920, height = 1080) => {
    const on = new Map<string, () => void>();
    return {
      stop: vi.fn(),
      applyConstraints: vi.fn(async () => {}),
      getSettings: () => ({ width, height }),
      addEventListener: (t: string, f: () => void) => on.set(t, f),
      fire: (t: string) => on.get(t)?.(),
    };
  };
  const shareMsgs = () => sig.sent.filter((m) => (m as { type: string }).type.startsWith('share_'));
  beforeEach(() => {
    send.capture.mockReset();
    send.applyShare.mockReset().mockResolvedValue(undefined);
  });
  async function sharing(s = { fps: 60 as const, kbps: 8000, prefer: 'sharp' as const }) {
    const call = await joined();
    const track = screen();
    send.capture.mockResolvedValue(track);
    await v.startShare(s);
    return { ...call, track };
  }

  it('starts_a_share', async () => {
    const { video, track } = await sharing();
    expect(send.capture).toHaveBeenCalledWith(60, 'sharp');
    expect(video.replaceTrack).toHaveBeenCalledWith(track);
    expect(send.applyShare).toHaveBeenCalledWith(video, expect.objectContaining({ fps: 60, kbps: 8000, prefer: 'sharp' }), false); // on once someone watches
    expect(send.applyShare.mock.invocationCallOrder[0]).toBeLessThan(video.replaceTrack.mock.invocationCallOrder[0]); // never sends before that
    expect(shareMsgs()).toEqual([{ type: 'share_start', fps: 60, bitrate_kbps: 8000 }]);
    expect(shares.mine).toMatchObject({ track, fps: 60, kbps: 8000, prefer: 'sharp', watchers: 0 });
  });

  it('no bitrate set: the suggestion, capped by the browser ceiling for the captured size', async () => {
    const nav = navigator as unknown as { userAgent?: string };
    nav.userAgent = 'Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0';
    try {
      await joined();
      send.capture.mockResolvedValue(screen(1280, 720));
      await v.startShare({ fps: 30, kbps: null, prefer: 'smooth' });
      expect(shareMsgs()).toEqual([{ type: 'share_start', fps: 30, bitrate_kbps: 5000 }]);
      await v.updateShare({ fps: 30, kbps: 9000, prefer: 'smooth' });
      expect(shareMsgs().at(-1)).toEqual({ type: 'share_start', fps: 30, bitrate_kbps: 5000 });
    } finally {
      delete nav.userAgent;
    }
  });

  it('cancelled_picker_sends_nothing', async () => {
    await joined();
    send.capture.mockResolvedValue(null);
    await v.startShare({ fps: 60, kbps: 8000, prefer: 'sharp' });
    expect(shareMsgs()).toEqual([]);
    expect(voice.error).toBe('');
    expect(shares.mine).toBeNull();
  });

  it('browser_stop_ends_the_share', async () => {
    const { video, track } = await sharing();
    track.fire('ended');
    expect(shareMsgs().at(-1)).toEqual({ type: 'share_stop' });
    expect(shares.mine).toBeNull();
    expect(video.replaceTrack).toHaveBeenLastCalledWith(null);
  });

  it('Stop ends the share and tells the server once', async () => {
    const { track } = await sharing();
    v.stopShare();
    v.stopShare();
    expect(track.stop).toHaveBeenCalled();
    expect(shareMsgs()).toEqual([{ type: 'share_start', fps: 60, bitrate_kbps: 8000 }, { type: 'share_stop' }]);
  });

  it('watchers_turn_the_encoder_on_and_off', async () => {
    const { video } = await sharing();
    sig.emit({ type: 'watchers', count: 1 });
    await vi.advanceTimersByTimeAsync(0);
    expect(send.applyShare).toHaveBeenLastCalledWith(video, expect.objectContaining({ fps: 60, kbps: 8000, prefer: 'sharp' }), true);
    expect(shares.mine?.watchers).toBe(1);
    sig.emit({ type: 'watchers', count: 0 });
    await vi.advanceTimersByTimeAsync(0);
    expect(send.applyShare).toHaveBeenLastCalledWith(video, expect.objectContaining({ fps: 60 }), false);
  });

  it('new settings while sharing: applied with the encoder still on, and sent', async () => {
    const { video, track } = await sharing();
    sig.emit({ type: 'watchers', count: 2 });
    await v.updateShare({ fps: 30, kbps: 4000, prefer: 'smooth' });
    expect(track.applyConstraints).toHaveBeenCalledWith({ frameRate: 30 });
    expect(send.applyShare).toHaveBeenLastCalledWith(video, expect.objectContaining({ fps: 30, kbps: 4000, prefer: 'smooth' }), true);
    expect(shareMsgs().at(-1)).toEqual({ type: 'share_start', fps: 30, bitrate_kbps: 4000 });
    expect(shares.mine).toMatchObject({ fps: 30, kbps: 4000, prefer: 'smooth', watchers: 2 });
  });

  it('server_cut_stops_locally', async () => {
    const { track } = await sharing();
    sig.emit({ type: 'peer_share', user_id: me, share: null });
    expect(track.stop).toHaveBeenCalled();
    expect(shares.mine).toBeNull();
    expect(shareMsgs()).toEqual([{ type: 'share_start', fps: 60, bitrate_kbps: 8000 }]);
  });

  it('a_moderator_stop_ends_the_share_with_a_notice', async () => {
    const { track } = await sharing();
    sig.emit({ type: 'peer_share', user_id: me, share: null });
    sig.emit({ type: 'share_stopped' });
    expect(track.stop).toHaveBeenCalled();
    expect(shares.mine).toBeNull();
    expect(voice.notice).toBe('A moderator stopped your screen share.');
    expect(voice.error).toBe('');
    expect(shareMsgs()).toEqual([{ type: 'share_start', fps: 60, bitrate_kbps: 8000 }]); // no share_stop: the server ended it
  });

  it('rate_limited_just_after_a_cut_says_the_share_was_stopped', async () => {
    const cut = 'Your screen share sent more than 35 Mbps and was stopped.';
    const limited = Object.assign(new Error('Too many tries. Wait a moment and try again.'), { code: 'rate_limited' });
    await sharing();
    const { onError } = joinRoom.mock.calls.at(-1)![1];
    onError(limited); // the message budget while sharing, not a cut
    expect(voice.error).toBe('Too many tries. Wait a moment and try again.');
    voice.error = '';
    sig.emit({ type: 'peer_share', user_id: me, share: null });
    onError(limited);
    expect(voice.error).toBe(cut);
    voice.error = '';
    onError(new Error('Something else.'));
    expect(voice.error).toBe('Something else.');
  });

  const capped = () => Object.assign(new Error('Too many screens are being shared right now.'), { code: 'share_cap' });

  it('refused_start_ends_the_local_share', async () => {
    const { video, track } = await sharing();
    const { onError } = joinRoom.mock.calls.at(-1)![1];
    onError(capped());
    expect(track.stop).toHaveBeenCalled();
    expect(shares.mine).toBeNull();
    expect(video.replaceTrack).toHaveBeenLastCalledWith(null);
    expect(shareMsgs()).toEqual([{ type: 'share_start', fps: 60, bitrate_kbps: 8000 }]); // no share_stop: the server has no share
    expect(voice.error).toBe('Too many screens are being shared right now.');
    onError(Object.assign(new Error('Too many tries. Wait a moment and try again.'), { code: 'rate_limited' }));
    expect(voice.error).toBe('Too many tries. Wait a moment and try again.'); // not a cut
    // A DM refuses with forbidden.
    const again = screen();
    send.capture.mockResolvedValue(again);
    await v.startShare({ fps: 60, kbps: 8000, prefer: 'sharp' });
    onError(Object.assign(new Error('You cannot join this room.'), { code: 'forbidden' }));
    expect(again.stop).toHaveBeenCalled();
    expect(shares.mine).toBeNull();
    expect(voice.error).toBe('Screen share is only in group rooms.');
  });

  it('refused_update_restores_the_previous_settings', async () => {
    const { video, track } = await sharing();
    sig.emit({ type: 'watchers', count: 2 });
    await v.updateShare({ fps: 30, kbps: 20000, prefer: 'smooth' });
    joinRoom.mock.calls.at(-1)![1].onError(capped());
    await vi.advanceTimersByTimeAsync(0);
    expect(shares.mine).toMatchObject({ track, fps: 60, kbps: 8000, prefer: 'sharp', watchers: 2 });
    expect(send.applyShare).toHaveBeenLastCalledWith(video, expect.objectContaining({ fps: 60, kbps: 8000, prefer: 'sharp' }), true);
    expect(track.applyConstraints).toHaveBeenLastCalledWith({ frameRate: 60 });
    expect(track.stop).not.toHaveBeenCalled();
    expect(shareMsgs().at(-1)).toEqual({ type: 'share_start', fps: 30, bitrate_kbps: 20000 }); // nothing more sent
    expect(voice.error).toBe('Too many screens are being shared right now.');
  });

  describe('share cue', () => {
    const peer = (share: { fps: number; bitrate_kbps: number } | null) => ({ user_id: bob, name: 'bob', room_id: 1, muted: false, deafened: false, force_muted: false, share });
    const setPeers = (voiceList: ReturnType<typeof peer>[]) => { session.state = { ...session.state, voice: voiceList }; flushSync(); };
    it('a_peer_in_my_call_starting_a_share_plays_share_start', async () => {
      await joined();
      setPeers([peer(null)]);
      chime.mockClear();
      setPeers([peer({ fps: 60, bitrate_kbps: 8000 })]);
      expect(chime).toHaveBeenCalledWith('share-start', null, expect.any(Number));
      setPeers([peer(null)]);
      expect(chime).toHaveBeenLastCalledWith('share-stop', null, expect.any(Number));
    });
    it('call_cues_off_plays_nothing', async () => {
      await joined();
      stored.set('vesper.notify', JSON.stringify({ call: false }));
      setPeers([peer(null)]);
      chime.mockClear();
      setPeers([peer({ fps: 60, bitrate_kbps: 8000 })]);
      expect(chime).not.toHaveBeenCalled();
    });
  });

  it('rate_limited_without_a_share_keeps_its_text', async () => {
    await joined();
    joinRoom.mock.calls.at(-1)![1].onError(Object.assign(new Error('Too many tries. Wait a moment and try again.'), { code: 'rate_limited' }));
    expect(voice.error).toBe('Too many tries. Wait a moment and try again.');
  });

  it('leaving the call ends the share locally and clears the shares', async () => {
    const { track } = await sharing();
    joinRoom.mock.calls.at(-1)![1].onRemoteStream(bob, { id: bob }, 'video');
    v.leave();
    expect(track.stop).toHaveBeenCalled();
    expect(shareMsgs()).toEqual([{ type: 'share_start', fps: 60, bitrate_kbps: 8000 }]);
    expect(shares.mine).toBeNull();
    expect(shares.streams.size).toBe(0);
  });

  it('remote_video_never_reaches_playback', async () => {
    await joined();
    const { onRemoteStream, onRemoteStreamEnd } = joinRoom.mock.calls.at(-1)![1];
    const s = { id: bob, getAudioTracks: () => [] };
    onRemoteStream(bob, s, 'video');
    expect(shares.streams.get(bob)).toBe(s);
    const pb = h.playbacks[0] as unknown as { addRemote: ReturnType<typeof vi.fn>; remove: ReturnType<typeof vi.fn> };
    expect(pb.addRemote).not.toHaveBeenCalled();
    onRemoteStreamEnd(bob, 'video');
    expect(shares.streams.has(bob)).toBe(false);
    expect(pb.remove).not.toHaveBeenCalled();
  });

  it('watch and point go to the server', async () => {
    await joined();
    v.watchShare(bob, true);
    v.pointAt(bob, 0.5, 0.25);
    expect(sig.sent.slice(-2)).toEqual([{ type: 'watch', user_id: bob, on: true }, { type: 'point', user_id: bob, x: 0.5, y: 0.25 }]);
  });

  it('a rejoin sends watch on again: the server forgot the watches, so the gates start over', async () => {
    const { syncWatches } = await import('./share/watch');
    const watches = () => sig.sent.filter((m) => (m as { type: string }).type === 'watch');
    await joined();
    shares.shown = new Set([bob]);
    syncWatches([bob], v.watchShare);
    v.leave();
    await joined();
    shares.shown = new Set([bob]);
    syncWatches([bob], v.watchShare);
    expect(watches()).toEqual([{ type: 'watch', user_id: bob, on: true }, { type: 'watch', user_id: bob, on: true }]);
  });

  it('point_adds_a_marker', async () => {
    await joined();
    const from = 'c'.repeat(32);
    sig.emit({ type: 'point', user_id: bob, from, x: 0.5, y: 0.25 });
    expect(shares.markers).toEqual([{ share: bob, from, x: 0.5, y: 0.25, until: Date.now() + 3000 }]);
    await vi.advanceTimersByTimeAsync(2999);
    expect(shares.markers).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(shares.markers).toEqual([]);
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
    // The session's handler (logout, stop) ran after the call's handler: the call already set its rejoin.
    await joined();
    sig.drop();
    expect(voice.rejoin).toBe(1);
    restart();
    expect(voice.error).toBe('');
    expect(voice.rejoin).toBe(null);
    vi.advanceTimersByTime(REJOIN_MS); // the give-up timer went with the session
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

it('a volume drag saves at most every 250 ms, the last value included, and sign out saves at once', async () => {
  writes.length = 0;
  for (const x of [110, 120, 130]) v.setVolume('u1', x);
  expect(writes).toEqual([]);
  await vi.advanceTimersByTimeAsync(250);
  expect(writes).toEqual(['vesper.volumes={"u1":130}']);
  v.setVolume('u1', 90);
  stop(); // signed out before the timer: written now, not lost and not overwritten by the next account's empty map
  expect(writes.at(-1)).toBe('vesper.volumes={"u1":90}');
  await vi.advanceTimersByTimeAsync(300);
  expect(writes).toHaveLength(2);
  stop = v.startVoice();
});
