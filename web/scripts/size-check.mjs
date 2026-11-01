// Fails when the built app passes its gzip budgets. Four numbers count (Ruling of 2026-11-01):
// - first load: the entry chunk in dist/index.html, its modulepreloads and every chunk they import statically;
// - the largest lazy chunk: any other JS file (dynamic import() chunks and the workers);
// - the sum of all JS in dist/assets, and the sum of all CSS.
// The wasm and the model live in dist/dsp, not dist/assets, so they are not counted.
// Raise a budget on purpose, in the same commit as the change that needs it.
import { readdirSync, readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { posix } from 'node:path';
import { pathToFileURL } from 'node:url';

// History: the budget was one summed JS number (29.5 kB on 2026-10-30, 73.7 kB at Task 6) against an estimated 80 kB
// ceiling. On 2026-11-01 (Task 6 fix round) it became first load / lazy chunk / sum / CSS, because with lazy chunks
// the sum is not what a user waits for. Measured then: first load 52.9, largest lazy 6.0, sum 73.7, css 8.1 kB.
export const BUDGET_GZIP_KB = { first: 70, lazy: 35, sum: 150, css: 14 };

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

/** Gzip sizes in bytes: { first, lazy (largest), lazyName, sum, css }. */
export function measure(dist) {
  const first = firstLoad(dist);
  const r = { first: 0, lazy: 0, lazyName: '', sum: 0, css: 0 };
  for (const name of readdirSync(posix.join(dist, 'assets'))) {
    const f = `assets/${name}`;
    const n = gzipSync(readFileSync(posix.join(dist, f))).length;
    if (name.endsWith('.css')) r.css += n;
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
    const label = { first: 'first load js', lazy: `largest lazy js (${r.lazyName})`, sum: 'all js', css: 'all css' }[k];
    console.log(`${label}: ${kb.toFixed(1)} kB gzip (budget ${max})`);
    if (kb > max) bad = true;
  }
  if (bad) console.error('size-check: over budget');
  process.exit(bad ? 1 : 0);
}
