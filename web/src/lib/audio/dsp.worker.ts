import { createDspHandler, type DspIn, type DspWasm } from './dsp-handler';

// Built by scripts/build-dsp.sh into web/public/dsp/ (gitignored), so resolved at runtime, not bundled.
const base = new URL('/dsp/', self.location.href).href;

const handle = createDspHandler({
  loadWasm: async () => {
    const m = await import(/* @vite-ignore */ `${base}vesper_dsp.js`);
    await m.default({ module_or_path: `${base}vesper_dsp_bg.wasm` });
    return m as DspWasm;
  },
  fetchModel: async (url) => {
    const r = await fetch(url);
    if (!r.ok) throw new Error(`model fetch failed: ${r.status}`);
    return new Uint8Array(await r.arrayBuffer());
  },
});

self.onmessage = async ({ data }: MessageEvent<DspIn>) => {
  for (const r of await handle(data)) {
    (self as unknown as Worker).postMessage(r, r.type === 'frame' ? [r.data.buffer] : []);
  }
};
