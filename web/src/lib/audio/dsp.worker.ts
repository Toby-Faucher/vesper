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

// Control messages arrive on self; audio frames on the port the capture hands over after 'ready',
// so the audio worklet talks to this worker directly and replies go back the same way.
const serve = (to: MessagePort | Worker) => async ({ data }: MessageEvent<DspIn>) => {
  for (const r of await handle(data)) to.postMessage(r, r.type === 'frame' ? [r.data.buffer] : []);
};
const main = serve(self as unknown as Worker);
self.onmessage = (e: MessageEvent<DspIn | { type: 'port' }>) => {
  if (e.data.type !== 'port') return main(e as MessageEvent<DspIn>);
  e.ports[0].onmessage = serve(e.ports[0]);
};
