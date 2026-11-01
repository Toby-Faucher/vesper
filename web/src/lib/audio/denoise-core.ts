import type { DspOut } from './dsp-handler';
import { FrameRing, HOP } from './ring';
import { ModeController } from './mode';
import { Gate, Limiter } from './dynamics';

// The cushion. postMessage between the worklet and the worker jitters in a browser, so keep enough frames in
// each place to ride out a slow hop. This adds about 30 ms of delay over the old 2/3/2 setup (estimated, not measured).
const MAX_INFLIGHT = 4; // frames at the worker; beyond this they wait in the input ring
const IN_FRAMES = 4; // input ring size (40 ms); beyond this it drops the oldest samples
const MAX_OUT = 8; // denoised frames queued for output (80 ms); beyond this drop the oldest
// Trim a deep queue: above 5 frames for 2 s straight, drop one frame per second until 3 are left.
const TRIM_ABOVE = 5;
const TRIM_TO = 3;
const TRIM_AFTER_S = 2;
const PRIME = 3 * HOP; // start output with 30 ms queued, so one late reply does not cause a gap
// Fallback rule. A frame is late past 30 ms, the depth the output queue primes to.
const LATE_MS = 30;
// Judge the last second (100 frames of 10 ms) so one GC pause or CPU burst cannot trip it.
const WINDOW_FRAMES = 100;
// 20 late frames means at least 200 ms of slow audio in that second, not a blip.
const MAX_LATE = 20;
// Skip the first 3 s of replies: the WASM engine runs on its slow baseline tier right after load.
const WARMUP_FRAMES = 300;

const REPLY_WINDOW = 100; // replies kept for the average and max denoiser time

export type PipelineState = 'passthrough' | 'connecting' | 'priming' | 'playing' | 'failed';
/** Plain numbers, so the worklet can post them as they are. Levels are linear RMS, 0..1. */
export interface PipelineStats {
  state: PipelineState;
  failure: string | null;
  inputBlocks: number;
  emptyInputBlocks: number;
  framesSent: number;
  replies: number;
  underruns: number;
  reprimes: number;
  droppedInput: number;
  droppedOut: number;
  queuedMs: number;
  inflight: number;
  lastReplyMs: number;
  avgReplyMs: number;
  maxReplyMs: number;
  inputRms: number;
  outputRms: number;
}

/**
 * The denoise worklet's logic, free of AudioWorkletGlobalScope so it can be unit-tested.
 * Until connected, and after a failure, the raw input is passed through so the user stays audible.
 */
export class DenoiseCore {
  failure: string | null = null;
  inflight = 0;
  private send: ((f: Float32Array) => void) | null = null;
  private input = new FrameRing(IN_FRAMES);
  private out: Float32Array[] = [];
  private pos = 0;
  private count = 0;
  private playing = false;
  private idle = 0;
  private mode = new ModeController({
    budgetMs: LATE_MS,
    windowFrames: WINDOW_FRAMES,
    maxLate: MAX_LATE,
    warmupFrames: WARMUP_FRAMES,
  });
  private onFail: (reason: string) => void;
  private stallSamples: number;
  private rate: number;
  private n = { inputBlocks: 0, emptyInputBlocks: 0, framesSent: 0, replies: 0, underruns: 0, reprimes: 0, droppedOut: 0 };
  private droppedSamples = 0;
  private played = false;
  private deep = 0; // samples the queue has stayed above TRIM_ABOVE
  private trimming = false;
  private cool = 0; // samples until the next trim drop
  private replyMs = new Float64Array(REPLY_WINDOW);
  // Level meters: sums over the current half second; the last full half second is what stats() reports.
  private sq = { in: 0, out: 0, n: 0 };
  private rms = { in: 0, out: 0 };
  private limiter: Limiter;
  private limited = new Float32Array(128); // reallocated only if the quantum size changes
  /** The noise gate on the denoised output; capture.ts sets it from the settings. */
  gate: Gate;

  constructor(onFail: (reason: string) => void, sampleRate: number) {
    this.onFail = onFail;
    this.rate = sampleRate;
    this.stallSamples = sampleRate / 2; // a killed worker sends no error; half a second of silence means stalled
    this.limiter = new Limiter(sampleRate);
    this.gate = new Gate(sampleRate);
  }

