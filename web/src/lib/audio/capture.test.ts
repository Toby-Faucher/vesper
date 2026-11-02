import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { CHECK_MS, createCapture, forgetSwapSkip, LOADING, READY_TIMEOUT_MS, REOPEN_TIMEOUT_MS, SILENT_REPORTS, SWAP_SKIPPED, type CaptureDeps } from './capture';
import { DEFAULT_AUDIO_SETTINGS, clampSettings } from './settings';

const flush = async () => { for (let i = 0; i < 20; i++) await Promise.resolve(); };

class FakeStream {
  tracks: any[];
  constructor(t: any[] = []) { this.tracks = [...t]; }
  addTrack(t: any) { this.tracks.push(t); }
  removeTrack(t: any) { this.tracks = this.tracks.filter((x) => x !== t); }
  getAudioTracks() { return this.tracks; }
  getTracks() { return this.tracks; }
}
// A constraint asked as { exact: false } is reported as false.
const flag = (v: ConstrainBoolean | undefined) => (typeof v === 'object' ? v.exact ?? v.ideal : v);
/** The request for the second, unfiltered track of the mic. */
const unfilteredAsk = (c: MediaStreamConstraints) => typeof (c.audio as MediaTrackConstraints).noiseSuppression === 'object';
const track = (label: string, c: MediaTrackConstraints = {}) => {
  const t = {
    kind: 'audio', label, enabled: true, muted: false, readyState: 'live', stop: vi.fn(),
    onended: null as any, onmute: null as any, onunmute: null as any,
    /** RMS the analyser hears from this track; 0 is a live track of zeros (seen on Edge). */
    level: 0.01,
    getSettings: () => ({ deviceId: `${label}-device`, noiseSuppression: flag(c.noiseSuppression) }),
    getConstraints: () => c,
    /** Chromium by default: resolves and changes nothing. `honours`: applies it (Firefox). `rejects`: OverconstrainedError. */
    honours: false,
    rejects: false,
    applyConstraints: vi.fn(async (n: MediaTrackConstraints) => {
      if (t.rejects) throw Object.assign(new Error('Cannot satisfy constraints'), { name: 'OverconstrainedError' });
      if (t.honours) c = n;
    }),
  };
  return t;
};
const node = () => ({ connect: vi.fn((n: unknown) => n), disconnect: vi.fn() });

const clock = { t: 0 };
function setup(over: Partial<CaptureDeps> = {}) {
  let micFail: Error | null = null;
  const mics: { c: MediaTrackConstraints; t: ReturnType<typeof track> }[] = [];
  const workers: any[] = [];
  const worklets: any[] = [];
  const dests: ReturnType<typeof track>[] = [];
  const ctx = {
    sampleRate: 48000,
    state: 'running',
    resume: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    createMediaStreamSource: vi.fn(() => node()),
    // Only the newest mic is ever measured.
    createAnalyser: vi.fn(() => ({ ...node(), fftSize: 4, getFloatTimeDomainData: (b: Float32Array) => b.fill(mics.at(-1)!.t.level) })),
    createGain: () => ({ ...node(), gain: { value: 1 } }),
    createMediaStreamDestination: () => {
      const t = track(`dest${dests.length}`);
      dests.push(t);
      return { ...node(), stream: new FakeStream([t]) };
    },
  };
  const deps: CaptureDeps = {
    getUserMedia: async (c) => {
      if (micFail) throw micFail;
      const t = track(`mic${mics.length}`, { ...(c.audio as MediaTrackConstraints) });
      mics.push({ c: c.audio as MediaTrackConstraints, t });
      return new FakeStream([t]) as unknown as MediaStream;
    },
    createContext: () => ctx as unknown as AudioContext,
    createWorkletNode: async () => {
      const n = { ...node(), port: { postMessage: vi.fn(), onmessage: null as any } };
      worklets.push(n);
      return n as unknown as AudioWorkletNode;
    },
    createWorker: () => {
      const w = { postMessage: vi.fn(), terminate: vi.fn(), onmessage: null as any, onerror: null as any };
      workers.push(w);
      return w as unknown as Worker;
    },
    readyTimeoutMs: READY_TIMEOUT_MS,
    startLive: true,
    now: () => clock.t,
    ...over,
  };
  const sender = { track: null as any, replaceTrack: vi.fn(async (t: any) => { sender.track = t; }) };
  return { failMic: (e: Error) => (micFail = e), deps, ctx, mics, workers, worklets, dests, sender, asSender: sender as unknown as RTCRtpSender };
}

