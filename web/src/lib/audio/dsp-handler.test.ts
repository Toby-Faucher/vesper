import { describe, expect, it } from 'vitest';
import { createDspHandler, type DspWasm } from './dsp-handler';

const HOP = 480;
const wasm: DspWasm = {
  WasmDenoiser: class {
    process(input: Float32Array) {
      if (input.length !== HOP) throw new Error(`frame must be ${HOP} samples, got ${input.length}`);
      return input.map((x) => x * 0.5);
    }
    setAttenLimit() {}
    setPostFilter() {}
  },
};
const make = (fetchModel = async () => new Uint8Array([1])) =>
  createDspHandler({ loadWasm: async () => wasm, fetchModel });
const init = { type: 'init', modelUrl: 'm', atten: 100, beta: 0 } as const;

describe('dsp handler', () => {
  it('init then 3 frames', async () => {
    const h = make();
    expect(await h(init)).toEqual([{ type: 'ready' }]);
    for (let i = 0; i < 3; i++) {
      const [r] = await h({ type: 'frame', data: new Float32Array(HOP).fill(1) });
      expect(r.type).toBe('frame');
      if (r.type !== 'frame') throw new Error();
      expect(r.data).toHaveLength(HOP);
      expect(r.data[0]).toBe(0.5);
      expect(r.ms).toBeGreaterThanOrEqual(0);
    }
  });

  it('wrong-length frame errors and keeps serving', async () => {
    const h = make();
    await h(init);
    const [e] = await h({ type: 'frame', data: new Float32Array(100) });
    expect(e.type).toBe('error');
    const [ok] = await h({ type: 'frame', data: new Float32Array(HOP) });
    expect(ok.type).toBe('frame');
  });

  it('failed model fetch errors and never readies', async () => {
    const h = make(async () => {
      throw new Error('404');
    });
    const out = await h(init);
    expect(out.map((r) => r.type)).toEqual(['error']);
    expect((await h({ type: 'frame', data: new Float32Array(HOP) }))[0].type).toBe('error');
  });

  it('set forwards atten and beta', async () => {
    const calls: string[] = [];
    const h = createDspHandler({
      loadWasm: async () => ({
        WasmDenoiser: class {
          process = (i: Float32Array) => i;
          setAttenLimit = (v: number) => calls.push(`a${v}`);
          setPostFilter = (v: number) => calls.push(`b${v}`);
        },
      }),
      fetchModel: async () => new Uint8Array(),
    });
    await h(init);
    expect(await h({ type: 'set', atten: 20 })).toEqual([]);
    await h({ type: 'set', beta: 0.02 });
    expect(calls).toEqual(['a20', 'b0.02']);
  });
});
