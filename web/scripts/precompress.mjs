// Writes `<file>.br` and `<file>.gz` next to every text-like file in a built dist/, so the server can send them as they
// are (ServeDir's precompressed variants, picked by Accept-Encoding). Formats that are already compressed (images,
// fonts, archives) get none, and neither does a file the compression would not shrink. A variant already there is kept. Run: node scripts/precompress.mjs dist
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';

const TEXT = /\.(?:html|js|mjs|css|json|webmanifest|svg|txt|wasm)$/;
export const compressible = (name) => TEXT.test(name);

/** Every compressible file under `dir`, recursively; returns the paths written. */
export function precompress(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...precompress(p));
    else if (compressible(name)) {
      const raw = readFileSync(p);
      const variants = [
        ['.br', brotliCompressSync(raw, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: raw.length } })],
        ['.gz', gzipSync(raw, { level: 9 })],
      ];
      for (const [ext, z] of variants) {
        // A copy made earlier stays (build-dsp.sh writes the wasm's .gz): one source per variant.
        if (z.length >= raw.length || existsSync(p + ext)) continue;
        writeFileSync(p + ext, z);
        out.push(p + ext);
      }
    }
  }
  return out;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const n = precompress(process.argv[2] ?? 'dist').length;
  console.log(`precompressed: ${n} files`);
}
