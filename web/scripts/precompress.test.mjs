import { mkdtempSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { brotliDecompressSync, gunzipSync } from 'node:zlib';
import { describe, expect, it } from 'vitest';
import { compressible, precompress } from './precompress.mjs';

describe('precompress', () => {
  it('writes .br and .gz for text files only, and they unpack to the same bytes', () => {
    const d = mkdtempSync(join(tmpdir(), 'precompress-'));
    mkdirSync(join(d, 'assets'));
    const js = 'export const a = "hello hello hello hello hello";\n'.repeat(50);
    writeFileSync(join(d, 'assets', 'x.js'), js);
    writeFileSync(join(d, 'index.html'), '<!doctype html>' + '<p>x</p>'.repeat(50));
    writeFileSync(join(d, 'icon.png'), Buffer.alloc(500, 1));
    writeFileSync(join(d, 'tiny.css'), 'a{}'); // would grow: left alone
    writeFileSync(join(d, 'assets', 'w.wasm'), Buffer.alloc(4000, 7));
    writeFileSync(join(d, 'assets', 'w.wasm.gz'), 'FROM BUILD-DSP'); // made earlier: kept, the .br is added
    precompress(d);
    expect(readFileSync(join(d, 'assets', 'w.wasm.gz'), 'utf8')).toBe('FROM BUILD-DSP');
    expect(readdirSync(join(d, 'assets'))).toContain('w.wasm.br');
    expect(readdirSync(d).sort()).toEqual(['assets', 'icon.png', 'index.html', 'index.html.br', 'index.html.gz', 'tiny.css']);
    expect(brotliDecompressSync(readFileSync(join(d, 'assets', 'x.js.br'))).toString()).toBe(js);
    expect(gunzipSync(readFileSync(join(d, 'assets', 'x.js.gz'))).toString()).toBe(js);
  });
  it('never for formats that are compressed already', () => {
    for (const f of ['a.png', 'a.jpg', 'a.webp', 'a.woff2', 'model.tar.gz', 'a.js.br', 'a.js.gz']) expect(compressible(f), f).toBe(false);
    for (const f of ['a.js', 'a.css', 'index.html', 'manifest.webmanifest', 'a.svg', 'vesper_dsp_bg.wasm']) expect(compressible(f), f).toBe(true);
  });
});
