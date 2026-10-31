import type { DspOut } from './dsp-handler';
import { FrameRing, HOP } from './ring';
import { ModeController } from './mode';

const MAX_INFLIGHT = 2; // frames at the worker; beyond this the input ring drops the oldest
const MAX_OUT = 3; // denoised frames queued for output (30 ms); beyond this drop the oldest
// Fallback rule. A frame is late past 30 ms, where the 3-frame output queue would run dry.
const LATE_MS = 30;
// Judge the last second (100 frames of 10 ms) so one GC pause or CPU burst cannot trip it.
const WINDOW_FRAMES = 100;
// 20 late frames means at least 200 ms of slow audio in that second, not a blip.
const MAX_LATE = 20;
// Skip the first 3 s of replies: the WASM engine runs on its slow baseline tier right after load.
const WARMUP_FRAMES = 300;
const PRIME = 2 * HOP; // start output with one spare frame to absorb worker spikes

/**
 * The denoise worklet's logic, free of AudioWorkletGlobalScope so it can be unit-tested.
 * Until connected, and after a failure, the raw input is passed through so the user stays audible.
 */
export class DenoiseCore {
  failure: string | null = null;
  inflight = 0;
  private send: ((f: Float32Array) => void) | null = null;
  private input = new FrameRing(2);
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

  constructor(onFail: (reason: string) => void, sampleRate: number) {
    this.onFail = onFail;
    this.stallSamples = sampleRate / 2; // a killed worker sends no error; half a second of silence means stalled
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
    this.out.push(m.data);
    this.count += m.data.length;
    if (this.out.length > MAX_OUT) {
      this.count -= this.out.shift()!.length;
      this.pos = 0;
    }
    if (this.mode.report(m.ms) === 'browser') this.fail('denoiser too slow for real time');
  }

  render(input: Float32Array | undefined, o: Float32Array): void {
    if (!this.send || this.failure) {
      for (let i = 0; i < o.length; i++) o[i] = input ? input[i] : 0;
      return;
    }
    if (input) this.input.push(input);
    for (let f; this.inflight < MAX_INFLIGHT && (f = this.input.pop()); this.inflight++) this.send(f);
    if (this.inflight > 0 && (this.idle += o.length) > this.stallSamples) {
      this.fail('denoiser stalled');
      return this.render(input, o);
    }
    if (!this.playing) this.playing = this.queued >= PRIME;
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
    if (i < o.length) this.playing = false; // underrun: re-prime
    for (; i < o.length; i++) o[i] = 0;
  }
}
