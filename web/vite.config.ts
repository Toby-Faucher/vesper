import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig(({ mode }) => ({
  plugins: [tailwindcss(), svelte()],
  // Proxied requests keep the browser's Origin (http://localhost:5173), so the server needs VESPER_ORIGIN set to it.
  server: { proxy: { '/ws': { target: 'ws://localhost:8080', ws: true }, '/api': 'http://localhost:8080' } },
  // css: tokens.test.ts reads tokens.css and app.css as ?raw text; vitest returns CSS as empty otherwise.
  // client-env, the browser condition and inlining svelte: tests get Svelte's client build, so $effect runs (voice.test.ts).
  test: { include: ['src/**/*.test.ts', 'scripts/*.test.mjs'], css: true, environment: './src/client-env.ts', server: { deps: { inline: ['svelte'] } } },
  resolve: mode === 'test' ? { conditions: ['browser'] } : undefined,
}));
