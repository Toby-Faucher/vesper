// Runs on the audio rendering thread; a thin wrapper, the logic is DenoiseCore (unit-tested).
import type { DspOut } from './dsp-handler';
import { DenoiseCore } from './denoise-core';

declare const sampleRate: number;
declare function registerProcessor(name: string, ctor: unknown): void;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}

class Denoise extends AudioWorkletProcessor {
  private dsp: MessagePort | null = null;
  private stopped = false;
  private core = new DenoiseCore((message) => this.port.postMessage({ type: 'error', message }), sampleRate);

  constructor() {
    super();
    this.port.onmessage = ({ data, ports }) => {
      if (data.type === 'stop') {
        this.stopped = true;
        this.dsp?.close();
      } else if (data.type === 'port') {
        const p = (this.dsp = ports[0]);
        p.onmessage = ({ data }: MessageEvent<DspOut>) => this.core.reply(data);
        this.core.connect((f) => p.postMessage({ type: 'frame', data: f }, [f.buffer]));
      }
    };
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]) {
    if (this.stopped) return false;
    this.core.render(inputs[0][0], outputs[0][0]);
    return true;
  }
}

registerProcessor('vesper-denoise', Denoise);
