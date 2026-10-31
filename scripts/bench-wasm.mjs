// Real-browser speed of the shipped WasmDenoiser. Needs web/public/dsp (scripts/build-dsp.sh) and Playwright.
// usage: node scripts/bench-wasm.mjs <chromium|firefox> [cpuThrottle=1] [wav-dir]   -> one JSON line (throttle > 1 runs on the main thread)
// wav-dir defaults to DeepFilterNet/assets (clean_freesound_33711.wav + noise_freesound_573577.wav); needs 16 bit mono 48 kHz.
import { createServer } from 'node:http';
import { readFileSync, existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { dirname, join, extname } from 'node:path';

const [browserName = 'chromium', throttle = '1', assets = process.env.HOME + '/Projects/friendchat-spike/DeepFilterNet/assets'] = process.argv.slice(2);
const root = join(dirname(fileURLToPath(import.meta.url)), '../web/public');
const pw = createRequire(process.env.PW_PKG ?? join(process.cwd(), 'web/package.json'))('playwright-core');
const WARM = 300, FRAMES = 2000, N = 480; // 20 s

const wav = (f) => { const b = readFileSync(join(assets, f)); const o = b.indexOf('data') + 8; return Float32Array.from({ length: (b.length - o) >> 1 }, (_, i) => b.readInt16LE(o + 2 * i) / 32768); };
const clean = wav('clean_freesound_33711.wav'), noise = wav('noise_freesound_573577.wav');
const rms = (a) => Math.sqrt(a.reduce((s, x) => s + x * x, 0) / a.length);
const g = rms(clean) / rms(noise) / 10 ** (5 / 20); // 5 dB SNR
const mix = new Float32Array((WARM + FRAMES) * N).map((_, i) => clean[i % clean.length] + g * noise[i % noise.length]);

const workerJs = `
const [base, mixUrl, warm, frames, n] = ${JSON.stringify(['/dsp/', '/mix.f32', WARM, FRAMES, N])};
const m = await import(base + 'vesper_dsp.js');
await m.default({ module_or_path: base + 'vesper_dsp_bg.wasm' });
const model = new Uint8Array(await (await fetch(base + 'model.tar.gz')).arrayBuffer());
const mix = new Float32Array(await (await fetch(mixUrl)).arrayBuffer());
const d = new m.WasmDenoiser(model, 100, 0.02);
const ms = new Float64Array(frames);
for (let i = 0; i < warm + frames; i++) {
  const f = mix.slice(i * n, (i + 1) * n);
  const t = performance.now(); d.process(f); const dt = performance.now() - t;
  if (i >= warm) ms[i - warm] = dt;
}
(self.window ? self.__r : postMessage)({ ms });`;

const types = { '.js': 'text/javascript', '.wasm': 'application/wasm', '.html': 'text/html' };
const srv = createServer((req, res) => {
  const h = { 'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'require-corp' }; // fine performance.now in Firefox
  if (req.url === '/') { res.writeHead(200, { ...h, 'Content-Type': 'text/html' }).end('<!doctype html><title>b</title>'); return; }
  if (req.url === '/worker.js') { res.writeHead(200, { ...h, 'Content-Type': 'text/javascript' }).end(workerJs); return; }
  if (req.url === '/mix.f32') { res.writeHead(200, h).end(Buffer.from(mix.buffer)); return; }
  const p = join(root, req.url.split('?')[0]);
  if (!p.startsWith(root) || !existsSync(p)) { res.writeHead(404).end(); return; }
  res.writeHead(200, { ...h, 'Content-Type': types[extname(p)] ?? 'application/octet-stream' }).end(readFileSync(p));
});
await new Promise((r) => srv.listen(0, '127.0.0.1', r));
let browser;
try {
  browser = browserName === 'firefox' ? await pw.firefox.launch({ headless: true })
    : await pw.chromium.launch({ headless: true, executablePath: process.env.HOME + '/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome', args: ['--no-sandbox'] });
  const page = await browser.newPage();
  if (+throttle > 1) await (await page.context().newCDPSession(page)).send('Emulation.setCPUThrottlingRate', { rate: +throttle });
  await page.goto(`http://127.0.0.1:${srv.address().port}/`);
  // CDP throttling does not reach workers, so a throttled run uses the page's main thread instead.
  const r = await page.evaluate((main) => new Promise((ok, no) => {
    if (main) { window.__r = (d) => ok({ ms: Array.from(d.ms) }); import('/worker.js').catch(no); return; }
    const w = new Worker('/worker.js', { type: 'module' });
    w.onmessage = (e) => ok({ ms: Array.from(e.data.ms) });
    w.onerror = (e) => no(new Error(e.message));
  }), +throttle > 1);
  const s = r.ms.slice().sort((a, b) => a - b), q = (p) => s[Math.min(s.length - 1, Math.floor(p * s.length))];
  const sum = r.ms.reduce((a, b) => a + b, 0);
  console.log(JSON.stringify({ browser: browserName, throttle: +throttle, median: q(0.5), p95: q(0.95), p99: q(0.99), max: s.at(-1), over10: s.filter((x) => x > 10).length, over30: s.filter((x) => x > 30).length, rtf: sum / r.ms.length / 10, distinctValues: new Set(r.ms).size }));
} finally { await browser?.close(); srv.close(); }