beforeEach(() => vi.stubGlobal('MediaStream', FakeStream));
afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe('createCapture', () => {
  it('deepfilter: browser NS/AGC off, EC on, sends the denoised track, init then port after ready', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    // The browser filter is on while the model loads, and off once DeepFilterNet takes over.
    expect(t.mics[0].c).toMatchObject({ echoCancellation: true, noiseSuppression: true, autoGainControl: false });
    expect(cap.stream.getAudioTracks()).toEqual([t.dests[0]]);
    const w = t.workers[0];
    expect(w.postMessage).toHaveBeenCalledWith({ type: 'init', modelUrl: '/dsp/model.tar.gz', atten: 100, beta: 0.02 });
    // Before ready only the dynamics settings, no frame port. No boost by default, so no limiter.
    expect(t.worklets[0].port.postMessage.mock.calls).toEqual([[{ type: 'dynamics', limit: false, gate: false, thresholdDb: -50 }]]);
    const cb = vi.fn();
    cap.onModeChange(cb);
    expect(cb).toHaveBeenLastCalledWith('deepfilter', LOADING);
    w.onmessage({ data: { type: 'ready' } });
    expect(cb).toHaveBeenLastCalledWith('deepfilter', '');
    // Ready: the graph moves to a second track of the same mic without the browser filter, once it has sound; the sent track stays.
    await flush();
    expect(t.mics).toHaveLength(2);
    expect(t.mics[1].c).toMatchObject({ echoCancellation: true, noiseSuppression: { exact: false }, autoGainControl: false, deviceId: { exact: 'mic0-device' } });
    const mic = cap.getStats()!.mic;
    expect(mic).toMatchObject({ opens: 2, swap: 'reopened' });
    expect(mic.settings).not.toHaveProperty('deviceId');
    const [src0, src1] = t.ctx.createMediaStreamSource.mock.results.map((r) => r.value);
    expect(src0.disconnect).toHaveBeenCalled();
    expect(src1.connect).toHaveBeenCalledWith(expect.objectContaining({ gain: expect.anything() }));
    expect(t.mics[0].t.stop).not.toHaveBeenCalled(); // held until a worklet report shows the new track has sound
    t.worklets[0].port.onmessage({ data: { type: 'stats', state: 'playing', inputRms: 0.01 } });
    expect(t.mics[0].t.stop).toHaveBeenCalled();
    expect(t.mics[1].t.stop).not.toHaveBeenCalled();
    expect(cap.stream.getAudioTracks()).toEqual([t.dests[0]]);
    t.mics[0].t.onended?.(); // the old track's end is not a lost mic
    expect(cb).toHaveBeenLastCalledWith('deepfilter', '');
    expect(cap.getStats()?.mic.readyState).toBe('live');
    const [msg, transfer] = w.postMessage.mock.calls[1];
    expect(msg).toEqual({ type: 'port' });
    const [wmsg, wtransfer] = t.worklets[0].port.postMessage.mock.calls[1];
    expect(wmsg).toEqual({ type: 'port' });
    expect(transfer).toHaveLength(1);
    expect(wtransfer).toHaveLength(1);
    expect(transfer[0]).not.toBe(wtransfer[0]); // the two ends of one channel
    cap.close();
  });

  it('starts silent when startLive is false, and new tracks after a mic switch stay gated', async () => {
    const t = setup({ startLive: false });
    const cap = await createCapture(clampSettings({ ...DEFAULT_AUDIO_SETTINGS, mode: 'browser' }), t.deps);
    expect(t.mics[0].t.enabled).toBe(false);
    cap.setLive(true);
    expect(t.mics[0].t.enabled).toBe(true);
    cap.setLive(false);
    cap.setSettings(clampSettings({ ...DEFAULT_AUDIO_SETTINGS, mode: 'browser', micId: 'usb' }));
    await flush();
    expect(t.mics[1].t.enabled).toBe(false);
    cap.close();
  });

  it('worker error: falls back to browser NS, replaces the track, keeps mute, reports reason', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    cap.setSender(t.asSender);
    const seen: [string, string][] = [];
    cap.onModeChange((m, r) => seen.push([m, r]));
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush(); // mics[1]: the same mic without the browser filter, now under DeepFilterNet
    cap.setLive(false); // muted
    t.workers[0].onmessage({ data: { type: 'error', message: 'wasm panicked' } });
    await flush();
    expect(t.mics[2].c).toMatchObject({ echoCancellation: true, noiseSuppression: true });
    expect(t.sender.replaceTrack).toHaveBeenCalledWith(t.mics[2].t);
    expect(cap.stream.getAudioTracks()).toEqual([t.mics[2].t]);
    expect(t.mics[2].t.enabled).toBe(false);
    expect(seen.at(-1)).toEqual(['browser', expect.stringContaining('wasm panicked')]);
    expect(t.worklets[0].port.postMessage).toHaveBeenCalledWith({ type: 'stop' });
    expect(t.workers[0].terminate).toHaveBeenCalled();
    expect(t.mics[1].t.stop).toHaveBeenCalled();
    expect(t.dests[0].stop).toHaveBeenCalled();
    t.workers[0].onmessage({ data: { type: 'error', message: 'again' } }); // stale pipeline ignored
    await flush();
    expect(t.mics).toHaveLength(3);
    cap.close();
  });

  it('ready timeout is 20 s', () => expect(READY_TIMEOUT_MS).toBe(20_000));

  it('not ready within the ready timeout: falls back', async () => {
    vi.useFakeTimers();
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    const cb = vi.fn();
    cap.onModeChange(cb);
    await vi.advanceTimersByTimeAsync(READY_TIMEOUT_MS - 1);
    expect(cb).not.toHaveBeenCalledWith('browser', expect.anything());
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(cb).toHaveBeenCalledWith('browser', expect.stringMatching(/20 s/));
    cap.close();
  });

  it('worklet reports stall or slowness: falls back; fallback before setSender is applied on setSender', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    t.worklets[0].port.onmessage({ data: { type: 'error', message: 'denoiser stalled' } });
    await flush();
    t.sender.track = t.dests[0];
    cap.setSender(t.asSender);
    expect(t.sender.replaceTrack).toHaveBeenCalledWith(t.mics.at(-1)!.t);
    expect(t.mics.at(-1)!.c.noiseSuppression).toBe(true); // the browser fallback
  });

  it('graph setup failure falls back at once; late subscribers still learn the mode', async () => {
    const t = setup({ createWorkletNode: async () => { throw new Error('no AudioWorklet'); } });
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    expect(cap.stream.getAudioTracks()).toEqual([t.mics[1].t]);
    expect(t.mics[0].t.stop).toHaveBeenCalled();
    const cb = vi.fn();
    cap.onModeChange(cb);
    expect(cb).toHaveBeenCalledWith('browser', expect.stringContaining('no AudioWorklet'));
  });

  it('the browser filter stays on after ready when the user wants it on top; a failed reopen keeps the filtered track, named', async () => {
    const t = setup();
    const cap = await createCapture(clampSettings({ noiseSuppression: true }), t.deps);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    expect(t.mics).toHaveLength(1);
    cap.close();
    // The unfiltered mic will not open: DeepFilterNet goes on over the filtered track, never stopped, and the line says why.
    const u = setup();
    let n = 0;
    const deps = { ...u.deps, getUserMedia: async (c: MediaStreamConstraints) => { if (++n === 2) throw new Error('Could not start audio source'); return u.deps.getUserMedia(c); } };
    const cap2 = await createCapture(DEFAULT_AUDIO_SETTINGS, deps);
    const cb = vi.fn();
    cap2.onModeChange(cb);
    u.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    expect(cb).toHaveBeenLastCalledWith('deepfilter', 'swap skipped: the mic did not reopen without the browser filter: Could not start audio source');
    expect(n).toBe(2); // no other request
    expect(u.mics[0].t.stop).not.toHaveBeenCalled();
    expect(u.ctx.createMediaStreamSource.mock.results[0].value.disconnect).not.toHaveBeenCalled();
    expect(cap2.stream.getAudioTracks()).toEqual([u.dests[0]]);
    expect(cap2.getStats()!.mic).toMatchObject({ opens: 1, swap: 'skipped: the mic did not reopen without the browser filter: Could not start audio source' });
    cap2.close();
  });

  it('a browser that applies the constraint (Firefox) keeps its track: no reopen, no gap', async () => {
    const t = setup();
    const deps = { ...t.deps, getUserMedia: async (c: MediaStreamConstraints) => { const m = await t.deps.getUserMedia(c); t.mics.at(-1)!.t.honours = true; return m; } };
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, deps);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    expect(t.mics).toHaveLength(1);
    expect(t.mics[0].t.getSettings().noiseSuppression).toBe(false);
    expect(t.mics[0].t.applyConstraints).toHaveBeenCalledWith(expect.objectContaining({ noiseSuppression: { exact: false } }));
    expect(cap.getStats()!.mic).toMatchObject({ opens: 1, swap: 'applied-constraints' });
    expect(t.mics[0].t.stop).not.toHaveBeenCalled();
    cap.close();
  });

  it('with the browser filter on top, loading says only "loading"', async () => {
    const t = setup();
    const cap = await createCapture(clampSettings({ noiseSuppression: true }), t.deps);
    const cb = vi.fn();
    cap.onModeChange(cb);
    expect(cb).toHaveBeenLastCalledWith('deepfilter', 'loading');
    cap.close();
  });

  it('a reopen that does not answer in 3 s keeps the filtered track, named; a late answer is stopped', async () => {
    vi.useFakeTimers();
    const t = setup();
    let release!: () => void;
    const held = new Promise<void>((r) => (release = r));
    const deps = { ...t.deps, getUserMedia: async (c: MediaStreamConstraints) => { if (unfilteredAsk(c)) await held; return t.deps.getUserMedia(c); } };
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, deps);
    const cb = vi.fn();
    cap.onModeChange(cb);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await vi.advanceTimersByTimeAsync(REOPEN_TIMEOUT_MS - 1);
    expect(cb).toHaveBeenLastCalledWith('deepfilter', '');
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(cb).toHaveBeenLastCalledWith('deepfilter', 'swap skipped: the mic did not reopen without the browser filter: no answer in 3 s');
    expect(t.mics[0].t.stop).not.toHaveBeenCalled();
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(t.mics).toHaveLength(2);
    expect(t.mics[1].t.stop).toHaveBeenCalled();
    expect(cap.getStats()!.mic.readyState).toBe('live');
    expect(cap.stream.getAudioTracks()).toEqual([t.dests[0]]);
    cap.close();
  });

  it('mute and push to talk survive the reopen: the sent track does not change', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    cap.setSender(t.asSender);
    cap.setLive(false);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    expect(t.mics).toHaveLength(2);
    expect(cap.stream.getAudioTracks()).toEqual([t.dests[0]]);
    expect(t.dests[0].enabled).toBe(false);
    expect(t.sender.track).toBe(t.dests[0]);
    cap.close();
  });

  it('a mode change while the reopen runs: the new mode wins, the reopened track is stopped', async () => {
    const t = setup();
    let release!: () => void;
    const held = new Promise<void>((r) => (release = r));
    const deps = { ...t.deps, getUserMedia: async (c: MediaStreamConstraints) => { if (unfilteredAsk(c)) await held; return t.deps.getUserMedia(c); } };
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, deps);
    const cb = vi.fn();
    cap.onModeChange(cb);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    cap.setSettings(clampSettings({ mode: 'browser' }));
    await flush();
    const browser = t.mics[1];
    expect(browser.c.noiseSuppression).toBe(true);
    expect(cap.stream.getAudioTracks()).toEqual([browser.t]);
    release();
    await flush();
    const reopened = t.mics[2];
    expect(reopened.c.noiseSuppression).toEqual({ exact: false });
    expect(reopened.t.stop).toHaveBeenCalled();
    expect(cap.stream.getAudioTracks()).toEqual([browser.t]);
    expect(cb).toHaveBeenLastCalledWith('browser', '');
    cap.close();
  });

  it('make before break: the filtered track still runs when the unfiltered one is asked for', async () => {
    const t = setup();
    const stoppedFirst: boolean[] = [];
    const deps = { ...t.deps, getUserMedia: async (c: MediaStreamConstraints) => { if (t.mics.length) stoppedFirst.push(t.mics[0].t.stop.mock.calls.length > 0); return t.deps.getUserMedia(c); } };
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, deps);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    expect(stoppedFirst).toEqual([false]);
    cap.close();
  });

  it('a pipe stopped while the unfiltered track is asked for stops that track too', async () => {
    const t = setup();
    let release!: () => void;
    const held = new Promise<void>((r) => (release = r));
    const deps = { ...t.deps, getUserMedia: async (c: MediaStreamConstraints) => { if (t.mics.length) await held; return t.deps.getUserMedia(c); } };
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, deps);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush(); // the reopen is now waiting for the mic
    cap.close();
    release();
    await flush();
    expect(t.mics).toHaveLength(2);
    expect(t.mics[1].t.stop).toHaveBeenCalled();
  });

  it('browser and off modes use no worker; mic denial propagates and closes the context', async () => {
    const t = setup();
    await createCapture(clampSettings({ mode: 'browser' }), t.deps);
    await createCapture(clampSettings({ mode: 'off' }), t.deps);
    expect(t.workers).toHaveLength(0);
    expect(t.mics.map((m) => m.c.noiseSuppression)).toEqual([true, false]);
    const d = setup({ getUserMedia: async () => { throw new Error('NotAllowedError'); } });
    await expect(createCapture(DEFAULT_AUDIO_SETTINGS, d.deps)).rejects.toThrow('NotAllowedError');
    expect(d.ctx.close).toHaveBeenCalled();
  });

  it('setSettings: suppressor values go to the worker live; mode change rebuilds', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush(); // mics[1]: the unfiltered track after ready
    cap.setSettings(clampSettings({ attenDb: 30, postFilter: 0.01 }));
    expect(t.workers[0].postMessage).toHaveBeenLastCalledWith({ type: 'set', atten: 30, beta: 0.01 });
    expect(t.worklets[0].port.postMessage).toHaveBeenLastCalledWith({ type: 'dynamics', limit: false, gate: false, thresholdDb: -50 });
    cap.setSettings(clampSettings({ vad: { thresholdDb: -42, gate: true }, inputBoostDb: 3 })); // live, no rebuild
    expect(t.worklets[0].port.postMessage).toHaveBeenLastCalledWith({ type: 'dynamics', limit: true, gate: true, thresholdDb: -42 });
    expect(t.mics).toHaveLength(2);
    const cb = vi.fn();
    cap.onModeChange(cb);
    cap.setSettings(clampSettings({ mode: 'off' }));
    await flush();
    expect(t.mics[2].c.noiseSuppression).toBe(false);
    expect(cap.stream.getAudioTracks()).toEqual([t.mics[2].t]);
    expect(cb).toHaveBeenLastCalledWith('off', '');
    expect(t.workers[0].terminate).toHaveBeenCalled();
  });

  it('fallback mic request fails: dead pipeline stopped, microphone loss surfaced', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    const cb = vi.fn();
    cap.onModeChange(cb);
    t.failMic(new Error('NotFoundError'));
    t.workers[0].onmessage({ data: { type: 'error', message: 'boom' } });
    await flush();
    expect(t.workers[0].terminate).toHaveBeenCalled();
    expect(t.mics[0].t.stop).toHaveBeenCalled();
    expect(cb).toHaveBeenLastCalledWith('off', expect.stringMatching(/microphone lost: .*NotFoundError/));
  });

  it('replaceTrack rejects: the sent track is kept and the failure surfaced', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    t.sender.track = t.dests[0];
    cap.setSender(t.asSender);
    t.sender.replaceTrack.mockRejectedValueOnce(new Error('InvalidStateError'));
    const cb = vi.fn();
    cap.onModeChange(cb);
    t.workers[0].onmessage({ data: { type: 'error', message: 'boom' } });
    await flush();
    expect(t.dests[0].stop).not.toHaveBeenCalled();
    expect(t.mics[0].t.stop).not.toHaveBeenCalled();
    expect(t.mics[1].t.stop).toHaveBeenCalled();
    expect(cap.stream.getAudioTracks()).toEqual([t.dests[0]]);
    expect(cb).toHaveBeenLastCalledWith('deepfilter', expect.stringMatching(/track swap failed: .*InvalidStateError/));
  });

  it('context not at 48 kHz: falls back with a clear reason, no worker', async () => {
    const t = setup();
    t.ctx.sampleRate = 44100;
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    const cb = vi.fn();
    cap.onModeChange(cb);
    expect(cb).toHaveBeenCalledWith('browser', expect.stringContaining('44100'));
    expect(t.workers).toHaveLength(0);
    expect(t.mics).toHaveLength(1);
  });

  it('close stops everything and later timeouts do nothing', async () => {
    vi.useFakeTimers();
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    cap.close();
    expect(t.workers[0].terminate).toHaveBeenCalled();
    expect(t.worklets[0].port.postMessage).toHaveBeenCalledWith({ type: 'stop' });
    expect(t.mics[0].t.stop).toHaveBeenCalled();
    expect(t.ctx.close).toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(6000);
    expect(t.mics).toHaveLength(1);
  });

  for (const mode of ['deepfilter', 'browser'] as const) {
    it(`${mode}: the mic track ending mid-call is surfaced once; a stale pipe's end is ignored`, async () => {
      const t = setup();
      const cap = await createCapture(clampSettings({ ...DEFAULT_AUDIO_SETTINGS, mode }), t.deps);
      const seen: [string, string][] = [];
      cap.onModeChange((m, r) => seen.push([m, r]));
      seen.length = 0;
      // swap to a new mic: the old pipe's end must not alarm
      cap.setSettings(clampSettings({ ...DEFAULT_AUDIO_SETTINGS, mode, micId: 'usb' }));
      await flush();
      seen.length = 0;
      t.mics[0].t.onended?.();
      expect(seen).toEqual([]);
      t.mics[1].t.onended();
      t.mics[1].t.onended?.();
      expect(seen).toEqual([['off', 'microphone lost: ended']]);
      expect(t.mics[1].t.stop).toHaveBeenCalled();
      // any settings change retries the mic once the pipe is gone
      cap.setSettings(clampSettings({ ...DEFAULT_AUDIO_SETTINGS, mode: 'browser', micId: 'usb' }));
      await flush();
      expect(t.mics).toHaveLength(3);
      expect(seen.at(-1)?.[0]).toBe('browser');
      cap.close();
    });
  }

  it('a mute that persists is surfaced, a short one is not, and unmute clears it', async () => {
    vi.useFakeTimers();
    const t = setup();
    const cap = await createCapture(clampSettings({ ...DEFAULT_AUDIO_SETTINGS, mode: 'browser' }), t.deps);
    const seen: [string, string][] = [];
    cap.onModeChange((m, r) => seen.push([m, r]));
    seen.length = 0;
    const m = t.mics[0].t;
    m.muted = true; m.onmute();
    await vi.advanceTimersByTimeAsync(500);
    m.muted = false; m.onunmute();
    await vi.advanceTimersByTimeAsync(5000);
    expect(seen).toEqual([]);
    m.muted = true; m.onmute();
    await vi.advanceTimersByTimeAsync(5000);
    expect(seen).toEqual([['browser', 'microphone lost: muted by the system']]);
    m.muted = false; m.onunmute();
    expect(seen.at(-1)).toEqual(['browser', '']);
    expect(m.stop).not.toHaveBeenCalled();
    cap.close();
  });

  it('puts the boost gain before the worklet', async () => {
    const t = setup();
    const g = { ...node(), gain: { value: 1 } };
    t.ctx.createGain = () => g;
    await createCapture(clampSettings({ ...DEFAULT_AUDIO_SETTINGS, inputBoostDb: 6 }), t.deps);
    const src = t.ctx.createMediaStreamSource.mock.results[0].value;
    expect(src.connect).toHaveBeenCalledWith(g);
    expect(g.connect).toHaveBeenCalledWith(t.worklets[0]);
    expect(g.gain.value).toBeCloseTo(1.995, 2);
  });

  it('notes that input boost is ignored outside deepfilter mode', async () => {
    const t = setup();
    const browser = clampSettings({ ...DEFAULT_AUDIO_SETTINGS, mode: 'browser' });
    const cap = await createCapture(browser, t.deps);
    const seen: [string, string][] = [];
    cap.onModeChange((m, r) => seen.push([m, r]));
    expect(seen.at(-1)).toEqual(['browser', '']);
    cap.setSettings({ ...browser, inputBoostDb: 6 });
    expect(seen.at(-1)).toEqual(['browser', 'input boost applies only in deepfilter mode']);
    cap.setSettings({ ...browser, inputBoostDb: 6, mode: 'deepfilter' });
    await flush();
    expect(seen.at(-1)).toEqual(['deepfilter', LOADING]);
    cap.close();
  });

  it('stats: the latest worklet snapshot plus the mic track and context state', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    const mic = { readyState: 'live', muted: false, enabled: true, opens: 1, swap: 'none', settings: { noiseSuppression: true } };
    const context = { state: 'running', sampleRate: 48000 };
    expect(cap.getStats()).toEqual({ pipeline: null, ageMs: null, mic, context });
    const seen = vi.fn();
    const off = cap.onStats(seen);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    const { type: _, ...pipeline } = { type: 'stats', state: 'playing', inputRms: 0.1, underruns: 2 };
    clock.t = 1000;
    t.worklets[0].port.onmessage({ data: { type: 'stats', ...pipeline } });
    await flush();
    expect(t.mics).toHaveLength(2); // the unfiltered track after ready; a stats message is not a failure
    expect(cap.getStats()).toEqual({ pipeline, ageMs: 0, mic: { ...mic, opens: 2, swap: 'reopened', settings: { noiseSuppression: false } }, context }); // the reopened track
    expect(seen).toHaveBeenCalledWith({ pipeline, ageMs: 0, mic, context });
    clock.t = 3500;
    expect(cap.getStats()!.ageMs).toBe(2500); // age since the snapshot arrived
    cap.setLive(false);
    t.mics[1].t.muted = true;
    expect(cap.getStats()!.mic).toMatchObject({ readyState: 'live', muted: true, enabled: false }); // enabled is the sent track's gate
    off();
    t.worklets[0].port.onmessage({ data: { type: 'stats', ...pipeline } });
    expect(seen).toHaveBeenCalledTimes(1);
    cap.close();
    expect(cap.getStats()).toBeNull();
  });

  it('stats in browser mode: no pipeline numbers, the raw mic track state', async () => {
    const t = setup();
    const cap = await createCapture(clampSettings({ ...DEFAULT_AUDIO_SETTINGS, mode: 'browser' }), t.deps);
    t.mics[0].t.readyState = 'ended';
    expect(cap.getStats()).toEqual({
      pipeline: null, ageMs: null, mic: { readyState: 'ended', muted: false, enabled: true, settings: { noiseSuppression: true }, opens: 1, swap: 'none' }, context: { state: 'running', sampleRate: 48000 },
    });
    cap.close();
  });
  describe('a saved mic that is gone', () => {
    // Firefox rejects with a MediaStreamError, which is no Error or DOMException: only the name counts.
    const named = (name: string) => ({ name, message: 'Constraints could not be satisfied.' });
    // The first n requests reject with `first`; the rest reach the fake mic. Records every constraint asked for.
    function failing(first: unknown, n = 1) {
      const t = setup();
      const asked: MediaTrackConstraints[] = [];
      const onMicFallback = vi.fn();
      const deps = {
        ...t.deps,
        onMicFallback,
        getUserMedia: async (c: MediaStreamConstraints) => {
          asked.push(c.audio as MediaTrackConstraints);
          if (asked.length <= n) throw first;
          return t.deps.getUserMedia(c);
        },
      };
      return { t, asked, deps, onMicFallback };
    }
    const saved = clampSettings({ mode: 'browser', micId: 'gone' });

    for (const name of ['OverconstrainedError', 'NotFoundError']) {
      it(`${name}: asks once more without the device, then forgets the id`, async () => {
        const { t, asked, deps, onMicFallback } = failing(named(name));
        const cap = await createCapture(saved, deps);
        expect(asked.map((c) => c.deviceId)).toEqual([{ exact: 'gone' }, undefined]);
        expect(asked[1]).not.toHaveProperty('deviceId');
        expect(onMicFallback).toHaveBeenCalledTimes(1);
        expect(cap.stream.getAudioTracks()).toEqual([t.mics[0].t]);
        // The capture uses the default now: settings with no mic id do not rebuild it, and the old id is not asked again.
        cap.setSettings(clampSettings({ mode: 'browser' }));
        await flush();
        expect(asked).toHaveLength(2);
        cap.close();
      });
    }

    it('deepfilter mode falls back in the same way', async () => {
      const { asked, deps, onMicFallback } = failing(named('OverconstrainedError'));
      const cap = await createCapture(clampSettings({ micId: 'gone' }), deps);
      expect(asked).toHaveLength(2);
      expect(onMicFallback).toHaveBeenCalledTimes(1);
      cap.close();
    });

    for (const name of ['NotAllowedError', 'NotReadableError', 'AbortError']) {
      it(`${name} is not retried and the id is kept`, async () => {
        const { asked, deps, onMicFallback } = failing(named(name));
        await expect(createCapture(saved, deps)).rejects.toMatchObject({ name });
        expect(asked).toHaveLength(1);
        expect(onMicFallback).not.toHaveBeenCalled();
      });
    }

    it('in a call: switching to a mic that is gone swaps in the default one', async () => {
      const t = setup();
      const onMicFallback = vi.fn();
      const asked: MediaTrackConstraints[] = [];
      const deps = {
        ...t.deps,
        onMicFallback,
        getUserMedia: async (c: MediaStreamConstraints) => {
          asked.push(c.audio as MediaTrackConstraints);
          if ((c.audio as MediaTrackConstraints).deviceId) throw named('NotFoundError');
          return t.deps.getUserMedia(c);
        },
      };
      const cap = await createCapture(clampSettings({ mode: 'browser' }), deps);
      cap.setSender(t.asSender);
      const cb = vi.fn();
      cap.onModeChange(cb);
      cap.setSettings(saved);
      await flush();
      expect(asked.map((c) => c.deviceId)).toEqual([undefined, { exact: 'gone' }, undefined]);
      expect(onMicFallback).toHaveBeenCalledTimes(1);
      expect(t.sender.replaceTrack).toHaveBeenLastCalledWith(t.mics[1].t);
      expect(cb).toHaveBeenLastCalledWith('browser', '');
      cap.close();
    });

    it('another mic picked while the default was asked for: that choice stands', async () => {
      const t = setup();
      const onMicFallback = vi.fn();
      let release!: () => void;
      let n = 0;
      const deps = {
        ...t.deps,
        onMicFallback,
        getUserMedia: async (c: MediaStreamConstraints) => {
          n++;
          if (n === 2) throw named('NotFoundError'); // the switch to 'gone'
          if (n === 3) await new Promise<void>((r) => (release = r)); // its default retry, slow
          return t.deps.getUserMedia(c);
        },
      };
      const cap = await createCapture(clampSettings({ mode: 'browser' }), deps);
      cap.setSettings(saved);
      await flush();
      cap.setSettings(clampSettings({ mode: 'browser', micId: 'usb' })); // picked while the retry runs
      release();
      await flush();
      expect(onMicFallback).not.toHaveBeenCalled();
      const usb = t.mics.find((m) => (m.c.deviceId as { exact?: string } | undefined)?.exact === 'usb')!;
      expect(cap.stream.getAudioTracks()).toEqual([usb.t]); // the late default mic was a stale build, dropped
      cap.close();
    });

    it('the default mic failing too: one retry only, its error surfaces', async () => {
      const { asked, deps, onMicFallback } = failing(named('NotFoundError'), 99);
      await expect(createCapture(saved, deps)).rejects.toMatchObject({ name: 'NotFoundError' });
      expect(asked).toHaveLength(2);
      expect(onMicFallback).not.toHaveBeenCalled();
    });

    it('no saved id: a missing mic is not retried', async () => {
      const { asked, deps } = failing(named('NotFoundError'));
      await expect(createCapture(clampSettings({ mode: 'browser' }), deps)).rejects.toMatchObject({ name: 'NotFoundError' });
      expect(asked).toHaveLength(1);
    });
  });
});

