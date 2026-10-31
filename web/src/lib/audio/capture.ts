import type { DspOut } from './dsp-handler';
import type { DenoiseMode } from './mode';
import type { AudioSettings } from './settings';
import workletUrl from './denoise.worklet.ts?worker&url';

export interface CaptureDeps {
  getUserMedia(c: MediaStreamConstraints): Promise<MediaStream>;
  /** Called synchronously by createCapture, so call createCapture inside the user gesture. */
  createContext(): AudioContext;
  createWorkletNode(ctx: AudioContext): Promise<AudioWorkletNode>;
  createWorker(): Worker;
  readyTimeoutMs: number;
}

const modules = new WeakMap<BaseAudioContext, Promise<void>>();
const browserDeps: CaptureDeps = {
  getUserMedia: (c) => navigator.mediaDevices.getUserMedia(c),
  createContext: () => new AudioContext({ sampleRate: 48000, latencyHint: 'interactive' }),
  createWorkletNode: async (ctx) => {
    if (!modules.has(ctx)) modules.set(ctx, ctx.audioWorklet.addModule(workletUrl));
    await modules.get(ctx);
    return new AudioWorkletNode(ctx, 'vesper-denoise', {
      numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1], channelCount: 1, channelCountMode: 'explicit',
    });
  },
  createWorker: () => new Worker(new URL('./dsp.worker.ts', import.meta.url), { type: 'module' }),
  readyTimeoutMs: 5000,
};

type Pipe = { mode: DenoiseMode; track: MediaStreamTrack; stop(): void; set?(s: AudioSettings): void };
const stopAll = (m: MediaStream) => m.getTracks().forEach((t) => t.stop());

/**
 * deepfilter: mic -> AudioWorklet <-MessageChannel-> DSP worker, worklet -> gain -> MediaStreamDestination.
 * browser/off: the raw mic track, so the fallback does not depend on the AudioContext or worklet.
 * `stream` is stable; its one track is swapped (and replaceTrack'd on the sender) when the mode changes.
 */
