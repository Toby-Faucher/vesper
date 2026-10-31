export type DspIn =
  | { type: 'init'; modelUrl: string; atten: number; beta: number }
  | { type: 'frame'; data: Float32Array }
  | { type: 'set'; atten?: number; beta?: number };

export type DspOut =
  | { type: 'ready' }
  | { type: 'frame'; data: Float32Array; ms: number }
  | { type: 'error'; message: string };

interface Denoiser {
  process(input: Float32Array): Float32Array;
  setAttenLimit(db: number): void;
  setPostFilter(beta: number): void;
}
export interface DspWasm {
  WasmDenoiser: new (model: Uint8Array, atten: number, beta: number) => Denoiser;
}

export interface DspDeps {
  loadWasm: () => Promise<DspWasm>;
  fetchModel: (url: string) => Promise<Uint8Array>;
}

const err = (e: unknown): DspOut[] => [{ type: 'error', message: e instanceof Error ? e.message : String(e) }];

/** Pure message handler; the worker file only wires it to postMessage. */
export function createDspHandler({ loadWasm, fetchModel }: DspDeps) {
  let d: Denoiser | null = null;
  return async (msg: DspIn): Promise<DspOut[]> => {
    try {
      if (msg.type === 'init') {
        d = null;
        const [wasm, model] = await Promise.all([loadWasm(), fetchModel(msg.modelUrl)]);
        d = new wasm.WasmDenoiser(model, msg.atten, msg.beta);
        return [{ type: 'ready' }];
      }
      if (!d) throw new Error('denoiser not initialised');
      if (msg.type === 'set') {
        if (msg.atten !== undefined) d.setAttenLimit(msg.atten);
        if (msg.beta !== undefined) d.setPostFilter(msg.beta);
        return [];
      }
      const t = performance.now();
      const data = d.process(msg.data);
      return [{ type: 'frame', data, ms: performance.now() - t }];
    } catch (e) {
      return err(e);
    }
  };
}
