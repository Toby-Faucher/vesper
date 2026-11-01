// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';
import { firstLoad, measure } from './size-check.mjs';

const dist = new URL('./fixture-dist/', import.meta.url).pathname;
const gz = (f) => gzipSync(readFileSync(dist + f)).length;

describe('size-check', () => {
  it('first load is the entry, its modulepreloads and their static imports, not dynamic import() chunks', () => {
    expect([...firstLoad(dist)].sort()).toEqual([
      'assets/deep-g.js',
      'assets/index-a.js',
      'assets/runtime-b.js',
      'assets/side-f.js',
    ]);
  });

  it('sums first load, all js and all css, and names the largest lazy chunk', () => {
    const r = measure(dist);
    const js = readdirSync(dist + 'assets').filter((f) => f.endsWith('.js'));
    expect(r.first).toBe(['index-a', 'runtime-b', 'side-f', 'deep-g'].reduce((s, f) => s + gz(`assets/${f}.js`), 0));
    expect(r.sum).toBe(js.reduce((s, f) => s + gz(`assets/${f}`), 0));
    expect(r.css).toBe(gz('assets/index-c.css') + gz('assets/Lazy-i.css'));
    expect(r.lazyName).toBe('shared-e.js');
    expect(r.lazy).toBe(gz('assets/shared-e.js'));
  });
});
