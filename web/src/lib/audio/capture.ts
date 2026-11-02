import type { DspOut } from './dsp-handler';
import type { PipelineStats } from './denoise-core';
import type { DenoiseMode } from './mode';
import type { AudioSettings } from './settings';
import { deviceGone } from '../devices';
import { captureContext } from './context';
import { SILENT_RMS } from './mic-silence';
import workletUrl from './denoise.worklet.ts?worker&url';

export interface CaptureDeps {
  getUserMedia(c: MediaStreamConstraints): Promise<MediaStream>;
  /** Called synchronously by createCapture, so call createCapture inside the user gesture. */
  createContext(): AudioContext;
  createWorkletNode(ctx: AudioContext): Promise<AudioWorkletNode>;
  createWorker(): Worker;
  readyTimeoutMs: number;
  /** Initial track state: false when PTT/mute must start silent. */
  startLive: boolean;
  now(): number;
  /** Called when the saved mic is gone and the default mic is used instead; the capture then forgets the saved id. */
  onMicFallback?(): void;
}

/** The mic passes through (with the browser's filter) while the model loads, and a slow first download can take longer than 5 s. */
export const READY_TIMEOUT_MS = 20_000;
/** The status while DeepFilterNet loads: the browser filter is on meanwhile, and goes off when the model takes over. */
export const LOADING = 'loading, browser filter until then';
/** The mic reopened without the browser filter (Chromium) must answer within this, or the call falls back to browser mode. */
export const REOPEN_TIMEOUT_MS = 3000;
/** Worklet reports (500 ms each) of digital silence from the reopened mic before the filtered track comes back. */
export const SILENT_REPORTS = 4;
export const SWAP_SKIPPED = 'swap skipped: reopened mic was silent';
// Kept for the page session, so the swap is not tried again (and cannot ping-pong) once a reopened mic was silent.
let swapSkipped = false;
/** For tests. */
export const forgetSwapSkip = () => void (swapSkipped = false);

const modules = new WeakMap<BaseAudioContext, Promise<void>>();
const browserDeps: CaptureDeps = {
  getUserMedia: (c) => navigator.mediaDevices.getUserMedia(c),
  createContext: captureContext,
  createWorkletNode: async (ctx) => {
    if (!modules.has(ctx)) modules.set(ctx, ctx.audioWorklet.addModule(workletUrl));
    await modules.get(ctx);
    return new AudioWorkletNode(ctx, 'vesper-denoise', {
      numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1], channelCount: 1, channelCountMode: 'explicit',
    });
  },
  createWorker: () => new Worker(new URL('./dsp.worker.ts', import.meta.url), { type: 'module' }),
  readyTimeoutMs: READY_TIMEOUT_MS,
  startLive: true,
  now: () => performance.now(),
};

type Pipe = {
  mode: DenoiseMode; track: MediaStreamTrack; raw: MediaStreamTrack; stop(): void; set?(s: AudioSettings): void; status?: string; reopens?: number;
  stats?: PipelineStats;
  statsAt?: number;
};
/** pipeline is null outside deepfilter mode or before the worklet's first report (every 500 ms). */
export interface CaptureStats {
  pipeline: PipelineStats | null;
  /** ms since that snapshot arrived; a growing age means the worklet stopped running. null with no pipeline. */
  ageMs: number | null;
  /** readyState and muted of the raw mic track; enabled is the sent track's mute gate. */
  mic: { readyState: MediaStreamTrackState; muted: boolean; enabled: boolean; settings: MicSettings; reopens: number };
  context: { state: AudioContextState; sampleRate: number };
}
/** The raw track's getSettings() without deviceId (it identifies the hardware). */
export type MicSettings = Pick<MediaTrackSettings, 'sampleRate' | 'channelCount' | 'echoCancellation' | 'noiseSuppression' | 'autoGainControl'> & { latency?: number };
const MUTE_GRACE_MS = 3000;
const stopAll = (m: MediaStream) => m.getTracks().forEach((t) => t.stop());

