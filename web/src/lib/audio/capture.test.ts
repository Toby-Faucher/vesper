import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createCapture, READY_TIMEOUT_MS, type CaptureDeps } from './capture';
import { DEFAULT_AUDIO_SETTINGS, clampSettings } from './settings';

const flush = async () => { for (let i = 0; i < 10; i++) await Promise.resolve(); };

class FakeStream {
  tracks: any[];
  constructor(t: any[] = []) { this.tracks = [...t]; }
  addTrack(t: any) { this.tracks.push(t); }
  removeTrack(t: any) { this.tracks = this.tracks.filter((x) => x !== t); }
  getAudioTracks() { return this.tracks; }
  getTracks() { return this.tracks; }
}
const track = (label: string) => ({
  kind: 'audio', label, enabled: true, muted: false, readyState: 'live', stop: vi.fn(),
  onended: null as any, onmute: null as any, onunmute: null as any,
});
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
      const t = track(`mic${mics.length}`);
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
    expect(t.mics[0].c).toMatchObject({ echoCancellation: true, noiseSuppression: false, autoGainControl: false });
    expect(cap.stream.getAudioTracks()).toEqual([t.dests[0]]);
    const w = t.workers[0];
    expect(w.postMessage).toHaveBeenCalledWith({ type: 'init', modelUrl: '/dsp/model.tar.gz', atten: 100, beta: 0.02 });
    expect(t.worklets[0].port.postMessage).not.toHaveBeenCalled(); // no frames before ready
    const cb = vi.fn();
    cap.onModeChange(cb);
    expect(cb).toHaveBeenLastCalledWith('deepfilter', 'loading, unfiltered');
    w.onmessage({ data: { type: 'ready' } });
    expect(cb).toHaveBeenLastCalledWith('deepfilter', '');
    const [msg, transfer] = w.postMessage.mock.calls[1];
    expect(msg).toEqual({ type: 'port' });
    const [wmsg, wtransfer] = t.worklets[0].port.postMessage.mock.calls[0];
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
    cap.setLive(false); // muted
    t.workers[0].onmessage({ data: { type: 'error', message: 'wasm panicked' } });
    await flush();
    expect(t.mics[1].c).toMatchObject({ echoCancellation: true, noiseSuppression: true });
    expect(t.sender.replaceTrack).toHaveBeenCalledWith(t.mics[1].t);
    expect(cap.stream.getAudioTracks()).toEqual([t.mics[1].t]);
    expect(t.mics[1].t.enabled).toBe(false);
    expect(seen.at(-1)).toEqual(['browser', expect.stringContaining('wasm panicked')]);
    expect(t.worklets[0].port.postMessage).toHaveBeenCalledWith({ type: 'stop' });
    expect(t.workers[0].terminate).toHaveBeenCalled();
    expect(t.mics[0].t.stop).toHaveBeenCalled();
    expect(t.dests[0].stop).toHaveBeenCalled();
    t.workers[0].onmessage({ data: { type: 'error', message: 'again' } }); // stale pipeline ignored
    await flush();
    expect(t.mics).toHaveLength(2);
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
    expect(t.sender.replaceTrack).toHaveBeenCalledWith(t.mics[1].t);
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
    cap.setSettings(clampSettings({ attenDb: 30, postFilter: 0.01 }));
    expect(t.workers[0].postMessage).toHaveBeenLastCalledWith({ type: 'set', atten: 30, beta: 0.01 });
    expect(t.mics).toHaveLength(1);
    const cb = vi.fn();
    cap.onModeChange(cb);
    cap.setSettings(clampSettings({ mode: 'off' }));
    await flush();
    expect(t.mics[1].c.noiseSuppression).toBe(false);
    expect(cap.stream.getAudioTracks()).toEqual([t.mics[1].t]);
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
    expect(seen.at(-1)).toEqual(['deepfilter', 'loading, unfiltered']);
    cap.close();
  });

  it('stats: the latest worklet snapshot plus the mic track and context state', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    const mic = { readyState: 'live', muted: false, enabled: true };
    const context = { state: 'running', sampleRate: 48000 };
    expect(cap.getStats()).toEqual({ pipeline: null, ageMs: null, mic, context });
    const seen = vi.fn();
    const off = cap.onStats(seen);
    t.workers[0].onmessage({ data: { type: 'ready' } });
    const { type: _, ...pipeline } = { type: 'stats', state: 'playing', inputRms: 0.1, underruns: 2 };
    clock.t = 1000;
    t.worklets[0].port.onmessage({ data: { type: 'stats', ...pipeline } });
    await flush();
    expect(t.mics).toHaveLength(1); // a stats message is not a failure
    expect(cap.getStats()).toEqual({ pipeline, ageMs: 0, mic, context });
    expect(seen).toHaveBeenCalledWith({ pipeline, ageMs: 0, mic, context });
    clock.t = 3500;
    expect(cap.getStats()!.ageMs).toBe(2500); // age since the snapshot arrived
    cap.setLive(false);
    t.mics[0].t.muted = true;
    expect(cap.getStats()!.mic).toEqual({ readyState: 'live', muted: true, enabled: false }); // enabled is the sent track's gate
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
      pipeline: null, ageMs: null, mic: { readyState: 'ended', muted: false, enabled: true }, context: { state: 'running', sampleRate: 48000 },
    });
    cap.close();
  });
});
