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

type Pipe = { mode: DenoiseMode; track: MediaStreamTrack; stop(): void; set?(s: AudioSettings): void; status?: string };
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
    if (ctx.sampleRate !== 48000) throw new Error(`AudioContext runs at ${ctx.sampleRate} Hz, DeepFilterNet needs 48000`);
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
      node?.port.postMessage({ type: 'stop' });
      nodes.forEach((n) => n.disconnect());
      stopAll(m);
      if (out) stopAll(out.stream);
    };
    const fail = (reason: string) => {
      if (dead) return;
      dead = true;
      void switchTo('browser', reason).catch(lost);
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
        me.status = '';
        if (pipe === me) notify('deepfilter', '');
      };
      w.onerror = (e) => fail(`denoiser worker failed: ${e.message ?? ''}`);
      n.port.onmessage = ({ data }: MessageEvent<{ type: 'error'; message: string }>) => fail(data.message);
      w.postMessage({ type: 'init', modelUrl: '/dsp/model.tar.gz', atten: s.attenDb, beta: s.postFilter });
      const me: Pipe = {
        mode: 'deepfilter',
        status: 'loading, unfiltered',
        track: out.stream.getAudioTracks()[0],
        stop,
        set: (n) => {
          gain.gain.value = 10 ** (n.inputBoostDb / 20);
          if (ready) set();
        },
      };
      return me;
    } catch (e) {
      stop();
      throw e;
    }
  }

  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const lost = (e: unknown) => notify('off', `microphone lost: ${msg(e)}`);
  const notify = (m: DenoiseMode, reason: string) => {
    last = [m, reason];
    cbs.forEach((cb) => cb(m, reason));
  };

  /** Rejects only when there is no current pipeline (initial start); later failures are surfaced via notify. */
  async function switchTo(mode: DenoiseMode, reason: string) {
    const g = ++gen;
    const stale = () => g !== gen || closed;
    let next: Pipe;
    try {
      try {
        next = mode === 'deepfilter' ? await deep() : await plain(mode);
      } catch (e) {
        if (mode !== 'deepfilter') throw e;
        reason = `denoiser unavailable: ${msg(e)}`;
        next = await plain('browser');
      }
    } catch (e) {
      if (!pipe) throw e;
      if (stale()) return;
      pipe.stop();
      pipe = null;
      return lost(e);
    }
    if (stale()) return next.stop();
    const prev = stream.getAudioTracks()[0];
    if (prev) next.track.enabled = prev.enabled; // keep mute across the swap
    if (sender) {
      try {
        await sender.replaceTrack(next.track);
      } catch (e) {
        next.stop(); // the sender still uses the old track, so keep it running
        return notify(pipe?.mode ?? mode, `track swap failed: ${msg(e)}`);
      }
      if (stale()) return next.stop();
    }
    const old = pipe;
    pipe = next;
    if (prev) stream.removeTrack(prev);
    stream.addTrack(next.track);
    old?.stop();
    notify(next.mode, next.status ?? reason);
  }

  try {
    await switchTo(s.mode, '');
  } catch (e) {
    void ctx.close().catch(() => {});
    throw e;
  }

  return {
    stream,
    setSender(snd: RTCRtpSender) {
      sender = snd;
      const cur = stream.getAudioTracks()[0];
      if (snd.track !== cur) snd.replaceTrack(cur).catch((e) => notify(last?.[0] ?? s.mode, `track swap failed: ${msg(e)}`));
    },
    setSettings(n: AudioSettings) {
      const rebuild =
        n.mode !== s.mode || n.micId !== s.micId || n.noiseSuppression !== s.noiseSuppression || n.autoGain !== s.autoGain;
      s = n;
      if (rebuild) void switchTo(n.mode, '').catch(lost);
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
