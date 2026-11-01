import { expect, it } from 'vitest';

// Every string the app shows goes through Svelte text nodes. These sinks would parse a string as HTML.
const SINKS = [
  '{@html', 'innerHTML', 'outerHTML', 'insertAdjacentHTML', 'document.write', 'srcdoc', 'setHTMLUnsafe', 'parseHTMLUnsafe',
  'createContextualFragment', 'DOMParser', 'eval(', 'new Function', 'javascript:',
];
const files = import.meta.glob(['../**/*.svelte', '../**/*.ts', '!../**/*.test.ts', '!./gen/**'], {
  query: '?raw', import: 'default', eager: true,
}) as Record<string, string>;

it('no_raw_html', () => {
  const names = Object.keys(files);
  expect(names.length).toBeGreaterThan(20);
  for (const f of ['../routes/MessageItem.svelte', '../routes/Markdown.svelte', './chat.svelte.ts', './markdown.ts']) expect(names).toContain(f);
  expect(files['../routes/MessageItem.svelte']).toContain('<Markdown'); // the raw source, not compiled output
  const hits = names.flatMap((f) => SINKS.filter((s) => files[f].includes(s)).map((s) => `${f}: ${s}`));
  expect(hits).toEqual([]);
});