describe('retry after the mic ended', () => {
  const named = (name: string) => ({ name, message: 'gone' });
  for (const mode of ['deepfilter', 'browser'] as const) {
    it(`${mode}: gets the mic again in the same call and hands the sender the new track`, async () => {
      const t = setup();
      const cap = await createCapture(clampSettings({ mode }), t.deps);
      cap.setSender(t.asSender);
      await flush();
      const seen: [string, string][] = [];
      cap.onModeChange((m, r) => seen.push([m, r]));
      t.mics[0].t.onended();
      expect(seen.at(-1)).toEqual(['off', 'microphone lost: ended']);
      expect(await cap.retry()).toBe(true);
      expect(t.mics).toHaveLength(2);
      expect(seen.at(-1)?.[0]).toBe(mode);
      const sent = mode === 'deepfilter' ? t.dests[1] : t.mics[1].t;
      expect(t.sender.track).toBe(sent);
      expect(cap.stream.getAudioTracks()).toEqual([sent]);
      cap.close();
    });
  }

  it('a failed retry says why and keeps the call; the next one can work', async () => {
    const t = setup();
    const cap = await createCapture(clampSettings({ mode: 'browser' }), t.deps);
    const seen: [string, string][] = [];
    cap.onModeChange((m, r) => seen.push([m, r]));
    t.mics[0].t.onended();
    t.failMic(Object.assign(new Error('Permission denied'), { name: 'NotAllowedError' }));
    expect(await cap.retry()).toBe(false);
    expect(seen.at(-1)).toEqual(['off', 'microphone lost: NotAllowedError: Permission denied']);
    t.failMic(null as unknown as Error);
    expect(await cap.retry()).toBe(true);
    expect(seen.at(-1)).toEqual(['browser', '']);
    cap.close();
  });

  it('a saved mic that went away falls back to the default one (the stale-id path)', async () => {
    const t = setup();
    const onMicFallback = vi.fn();
    let gone = false;
    const deps = {
      ...t.deps,
      onMicFallback,
      getUserMedia: async (c: MediaStreamConstraints) => {
        if (gone && (c.audio as MediaTrackConstraints).deviceId) throw named('NotFoundError');
        return t.deps.getUserMedia(c);
      },
    };
    const cap = await createCapture(clampSettings({ mode: 'browser', micId: 'usb' }), deps);
    gone = true;
    t.mics[0].t.onended();
    expect(await cap.retry()).toBe(true);
    expect(onMicFallback).toHaveBeenCalledTimes(1);
    expect(t.mics[1].c.deviceId).toBeUndefined();
    cap.close();
  });
});

