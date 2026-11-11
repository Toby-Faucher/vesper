import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig(({ mode }) => ({
  plugins: [tailwindcss(), svelte()],
  // The call code is one lazy chunk, also for the settings and diagnostics pages that share parts of it: split per
  // module it cost 0.6 kB gzip more in all, in small chunks that compress badly (measured 2026-11-01).
  // The share panel and its tiles are in it too: they show only in a call, and alone they split Icon.svelte out of the
  // first load (measured 2026-11-09).
  build: { rolldownOptions: { output: { codeSplitting: { includeDependenciesRecursively: false, groups: [{ name: 'call', test: /\/src\/(lib\/(call-stack|join|playback|diag|audio\/capture|audio\/opus|ui\/link-stats|ui\/quality|share\/(pip|geometry|struggle|line|icons|watch))\.ts|routes\/(CallBar|SharePanel|ShareTile)\.svelte)/ }] } } } },
  // Proxied requests keep the browser's Origin (http://localhost:5173), so the server needs VESPER_ORIGIN set to it.
  server: { proxy: { '/ws': { target: 'ws://localhost:8080', ws: true }, '/api': 'http://localhost:8080' } },
  // css: tokens.test.ts reads tokens.css and app.css as ?raw text; vitest returns CSS as empty otherwise.
  // client-env, the browser condition and inlining svelte: tests get Svelte's client build, so $effect runs (voice.test.ts).
  test: { include: ['src/**/*.test.ts', 'scripts/*.test.mjs'], css: true, environment: './src/client-env.ts', server: { deps: { inline: ['svelte'] } } },
  resolve: mode === 'test' ? { conditions: ['browser'] } : undefined,
}));
