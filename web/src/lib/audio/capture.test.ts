import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createCapture, type CaptureDeps } from './capture';
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
const track = (label: string) => ({ kind: 'audio', label, enabled: true, stop: vi.fn() });
const node = () => ({ connect: vi.fn((n: unknown) => n), disconnect: vi.fn() });

function setup(over: Partial<CaptureDeps> = {}) {
  const mics: { c: MediaTrackConstraints; t: ReturnType<typeof track> }[] = [];
  const workers: any[] = [];
  const worklets: any[] = [];
  const dests: ReturnType<typeof track>[] = [];
  const ctx = {
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
    readyTimeoutMs: 5000,
    ...over,
  };
  const sender = { track: null as any, replaceTrack: vi.fn(async (t: any) => { sender.track = t; }) };
  return { deps, ctx, mics, workers, worklets, dests, sender, asSender: sender as unknown as RTCRtpSender };
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
    w.onmessage({ data: { type: 'ready' } });
    const [msg, transfer] = w.postMessage.mock.calls[1];
    expect(msg).toEqual({ type: 'port' });
    const [wmsg, wtransfer] = t.worklets[0].port.postMessage.mock.calls[0];
    expect(wmsg).toEqual({ type: 'port' });
    expect(transfer).toHaveLength(1);
    expect(wtransfer).toHaveLength(1);
    expect(transfer[0]).not.toBe(wtransfer[0]); // the two ends of one channel
    cap.close();
  });

  it('worker error: falls back to browser NS, replaces the track, keeps mute, reports reason', async () => {
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    cap.setSender(t.asSender);
    const seen: [string, string][] = [];
    cap.onModeChange((m, r) => seen.push([m, r]));
    t.workers[0].onmessage({ data: { type: 'ready' } });
    cap.stream.getAudioTracks()[0].enabled = false; // muted
    t.workers[0].onmessage({ data: { type: 'error', message: 'wasm panicked' } });
    await flush();
    expect(t.mics[1].c).toMatchObject({ echoCancellation: true, noiseSuppression: true });
    expect(t.sender.replaceTrack).toHaveBeenCalledWith(t.mics[1].t);
    expect(cap.stream.getAudioTracks()).toEqual([t.mics[1].t]);
    expect(t.mics[1].t.enabled).toBe(false);
    expect(seen).toEqual([['browser', expect.stringContaining('wasm panicked')]]);
    expect(t.workers[0].terminate).toHaveBeenCalled();
    expect(t.mics[0].t.stop).toHaveBeenCalled();
    expect(t.dests[0].stop).toHaveBeenCalled();
    t.workers[0].onmessage({ data: { type: 'error', message: 'again' } }); // stale pipeline ignored
    await flush();
    expect(t.mics).toHaveLength(2);
    cap.close();
  });

  it('not ready within 5 s: falls back', async () => {
    vi.useFakeTimers();
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    const cb = vi.fn();
    cap.onModeChange(cb);
    await vi.advanceTimersByTimeAsync(4999);
    expect(cb).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    await flush();
    expect(cb).toHaveBeenCalledWith('browser', expect.stringMatching(/5 s/));
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
    expect(cb).toHaveBeenCalledWith('off', 'settings');
    expect(t.workers[0].terminate).toHaveBeenCalled();
  });

  it('close stops everything and later timeouts do nothing', async () => {
    vi.useFakeTimers();
    const t = setup();
    const cap = await createCapture(DEFAULT_AUDIO_SETTINGS, t.deps);
    cap.close();
    expect(t.workers[0].terminate).toHaveBeenCalled();
    expect(t.mics[0].t.stop).toHaveBeenCalled();
    expect(t.ctx.close).toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(6000);
    expect(t.mics).toHaveLength(1);
  });
});