/**
 * deepfilter: mic -> gain -> AudioWorklet <-MessageChannel-> DSP worker, gain -> worklet, worklet -> MediaStreamDestination.
 * Inside the worklet (dynamics.ts): a soft limiter on the boosted input (only with a boost) before the denoiser, and the optional noise gate
 * (settings.vad.gate, at the level marker) on the denoised output. Neither exists in browser or off mode.
 * The boost sits before the denoiser, so the model and the worklet's input meter (and the mic-silent banner) see the boosted signal.
 * Digital silence stays silent at any gain, so the banner means what it did.
 * browser/off: the raw mic track, so the fallback does not depend on the AudioContext or worklet.
 * `stream` is stable; its one track is swapped (and replaceTrack'd on the sender) when the mode changes.
 */
export async function createCapture(s: AudioSettings, deps: Partial<CaptureDeps> = {}) {
  const d = { ...browserDeps, ...deps };
  const ctx = d.createContext();
  void ctx.resume().catch(() => {}); // Firefox rejects it if close() comes first
  // The browser can suspend the context (device change, OS audio session): the graph would stop, so ask it to run again.
  ctx.onstatechange = () => {
    if (!closed && ctx.state !== 'running' && ctx.state !== 'closed') void ctx.resume().catch(() => {});
  };
  const stream = new MediaStream();
  const cbs: ((m: DenoiseMode, reason: string) => void)[] = [];
  const statCbs = new Set<(s: CaptureStats) => void>();
  let last: [DenoiseMode, string] | null = null; // replayed to late subscribers
  let pipe: Pipe | null = null;
  let sender: RTCRtpSender | null = null;
  let gen = 0;
  let closed = false;
  let live = d.startLive; // applied to every new track, so mute/PTT survive swaps and rebuilds

  // deepfilter asks with the browser filter on (while the model loads); `deep` asks again without it once the model is ready.
  const ask = (mode: DenoiseMode, id: string | null, ns = mode !== 'off') =>
    d.getUserMedia({
      audio: {
        echoCancellation: true,
        noiseSuppression: ns,
        autoGainControl: s.autoGain,
        channelCount: 1,
        sampleRate: 48000,
        ...(id ? { deviceId: { exact: id } } : {}),
      },
    });
  // A saved id can be gone (unplugged, another profile, Firefox new ids after a permission change): try the default mic
  // once. Any other error, a denial above all, is not retried.
  async function mic(mode: DenoiseMode) {
    const id = s.micId;
    if (!id) return ask(mode, null);
    try {
      return await ask(mode, id);
    } catch (e) {
      if (!deviceGone(e)) throw e;
      const m = await ask(mode, null);
      // Not when another mic was picked while this ran: that choice stands (and its own build follows).
      if (s.micId === id) {
        s = { ...s, micId: null };
        d.onMicFallback?.();
      }
      return m;
    }
  }

  /**
   * The raw mic can end under us (permission revoked, default device unplugged, OS reclaim): drop the pipe and
   * report it. A mute that outlasts MUTE_GRACE_MS is reported too but the pipe is kept, since it may come back.
   */
  function watch(raw: MediaStreamTrack, me: () => Pipe | undefined) {
    let t: ReturnType<typeof setTimeout> | undefined;
    let before: [DenoiseMode, string] | null = null;
    const current = () => !closed && !!me() && pipe === me();
    raw.onended = () => {
      if (!current()) return;
      pipe!.stop();
      pipe = null;
      notify('off', 'microphone lost: ended');
    };
    raw.onmute = () => {
      clearTimeout(t);
      t = setTimeout(() => {
        if (!raw.muted || !current()) return;
        before = last;
        notify(pipe!.mode, 'microphone lost: muted by the system');
      }, MUTE_GRACE_MS);
    };
    raw.onunmute = () => {
      clearTimeout(t);
      if (before && current()) notify(...before);
      before = null;
    };
    return () => {
      clearTimeout(t);
      raw.onended = raw.onmute = raw.onunmute = null;
    };
  }

  async function plain(mode: DenoiseMode): Promise<Pipe> {
    const m = await mic(mode);
    const unwatch = watch(m.getAudioTracks()[0], () => me);
    const t = m.getAudioTracks()[0];
    const me: Pipe = { mode, track: t, raw: t, stop: () => (unwatch(), stopAll(m)) };
    return me;
  }

  async function deep(): Promise<Pipe> {
    if (ctx.sampleRate !== 48000) throw new Error(`AudioContext runs at ${ctx.sampleRate} Hz, DeepFilterNet needs 48000`);
    let m = await mic('deepfilter');
    let me: Pipe | undefined;
    let unwatch = watch(m.getAudioTracks()[0], () => me);
    let dead = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let node: AudioWorkletNode | undefined;
    let worker: Worker | undefined;
    let ready = false;
    let silent = -1; // silent reports since the reopen; -1 when not watching
    const nodes: AudioNode[] = [];
    let out: MediaStreamAudioDestinationNode | undefined;
    const stop = () => {
      dead = true;
      clearTimeout(timer);
      unwatch();
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
      let src = ctx.createMediaStreamSource(m);
      const gain = ctx.createGain();
      gain.gain.value = 10 ** (s.inputBoostDb / 20);
      out = ctx.createMediaStreamDestination();
      nodes.push(src, gain, node);
      src.connect(gain).connect(node).connect(out);
      const w = (worker = d.createWorker());
      const n = node;
      const set = () => w.postMessage({ type: 'set', atten: s.attenDb, beta: s.postFilter });
      // The limiter only with a boost: at 0 dB the default path stays bit-identical to the mic.
      const dyn = () => n.port.postMessage({ type: 'dynamics', limit: s.inputBoostDb > 0, gate: s.vad.gate, thresholdDb: s.vad.thresholdDb });
      dyn(); // before the model is ready too: the passthrough is limited and the boost already applies
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
        me!.status = !s.noiseSuppression && swapSkipped ? SWAP_SKIPPED : '';
        // DeepFilterNet takes over now: the browser filter goes off, so the two never stack. A failure is named, not hidden.
        if (!s.noiseSuppression && !swapSkipped) void unfiltered().catch((e) => fail(`the mic did not reopen without the browser filter: ${msg(e)}`));
        if (pipe === me) notify('deepfilter', me!.status);
      };
      // applyConstraints turns the filter off where the browser can (Firefox). Chromium resolves it but keeps the filter
      // (getSettings() stays true), and a second track from the same mic opened while the first runs shares it. There only,
      // the first track ends and the graph's source moves to a new track asked for without the filter: a gap of one
      // getUserMedia call as the model starts. The sent track stays. A request that does not answer (a hidden tab can hold
      // it) would leave the call silent, so it gets REOPEN_TIMEOUT_MS.
      const unfiltered = async () => {
        const t = m.getAudioTracks()[0];
        await t.applyConstraints({ ...t.getConstraints(), noiseSuppression: false }).catch(() => {});
        if (dead || t.getSettings().noiseSuppression !== true) return;
        const heard = (me!.stats?.inputRms ?? 1) >= SILENT_RMS; // no report yet counts as sound
        await reopen(false);
        if (heard && !dead) silent = 0;
      };
      // Edge was seen to hand back a live, unmuted reopened track that delivers only zeros: then the filtered track comes back.
      const silentCheck = (rms: number) => {
        if (silent < 0) return;
        if (rms >= SILENT_RMS) return void (silent = -1);
        if (++silent < SILENT_REPORTS) return;
        silent = -1;
        swapSkipped = true;
        void reopen(true).then(() => {
          me!.status = SWAP_SKIPPED;
          if (pipe === me) notify('deepfilter', SWAP_SKIPPED);
        }, (e) => fail(`the mic did not reopen with the browser filter: ${msg(e)}`));
      };
      const reopen = async (ns: boolean) => {
        const id = m.getAudioTracks()[0].getSettings().deviceId ?? null;
        unwatch();
        stopAll(m);
        const asked = ask('deepfilter', id, ns);
        let timer: ReturnType<typeof setTimeout> | undefined;
        const late = new Promise<never>((_, no) => (timer = setTimeout(() => no(new Error(`no answer in ${REOPEN_TIMEOUT_MS / 1000} s`)), REOPEN_TIMEOUT_MS)));
        const next = await Promise.race([asked, late]).catch((e) => {
          void asked.then(stopAll, () => {}); // a late answer is not used
          throw e;
        }).finally(() => clearTimeout(timer));
        if (dead) return stopAll(next);
        const s2 = ctx.createMediaStreamSource(next);
        s2.connect(gain);
        src.disconnect();
        nodes[0] = src = s2;
        m = next;
        unwatch = watch(next.getAudioTracks()[0], () => me);
        me!.raw = next.getAudioTracks()[0];
        me!.reopens = (me!.reopens ?? 0) + 1;
      };
      w.onerror = (e) => fail(`denoiser worker failed: ${e.message ?? ''}`);
      n.port.onmessage = ({ data }: MessageEvent<{ type: 'error'; message: string } | ({ type: 'stats' } & PipelineStats)>) => {
        if (data.type !== 'stats') return fail(data.message);
        const { type: _, ...stats } = data;
        me!.stats = stats;
        silentCheck(stats.inputRms);
        me!.statsAt = d.now();
        const snap = pipe === me ? getStats() : null;
        if (snap) statCbs.forEach((cb) => cb(snap));
      };
      w.postMessage({ type: 'init', modelUrl: '/dsp/model.tar.gz', atten: s.attenDb, beta: s.postFilter });
      me = {
        mode: 'deepfilter',
        status: s.noiseSuppression ? 'loading' : LOADING, // with the filter on top it stays on after loading too
        track: out.stream.getAudioTracks()[0],
        raw: m.getAudioTracks()[0],
        stop,
        set: (n) => {
          gain.gain.value = 10 ** (n.inputBoostDb / 20);
          dyn();
          if (ready) set();
        },
      };
      return me;
    } catch (e) {
      stop();
      throw e;
    }
  }

  function getStats(): CaptureStats | null {
    if (!pipe || closed) return null;
    const { raw, track } = pipe;
    return {
      pipeline: pipe.stats ?? null,
      ageMs: pipe.statsAt === undefined ? null : d.now() - pipe.statsAt,
      mic: {
        readyState: raw.readyState, muted: raw.muted, enabled: track.enabled, reopens: pipe.reopens ?? 0,
        settings: (({ sampleRate, channelCount, echoCancellation, noiseSuppression, autoGainControl, latency }) => ({
          sampleRate, channelCount, echoCancellation, noiseSuppression, autoGainControl, latency,
        }))(raw.getSettings() as MediaTrackSettings & { latency?: number }),
      },
      context: { state: ctx.state, sampleRate: ctx.sampleRate },
    };
  }

  const msg = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const lost = (e: unknown) => notify('off', `microphone lost: ${(e as Error)?.name ?? 'Error'}: ${msg(e)}`);
  // The boost gain node exists only in the deepfilter graph.
  const note = (m: DenoiseMode, r: string) =>
    m !== 'deepfilter' && s.inputBoostDb > 0 ? (r ? `${r}; ` : '') + 'input boost applies only in deepfilter mode' : r;
  const notify = (m: DenoiseMode, reason: string) => {
    last = [m, reason];
    cbs.forEach((cb) => cb(m, note(m, reason)));
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
    next.track.enabled = live;
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
    /** Mute / push-to-talk gate: applies to the current track and every future one. */
    setLive(v: boolean) {
      live = v;
      stream.getAudioTracks().forEach((t) => (t.enabled = v));
    },
    setSettings(n: AudioSettings) {
      const rebuild =
        !pipe || n.mode !== s.mode || n.micId !== s.micId || n.noiseSuppression !== s.noiseSuppression || n.autoGain !== s.autoGain;
      const boostChanged = n.inputBoostDb !== s.inputBoostDb;
      s = n;
      if (rebuild) void switchTo(n.mode, '').catch(lost);
      else pipe?.set?.(n);
      if (!rebuild && boostChanged && last) notify(...last);
    },
    /**
     * After the mic ended (or the system muted it): get the mic again and build the graph anew, in the same call (the
     * sender's track is replaced). A saved mic that is gone falls back to the default one, as at the start. A failure is
     * reported through onModeChange as `microphone lost: ...`. Resolves true when a mic runs again. skipSwap keeps the
     * browser filter on after the model loads, for the rest of the session.
     */
    async retry(skipSwap = false): Promise<boolean> {
      if (skipSwap) swapSkipped = true; // "no sound" in a call: the swap is the one known suspect
      await switchTo(s.mode, '').catch(lost);
      return !!pipe;
    },
    /** The latest snapshot, or null when there is no pipeline. */
    getStats,
    /** Called with each new snapshot from the worklet (deepfilter mode only). Returns an unsubscribe function. */
    onStats(cb: (s: CaptureStats) => void) {
      statCbs.add(cb);
      return () => void statCbs.delete(cb);
    },
    onModeChange(cb: (m: DenoiseMode, reason: string) => void) {
      cbs.push(cb);
      if (last) cb(last[0], note(...last));
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
