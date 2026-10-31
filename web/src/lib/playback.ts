import { effectiveGain, isSpeaking } from './volume';

type Node = { gain?: GainNode; analyser: AnalyserNode; src: MediaStreamAudioSourceNode; el?: HTMLAudioElement };

/** One AudioContext; per stream: source -> gain -> destination, plus an analyser for the speaking ring. */
export class Playback {
  private ctx = new AudioContext();
  private nodes = new Map<string, Node>();
  private volumes = new Map<string, number>();
  private deafened = false;
  private buf = new Float32Array(1024);

  constructor(volumes: Map<string, number>) {
    this.volumes = volumes;
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
  }

  addRemote(id: string, stream: MediaStream) {
    this.remove(id);
    const gain = this.ctx.createGain();
    const n = { ...this.tap(stream), gain } as Node;
    n.src.connect(gain).connect(this.ctx.destination);
    // Chrome only pulls remote WebRTC audio when the stream is also attached to a media element.
    const el = new Audio();
    el.muted = true;
    el.srcObject = stream;
    void el.play().catch(() => {});
    n.el = el;
    this.nodes.set(id, n);
    this.apply(id);
    void this.ctx.resume();
  }

  remove(id: string) {
    const n = this.nodes.get(id);
    if (!n) return;
    n.src.disconnect();
    n.gain?.disconnect();
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
  setDeafened(d: boolean) {
    this.deafened = d;
    this.nodes.forEach((_, id) => this.apply(id));
  }
  speaking(): Set<string> {
    const out = new Set<string>();
    for (const [id, n] of this.nodes) {
      n.analyser.getFloatTimeDomainData(this.buf);
      if (isSpeaking(this.buf)) out.add(id);
    }
    return out;
  }
  close() {
    [...this.nodes.keys()].forEach((id) => this.remove(id));
    void this.ctx.close();
  }
}