  stats(): PipelineStats {
    const k = Math.min(this.n.replies, REPLY_WINDOW);
    let sum = 0;
    let max = 0;
    for (let i = 0; i < k; i++) {
      sum += this.replyMs[i];
      max = Math.max(max, this.replyMs[i]);
    }
    const state: PipelineState = this.failure ? 'failed' : !this.send ? 'passthrough' : this.playing ? 'playing'
      : this.n.replies ? 'priming' : 'connecting';
    return {
      state,
      failure: this.failure,
      ...this.n,
      droppedInput: Math.ceil(this.droppedSamples / HOP),
      queuedMs: (this.queued / this.rate) * 1000,
      inflight: this.inflight,
      lastReplyMs: this.n.replies ? this.replyMs[(this.n.replies - 1) % REPLY_WINDOW] : 0,
      avgReplyMs: k ? sum / k : 0,
      maxReplyMs: max,
      inputRms: this.rms.in,
      outputRms: this.rms.out,
    };
  }

  private meter(input: Float32Array | undefined, o: Float32Array) {
    for (let i = 0; i < o.length; i++) {
      if (input) this.sq.in += input[i] * input[i];
      this.sq.out += o[i] * o[i];
    }
    if ((this.sq.n += o.length) < this.rate / 2) return;
    this.rms.in = Math.sqrt(this.sq.in / this.sq.n);
    this.rms.out = Math.sqrt(this.sq.out / this.sq.n);
    this.sq.in = this.sq.out = this.sq.n = 0;
  }

  connect(send: (f: Float32Array) => void) {
    this.send = send;
  }

  get queued() {
    return this.count - this.pos;
  }

  fail(reason: string) {
    if (this.failure) return;
    this.failure = reason;
    this.onFail(reason);
  }

  reply(m: DspOut) {
    if (m.type !== 'frame') return this.fail(m.type === 'error' ? `denoiser error: ${m.message}` : 'denoiser protocol');
    this.inflight = Math.max(0, this.inflight - 1);
    this.idle = 0;
    this.replyMs[this.n.replies++ % REPLY_WINDOW] = m.ms;
    this.gate.detect(m.data); // on arrival, a queue depth ahead of playout: the gate's lookahead
    this.out.push(m.data);
    this.count += m.data.length;
    if (this.out.length > MAX_OUT) {
      this.count -= this.out.shift()!.length;
      this.pos = 0;
      this.n.droppedOut++;
    }
    if (this.mode.report(m.ms) === 'browser') this.fail('denoiser too slow for real time');
  }

  /** The limiter runs first, so the denoiser, the passthrough and the input meter all see the limited boost. */
  render(raw: Float32Array | undefined, o: Float32Array): void {
    let input: Float32Array | undefined;
    if (raw) {
      this.n.inputBlocks++;
      if (this.limited.length !== raw.length) this.limited = new Float32Array(raw.length);
      this.limiter.process(raw, (input = this.limited));
    } else this.n.emptyInputBlocks++;
    this.process(input, o);
    this.trim(o.length);
    this.meter(input, o);
  }

  private trim(n: number) {
    if (!this.trimming) {
      this.deep = this.out.length > TRIM_ABOVE ? this.deep + n : 0;
      if (this.deep < TRIM_AFTER_S * this.rate) return;
      this.trimming = true;
      this.cool = 0;
    }
    if (this.out.length <= TRIM_TO) return void (this.trimming = false, this.deep = 0);
    if ((this.cool -= n) > 0) return;
    this.cool = this.rate;
    this.count -= this.out.splice(1, 1)[0].length; // not the head: it may be half played
    this.n.droppedOut++;
  }

  private process(input: Float32Array | undefined, o: Float32Array): void {
    if (!this.send || this.failure) {
      for (let i = 0; i < o.length; i++) o[i] = input ? input[i] : 0;
      return;
    }
    if (input) this.droppedSamples += this.input.push(input);
    for (let f; this.inflight < MAX_INFLIGHT && (f = this.input.pop()); this.inflight++, this.n.framesSent++) this.send(f);
    if (this.inflight > 0 && (this.idle += o.length) > this.stallSamples) {
      this.fail('denoiser stalled');
      return this.process(input, o);
    }
    if (!this.playing && this.queued >= PRIME) {
      this.playing = true;
      if (this.played) this.n.reprimes++;
      this.played = true;
    }
    const wasPlaying = this.playing;
    let i = 0;
    while (this.playing && i < o.length && this.out.length) {
      const f = this.out[0];
      while (i < o.length && this.pos < f.length) o[i++] = f[this.pos++];
      if (this.pos === f.length) {
        this.out.shift();
        this.count -= f.length;
        this.pos = 0;
      }
    }
    if (i < o.length) {
      if (wasPlaying) this.n.underruns++;
      this.playing = false; // underrun: re-prime
    }
    for (; i < o.length; i++) o[i] = 0;
    this.gate.apply(o); // denoised output only: the passthrough above stays ungated, so the user stays audible
  }
}
