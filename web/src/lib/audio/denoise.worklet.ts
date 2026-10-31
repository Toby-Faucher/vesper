// Runs on the audio rendering thread. Not unit-testable here (no AudioWorkletGlobalScope in Node):
// the queueing logic lives in FrameRing and ModeController, which are tested.
import type { DspOut } from './dsp-handler';
import { FrameRing, HOP } from './ring';
import { ModeController } from './mode';

declare const sampleRate: number;
declare function registerProcessor(name: string, ctor: unknown): void;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}

const MAX_INFLIGHT = 2; // frames at the worker; beyond this the input ring drops the oldest
const MAX_OUT = 3; // denoised frames queued for output (30 ms); beyond this drop the oldest
const PRIME = 2 * HOP; // start output with one spare frame to absorb worker spikes

class Denoise extends AudioWorkletProcessor {
  private dsp: MessagePort | null = null;
  private input = new FrameRing(2);
  private out: Float32Array[] = [];
  private pos = 0;
  private playing = false;
  private inflight = 0;
  private idle = 0;
  private failed = false;
  private mode = new ModeController({ budgetMs: 10, windowFrames: 100, maxLate: 10 });

  constructor() {
    super();
    this.port.onmessage = ({ data, ports }) => {
      if (data.type !== 'port') return;
      this.dsp = ports[0];
      this.dsp.onmessage = ({ data }: MessageEvent<DspOut>) => this.reply(data);
    };
  }

  private fail(message: string) {
    if (this.failed) return;
    this.failed = true;
    this.port.postMessage({ type: 'error', message });
  }

  private reply(m: DspOut) {
    if (m.type !== 'frame') return this.fail(m.type === 'error' ? `denoiser error: ${m.message}` : 'denoiser protocol');
    this.inflight--;
    this.idle = 0;
    this.out.push(m.data);
    if (this.out.length > MAX_OUT) {
      this.out.shift();
      this.pos = 0;
    }
    if (this.mode.report(m.ms) === 'browser') this.fail('denoiser too slow for real time');
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]) {
    const o = outputs[0][0];
    if (!this.dsp || this.failed) return true; // silence until ready; the main thread swaps tracks on failure
    if (inputs[0][0]) this.input.push(inputs[0][0]);
    for (let f; this.inflight < MAX_INFLIGHT && (f = this.input.pop()); this.inflight++) {
      this.dsp.postMessage({ type: 'frame', data: f }, [f.buffer]);
    }
    // A killed worker sends no error; half a second without replies counts as a failure.
    if (this.inflight > 0 && (this.idle += o.length) > sampleRate / 2) this.fail('denoiser stalled');
    const queued = this.out.reduce((n, f) => n + f.length, 0) - this.pos;
    if (!this.playing) this.playing = queued >= PRIME;
    let i = 0;
    while (this.playing && i < o.length && this.out.length) {
      const f = this.out[0];
      const n = Math.min(o.length - i, f.length - this.pos);
      o.set(f.subarray(this.pos, this.pos + n), i);
      i += n;
      this.pos += n;
      if (this.pos === f.length) {
        this.out.shift();
        this.pos = 0;
      }
    }
    if (i < o.length) this.playing = false; // underrun: re-prime
    return true;
  }
}

registerProcessor('vesper-denoise', Denoise);
