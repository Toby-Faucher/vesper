import type { Environment } from 'vitest/runtime';

// Node globals, but Vite transforms the files as client code, so Svelte compiles $effect for the browser and runs it.
export default { name: 'client-node', viteEnvironment: 'client', setup: () => ({ teardown() {} }) } satisfies Environment;
