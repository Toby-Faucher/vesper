import { effectiveGain, isSpeaking } from './volume';
import type { LevelState } from './level';

type Node = { gain?: GainNode; level?: GainNode; ls?: LevelState; analyser: AnalyserNode; src: MediaStreamAudioSourceNode; el?: HTMLAudioElement };

/**
 * One AudioContext; per stream: source -> level -> gain -> destination, plus an analyser for the speaking ring. `level` is the
 * "Even out voices" gain (1 when off), under the manual volume in `gain`.
 */
export class Playback {
  private ctx: AudioContext;
  private nodes = new Map<string, Node>();
  private volumes = new Map<string, number>();
  private deafened = false;
  private buf = new Float32Array(1024);
  private levelling = false;
  private lv?: typeof import('./level'); // the levelling math, loaded when it is first turned on
  private lastTick = 0;

  constructor(volumes: Map<string, number>, ctx: AudioContext = new AudioContext()) {
    this.volumes = volumes;
    this.ctx = ctx;
    // Firefox rejects a pending resume when the context closes first (a join that fails at once): nothing to do then.
    void ctx.resume().catch(() => {});
  }

  private tap(stream: MediaStream) {
    const src = this.ctx.createMediaStreamSource(stream);
    const analyser = this.ctx.createAnalyser();
    analyser.fftSize = 1024;
    src.connect(analyser);
    return { src, analyser };
  }

  /** Local mic: analysed only, never played back. */
  addLocal(id: string, stream: MediaStream) {
    this.nodes.set(id, this.tap(stream));
    void this.ctx.resume().catch(() => {});
  }

  addRemote(id: string, stream: MediaStream) {
    this.remove(id);
    const gain = this.ctx.createGain();
    const level = this.ctx.createGain();
    const n = { ...this.tap(stream), gain, level } as Node;
    n.src.connect(level).connect(gain).connect(this.ctx.destination);
    // Chrome only pulls remote WebRTC audio when the stream is also attached to a media element.
    const el = new Audio();
    el.muted = true;
    el.srcObject = stream;
    void el.play().catch(() => {});
    n.el = el;
    this.nodes.set(id, n);
    this.apply(id);
    void this.ctx.resume().catch(() => {});
  }

  remove(id: string) {
    const n = this.nodes.get(id);
    if (!n) return;
    n.src.disconnect();
    n.gain?.disconnect();
    n.level?.disconnect();
    if (n.el) {
      n.el.pause();
      n.el.srcObject = null;
    }
    this.nodes.delete(id);
  }

  private apply(id: string) {
    const g = this.nodes.get(id)?.gain;
    if (g) g.gain.value = effectiveGain(this.volumes.get(id) ?? 100, this.deafened);
  }

  setVolume(id: string, v: number) {
    this.volumes.set(id, v);
    this.apply(id);
  }
  /** AudioContext.setSinkId is Chrome only; elsewhere the OS default output is used. */
  async setSink(id: string | null): Promise<boolean> {
    const c = this.ctx as AudioContext & { setSinkId?: (id: string) => Promise<void> };
    if (!c.setSinkId) return false;
    await c.setSinkId(id ?? '');
    return true;
  }
  setDeafened(d: boolean) {
    this.deafened = d;
    this.nodes.forEach((_, id) => this.apply(id));
  }
  /** Off resets every person's levelling, so turning it on again starts from 0 dB. */
  setLevelling(on: boolean) {
    this.levelling = on;
    if (on && !this.lv) void import('./level').then((m) => (this.lv = m), () => {});
    for (const n of this.nodes.values()) {
      if (!n.level) continue;
      n.ls = undefined;
      n.level.gain.cancelScheduledValues(this.ctx.currentTime);
      n.level.gain.setTargetAtTime(1, this.ctx.currentTime, 0.05); // a ramp: a jump of up to 10 dB mid-call would click
    }
  }
  /** Who is speaking now. Called every 100 ms; with levelling on it also steps each person's level from the same frame. */
  speaking(): Set<string> {
    const out = new Set<string>();
    const t = this.ctx.currentTime;
    const dt = Math.min(0.2, t - this.lastTick); // a throttled background tab adapts slower, not in jumps
    this.lastTick = t;
    for (const [id, n] of this.nodes) {
      n.analyser.getFloatTimeDomainData(this.buf);
      if (isSpeaking(this.buf)) out.add(id);
      const m = this.lv;
      if (!this.levelling || !n.level || !m) continue;
      const ls = n.ls ?? m.LEVEL_START;
      const next = m.stepLevel(ls, m.rmsDb(this.buf), dt);
      if (next.gainDb !== ls.gainDb) n.level.gain.setTargetAtTime(m.dbToGain(next.gainDb), t, 0.05); // 50 ms: no zipper noise
      n.ls = next;
    }
    return out;
  }
  /** The levelling gain in dB per person, for tests. */
  levelDb(id: string): number | undefined {
    const n = this.nodes.get(id);
    return n?.level ? (n.ls?.gainDb ?? 0) : undefined;
  }
  close() {
    [...this.nodes.keys()].forEach((id) => this.remove(id));
    void this.ctx.close().catch(() => {});
  }
}
