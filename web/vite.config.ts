import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  // Proxied requests keep the browser's Origin (http://localhost:5173), so the server needs VESPER_ORIGIN set to it.
  server: { proxy: { '/ws': { target: 'ws://localhost:8080', ws: true }, '/api': 'http://localhost:8080' } },
  test: { include: ['src/**/*.test.ts'] },
});