export async function createCapture(s: AudioSettings, deps: Partial<CaptureDeps> = {}) {
  const d = { ...browserDeps, ...deps };
  const ctx = d.createContext();
  void ctx.resume();
  const stream = new MediaStream();
  const cbs: ((m: DenoiseMode, reason: string) => void)[] = [];
  let last: [DenoiseMode, string] | null = null; // replayed to late subscribers
  let pipe: Pipe | null = null;
  let sender: RTCRtpSender | null = null;
  let gen = 0;
  let closed = false;

  const mic = (mode: DenoiseMode) =>
    d.getUserMedia({
      audio: {
        echoCancellation: true,
        noiseSuppression: mode === 'browser' || (mode === 'deepfilter' && s.noiseSuppression),
        autoGainControl: s.autoGain,
        channelCount: 1,
        sampleRate: 48000,
        ...(s.micId ? { deviceId: { exact: s.micId } } : {}),
      },
    });

  async function plain(mode: DenoiseMode): Promise<Pipe> {
    const m = await mic(mode);
    return { mode, track: m.getAudioTracks()[0], stop: () => stopAll(m) };
  }

  async function deep(): Promise<Pipe> {
    const m = await mic('deepfilter');
    let dead = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let node: AudioWorkletNode | undefined;
    let worker: Worker | undefined;
    let ready = false;
    const nodes: AudioNode[] = [];
    let out: MediaStreamAudioDestinationNode | undefined;
    const stop = () => {
      dead = true;
      clearTimeout(timer);
      worker?.terminate();
      nodes.forEach((n) => n.disconnect());
      stopAll(m);
      if (out) stopAll(out.stream);
    };
    const fail = (reason: string) => {
      if (dead) return;
      dead = true;
      void switchTo('browser', reason).catch((e) => console.error('denoiser fallback failed', e));
    };
    try {
      node = await d.createWorkletNode(ctx);
      const src = ctx.createMediaStreamSource(m);
      const gain = ctx.createGain();
      gain.gain.value = 10 ** (s.inputBoostDb / 20);
      out = ctx.createMediaStreamDestination();
      nodes.push(src, node, gain);
      src.connect(node).connect(gain).connect(out);
      const w = (worker = d.createWorker());
      const n = node;
      const set = () => w.postMessage({ type: 'set', atten: s.attenDb, beta: s.postFilter });
      timer = setTimeout(() => fail(`denoiser not ready after ${d.readyTimeoutMs / 1000} s`), d.readyTimeoutMs);
      w.onmessage = ({ data }: MessageEvent<DspOut>) => {
        if (data.type === 'error') return fail(`denoiser error: ${data.message}`);
        if (data.type !== 'ready' || ready) return;
        ready = true;
        clearTimeout(timer);
        const ch = new MessageChannel(); // audio frames flow worklet <-> worker, never via this thread
        w.postMessage({ type: 'port' }, [ch.port1]);
        n.port.postMessage({ type: 'port' }, [ch.port2]);
        set(); // settings may have changed while the model loaded
      };
      w.onerror = (e) => fail(`denoiser worker failed: ${e.message ?? ''}`);
      n.port.onmessage = ({ data }: MessageEvent<{ type: 'error'; message: string }>) => fail(data.message);
      w.postMessage({ type: 'init', modelUrl: '/dsp/model.tar.gz', atten: s.attenDb, beta: s.postFilter });
      return {
        mode: 'deepfilter',
        track: out.stream.getAudioTracks()[0],
        stop,
        set: (n) => {
          gain.gain.value = 10 ** (n.inputBoostDb / 20);
          if (ready) set();
        },
      };
    } catch (e) {
      stop();
      throw e;
    }
  }

  async function switchTo(mode: DenoiseMode, reason: string) {
    const g = ++gen;
    let next: Pipe;
    try {
      next = mode === 'deepfilter' ? await deep() : await plain(mode);
    } catch (e) {
      if (mode !== 'deepfilter') throw e;
      reason = `denoiser unavailable: ${e instanceof Error ? e.message : String(e)}`;
      next = await plain('browser');
    }
    if (g !== gen || closed) return next.stop();
    const old = pipe;
    pipe = next;
    const prev = stream.getAudioTracks()[0];
    if (prev) {
      next.track.enabled = prev.enabled; // keep mute across the swap
      stream.removeTrack(prev);
    }
    stream.addTrack(next.track);
    sender?.replaceTrack(next.track).catch((e) => console.error('replaceTrack failed', e));
    old?.stop();
    if (old || next.mode !== mode) {
      last = [next.mode, reason];
      cbs.forEach((cb) => cb(next.mode, reason));
    }
  }

  try {
    await switchTo(s.mode, 'initial');
  } catch (e) {
    void ctx.close().catch(() => {});
    throw e;
  }

  return {
    stream,
    setSender(snd: RTCRtpSender) {
      sender = snd;
      const cur = stream.getAudioTracks()[0];
      if (snd.track !== cur) snd.replaceTrack(cur).catch((e) => console.error('replaceTrack failed', e));
    },
    setSettings(n: AudioSettings) {
      const rebuild =
        n.mode !== s.mode || n.micId !== s.micId || n.noiseSuppression !== s.noiseSuppression || n.autoGain !== s.autoGain;
      s = n;
      if (rebuild) void switchTo(n.mode, 'settings').catch((e) => console.error('capture switch failed', e));
      else pipe?.set?.(n);
    },
    onModeChange(cb: (m: DenoiseMode, reason: string) => void) {
      cbs.push(cb);
      if (last) cb(...last);
    },
    close() {
      closed = true;
      gen++;
      pipe?.stop();
      pipe = null;
      void ctx.close().catch(() => {});
    },
  };
}

export type Capture = Awaited<ReturnType<typeof createCapture>>;
