// Fails when the built app passes its gzip budgets. Five numbers count (Rulings of 2026-11-01):
// - first load JS: the entry chunk in dist/index.html, its modulepreloads and every chunk they import statically;
// - the largest lazy chunk: any other JS file (dynamic import() chunks and the workers);
// - the sum of all JS in dist/assets;
// - first load CSS: the stylesheets dist/index.html links (Vite puts the CSS of the entry's static chunks there; a lazy
//   chunk's CSS loads with that chunk), and the sum of all CSS.
// The wasm and the model live in dist/dsp, not dist/assets, so they are not counted.
// Raise a budget on purpose, in the same commit as the change that needs it.
import { readdirSync, readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { posix } from 'node:path';
import { pathToFileURL } from 'node:url';

// History: the budget was one summed JS number (29.5 kB on 2026-10-30, 73.7 kB at Task 6) against an estimated 80 kB
// ceiling. On 2026-11-01 (Task 6 fix round) it became first load / lazy chunk / sum / CSS, because with lazy chunks
// the sum is not what a user waits for. Measured then: first load 52.9, largest lazy 6.0, sum 73.7, css 8.1 kB.
// Ruling #2 (2026-11-01, UI Task 9 fix round): CSS is judged like JS, first load 12 and sum 20 (measured 11.1 then).
export const BUDGET_GZIP_KB = { first: 70, lazy: 35, sum: 170, cssFirst: 12, css: 20 };
// The ratchet: first load may not grow back. Trimmed on 2026-11-01 (#88) from 68.8 to 55.9 kB; the limit is that plus
// 1 kB. Lower it when the first load shrinks; raise it only on purpose, in the commit that needs it, and say why.
// 2026-11-03: 56.9 to 57.5 for direct messages (the DM list in state.ts, the sidebar section and row, findRoom); measured
// 57.0 on the dev machine and 57.2 in CI, whose gzip output is a little larger.
// 2026-11-03: 57.5 to 57.9 for v1.6.0 (the DM sort, the ring state and its small shell hook, the DM switch); measured 57.6 on
// the dev machine, so CI is expected near 57.8.
export const RATCHET_GZIP_KB = { first: 57.9 };

// Static imports and re-exports in a built chunk: `import{a as b}from"./x.js"`, `import"./x.js"`, `export*from"./x.js"`.
// import("./x.js") is dynamic and left out: the `(` cannot follow `import` here.
const STATIC = /\b(?:import|export)\s*(?:[\w$*{}\s,]*?\bfrom\s*)?["']([^"']+\.js)["']/g;

/** Paths (relative to dist, like "assets/index.js") of the JS a first visit loads before anything is clicked. */
export function firstLoad(dist) {
  const html = readFileSync(posix.join(dist, 'index.html'), 'utf8');
  const roots = [...html.matchAll(/<(?:script[^>]*\bsrc|link[^>]*rel="modulepreload"[^>]*\bhref)="\/?([^"]+\.js)"/g)];
  const seen = new Set();
  const todo = roots.map((m) => m[1]);
  while (todo.length) {
    const f = todo.pop();
    if (seen.has(f)) continue;
    seen.add(f);
    for (const m of readFileSync(posix.join(dist, f), 'utf8').matchAll(STATIC)) {
      if (m[1].startsWith('.')) todo.push(posix.join(posix.dirname(f), m[1]));
    }
  }
  return seen;
}

/** Paths of the stylesheets a first visit loads: the ones dist/index.html links. */
export function firstCss(dist) {
  const html = readFileSync(posix.join(dist, 'index.html'), 'utf8');
  return new Set([...html.matchAll(/<link[^>]*rel="stylesheet"[^>]*\bhref="\/?([^"]+\.css)"/g)].map((m) => m[1]));
}

/** Gzip sizes in bytes: { first, lazy (largest), lazyName, sum, cssFirst, css }. */
export function measure(dist) {
  const first = firstLoad(dist);
  const css = firstCss(dist);
  const r = { first: 0, lazy: 0, lazyName: '', sum: 0, cssFirst: 0, css: 0 };
  for (const name of readdirSync(posix.join(dist, 'assets'))) {
    const f = `assets/${name}`;
    const n = gzipSync(readFileSync(posix.join(dist, f))).length;
    if (name.endsWith('.css')) r.css += n;
    if (css.has(f)) r.cssFirst += n;
    if (!name.endsWith('.js')) continue;
    r.sum += n;
    if (first.has(f)) r.first += n;
    else if (n > r.lazy) Object.assign(r, { lazy: n, lazyName: name });
  }
  return r;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const r = measure(new URL('../dist/', import.meta.url).pathname);
  let bad = false;
  for (const [k, max] of Object.entries(BUDGET_GZIP_KB)) {
    const kb = r[k] / 1024;
    const cap = Math.min(max, RATCHET_GZIP_KB[k] ?? Infinity);
    const label = { first: 'first load js', lazy: `largest lazy js (${r.lazyName})`, sum: 'all js', cssFirst: 'first load css', css: 'all css' }[k];
    console.log(`${label}: ${kb.toFixed(1)} kB gzip (budget ${max}${cap < max ? `, ratchet ${cap}` : ''})`);
    if (kb > cap) bad = true;
  }
  if (bad) console.error('size-check: over budget');
  process.exit(bad ? 1 : 0);
}
