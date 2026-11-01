// Fails when the built JS or CSS in dist/assets grows past its gzip budget.
// The wasm and the model live in dist/dsp, not dist/assets, so they are not counted.
// Raise a budget on purpose, in the same commit as the change that needs it.
import { readdirSync, readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

// Measured on 2026-10-30: js 29.5 kB, css 2.8 kB gzip. Budgets were those plus about 25 percent.
// Raised on 2026-11-01 for the chat panel (js 41.4 kB, css 3.4 kB): about 10 percent headroom.
const BUDGET_GZIP_KB = { js: 45, css: 4 };

const dir = new URL('../dist/assets/', import.meta.url);
const sums = { js: 0, css: 0 };
for (const f of readdirSync(dir)) {
  const ext = f.endsWith('.js') ? 'js' : f.endsWith('.css') ? 'css' : null;
  if (ext) sums[ext] += gzipSync(readFileSync(new URL(f, dir))).length;
}
let bad = false;
for (const [k, max] of Object.entries(BUDGET_GZIP_KB)) {
  const kb = sums[k] / 1024;
  console.log(`${k}: ${kb.toFixed(1)} kB gzip (budget ${max})`);
  if (kb > max) bad = true;
}
if (bad) console.error('size-check: over budget');
process.exit(bad ? 1 : 0);
