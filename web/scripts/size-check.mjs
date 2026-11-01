// Fails when the built JS or CSS in dist/assets grows past its gzip budget.
// The wasm and the model live in dist/dsp, not dist/assets, so they are not counted.
// Raise a budget on purpose, in the same commit as the change that needs it.
import { readdirSync, readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

// Measured on 2026-10-30: js 29.5 kB, css 2.8 kB gzip. Budgets were those plus about 25 percent.
// Raised on 2026-11-01 for the chat panel (js 41.4 kB, css 3.4 kB): about 10 percent headroom.
// Raised on 2026-11-01 for the design tokens and base styles (css 4.4 kB): about 10 percent headroom.
// Raised on 2026-11-01 for the primitives (css 5.3 kB, of which about 0.4 kB are classes only the dev gallery uses, since
// Tailwind scans routes/dev too): about 10 percent headroom. The components are not imported yet, so js is unchanged.
// Raised on 2026-11-01 for the app shell, sidebar, routes and the first primitives in use (js 59.9 kB, of which about 5 kB
// are animejs waapi and 6 kB the lazy Settings chunk; css 6.4 kB): about 10 percent headroom.
// Raised on 2026-11-01 for the call bar and the person card (css 7.5 kB, of which 0.7 kB is the lazy person card chunk with
// the dialog and range styles): about 10 percent headroom. js measured 65.2 kB fits the old 66 and stays.
// Raised on 2026-11-01 for the Task 4 fix round (js 66.8 kB, was 65.2): icons became one export each (unused ones drop,
// -0.5 kB) and Chat with the markdown renderer loads on the first room open. That cut the first load from 55.2 to 51.0 kB,
// but Rolldown then moves the Svelte runtime into a shared chunk, which costs about 2 kB in this sum: about 10 percent headroom.
// Raised on 2026-11-01 for the Ctrl/Cmd+K switcher and the shortcuts (js 73.6 kB, was 67.0; first load 52.9 kB, was 51.1):
// the switcher, its matcher and the shortcuts list are one lazy chunk (4.6 kB) loaded on the first Ctrl+K, and the dialog
// code became a shared chunk. Ten percent would pass the 80 kB ceiling, so the budget is the ceiling: later tasks have 6.4 kB.
const BUDGET_GZIP_KB = { js: 80, css: 8.2 };

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
