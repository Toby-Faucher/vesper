import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  server: { proxy: { '/ws': { target: 'ws://localhost:8080', ws: true } } },
  test: { include: ['src/**/*.test.ts'] },
});