describe('fail-safe: the reopened mic is live but silent (seen on Edge)', () => {
  beforeEach(() => forgetSwapSkip());
  const stats = (t: ReturnType<typeof setup>, inputRms: number) =>
    t.worklets.at(-1).port.onmessage({ data: { type: 'stats', state: 'playing', inputRms } });
  const src = (t: ReturnType<typeof setup>, i: number) => t.ctx.createMediaStreamSource.mock.results[i].value;
  async function swapped(t: ReturnType<typeof setup>) {
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    const cb = vi.fn();
    cap.onModeChange(cb);
    stats(t, 0.01); // the filtered track had sound
    t.workers.at(-1).onmessage({ data: { type: 'ready' } });
    await flush();
    expect(t.mics.at(-1)!.c.noiseSuppression).toEqual({ exact: false });
    return { cap, cb };
  }
  // The second track is opened silent: the analyser hears nothing for CHECK_MS.
  const silentSecond = (t: ReturnType<typeof setup>) => ({
    ...t.deps,
    getUserMedia: async (c: MediaStreamConstraints) => { const m = await t.deps.getUserMedia(c); if (unfilteredAsk(c)) t.mics.at(-1)!.t.level = 0; return m; },
  });

  it('a second track of zeros is dropped before the graph moves: the filtered track never stops, no third request', async () => {
    vi.useFakeTimers();
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, silentSecond(t));
    const cb = vi.fn();
    cap.onModeChange(cb);
    stats(t, 0.01);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await vi.advanceTimersByTimeAsync(CHECK_MS - 100);
    expect(t.mics[1].t.stop).not.toHaveBeenCalled(); // still listening
    await vi.advanceTimersByTimeAsync(100);
    expect(t.mics[1].t.stop).toHaveBeenCalled();
    expect(src(t, 1).connect).not.toHaveBeenCalledWith(expect.objectContaining({ gain: expect.anything() })); // never fed the denoiser
    expect(src(t, 0).disconnect).not.toHaveBeenCalled();
    expect(t.mics[0].t.stop).not.toHaveBeenCalled();
    expect(t.mics).toHaveLength(2);
    expect(cb).toHaveBeenLastCalledWith('deepfilter', SWAP_SKIPPED);
    expect(cap.getStats()!.mic).toMatchObject({ opens: 2, swap: 'skipped: reopened mic was silent', readyState: 'live' });
    cap.close();
    expect(t.mics[0].t.stop).toHaveBeenCalled();
  });

  it('the applied constraint rejected (Chromium): the parallel track takes over', async () => {
    const t = setup();
    const deps = { ...t.deps, getUserMedia: async (c: MediaStreamConstraints) => { const m = await t.deps.getUserMedia(c); t.mics.at(-1)!.t.rejects = true; return m; } };
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, deps);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    expect(t.mics[0].t.applyConstraints).toHaveBeenCalled();
    expect(cap.getStats()!.mic).toMatchObject({ opens: 2, swap: 'reopened', settings: { noiseSuppression: false } });
    cap.close();
  });

  it('goes back to the held filtered track without a new request, says why, and never swaps again this session', async () => {
    const t = setup();
    const { cap, cb } = await swapped(t);
    for (let i = 1; i < SILENT_REPORTS; i++) stats(t, 0);
    await flush();
    expect(cb).not.toHaveBeenLastCalledWith('deepfilter', SWAP_SKIPPED); // not yet
    src(t, 0).connect.mockClear();
    stats(t, 0);
    await flush();
    expect(t.mics).toHaveLength(2); // no getUserMedia: the first track was kept
    expect(t.mics[0].t.stop).not.toHaveBeenCalled();
    expect(t.mics[1].t.stop).toHaveBeenCalled();
    expect(src(t, 0).connect).toHaveBeenCalledWith(expect.objectContaining({ gain: expect.anything() })); // fed again
    expect(src(t, 1).disconnect).toHaveBeenCalled();
    expect(cb).toHaveBeenLastCalledWith('deepfilter', SWAP_SKIPPED);
    expect(cap.getStats()!.mic).toMatchObject({ readyState: 'live', settings: { noiseSuppression: true } });
    expect(cap.stream.getAudioTracks()).toEqual([t.dests[0]]); // the sent track stays
    t.mics[0].t.onended?.(); // the restored track is watched again
    expect(cb).toHaveBeenLastCalledWith('off', 'microphone lost: ended');
    cap.close();
    // The next capture (a rejoin, the Settings test) keeps the browser filter from the start.
    const u = setup();
    const cap2 = await createCapture(DEFAULT_AUDIO_SETTINGS, u.deps);
    const cb2 = vi.fn();
    cap2.onModeChange(cb2);
    u.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    expect(u.mics).toHaveLength(1);
    expect(cb2).toHaveBeenLastCalledWith('deepfilter', SWAP_SKIPPED);
    expect(cap2.getStats()!.mic.swap).toBe('skipped: earlier this session');
    cap2.close();
  });

  it('one report with sound ends the watch and releases the filtered track: later silence is the user, not the swap', async () => {
    const t = setup();
    const { cap, cb } = await swapped(t);
    stats(t, 0);
    expect(t.mics[0].t.stop).not.toHaveBeenCalled();
    stats(t, 0.01);
    expect(t.mics[0].t.stop).toHaveBeenCalled();
    for (let i = 0; i < SILENT_REPORTS + 2; i++) stats(t, 0);
    await flush();
    expect(t.mics).toHaveLength(2);
    expect(t.mics[1].t.stop).not.toHaveBeenCalled();
    expect(cb).toHaveBeenLastCalledWith('deepfilter', '');
    cap.close();
  });

  it('a mic that was already silent before the swap is not blamed on it', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    stats(t, 0);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    await flush();
    expect(t.mics[0].t.stop).toHaveBeenCalled(); // nothing to compare with, so it is not held
    for (let i = 0; i < SILENT_REPORTS + 2; i++) stats(t, 0);
    await flush();
    expect(t.mics[1].t.stop).not.toHaveBeenCalled();
    cap.close();
  });

  it('retry(true), from the "no sound" banner, ends the old mic first and rebuilds without the swap', async () => {
    const t = setup();
    const stoppedFirst: boolean[] = [];
    const gum = t.deps.getUserMedia;
    t.deps.getUserMedia = async (c) => { if (t.mics.length === 2) stoppedFirst.push(t.mics[1].t.stop.mock.calls.length > 0); return gum(c); };
    const { cap, cb } = await swapped(t);
    expect(await cap.retry(true)).toBe(true);
    expect(stoppedFirst).toEqual([true]); // a running track would hand the new request its (silent) source
    t.workers.at(-1).onmessage({ data: { type: 'ready' } });
    await flush();
    expect(t.mics).toHaveLength(3);
    expect(t.mics[2].c.noiseSuppression).toBe(true);
    expect(cb).toHaveBeenLastCalledWith('deepfilter', SWAP_SKIPPED);
    cap.close();
  });

  it('a suspended AudioContext is resumed', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    t.ctx.resume.mockClear();
    t.ctx.state = 'suspended';
    (t.ctx as any).onstatechange();
    expect(t.ctx.resume).toHaveBeenCalledTimes(1);
    t.ctx.state = 'running';
    (t.ctx as any).onstatechange();
    expect(t.ctx.resume).toHaveBeenCalledTimes(1);
    cap.close();
  });
});
