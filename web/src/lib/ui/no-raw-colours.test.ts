import { expect, it } from 'vitest';

// Colours come only from the roles in styles/tokens.css. Hex, rgb(), hsl(), oklch() and Tailwind palette classes elsewhere fail.
// avatar.ts is the one exception: SLOT_HEX holds the vesper swatch hexes that UserInfo.accent stores, and avatar.test.ts
// checks they equal --avatar-1..8 in tokens.css.
const RULES = [
  /#[0-9a-fA-F]{3,8}\b/,
  /\brgba?\(/,
  /\bhsla?\(/,
  /\boklch\(/,
  /\b(neutral|zinc|gray|slate|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|pink|rose)-\d{2,3}\b/,
];
const files = import.meta.glob(['../../**/*.svelte', '../../**/*.ts', '!../../**/*.test.ts', '!../../styles/**', '!../gen/**', '!./avatar.ts'], {
  query: '?raw', import: 'default', eager: true,
}) as Record<string, string>;

it('no_raw_colours', () => {
  const names = Object.keys(files);
  expect(names).toContain('../../routes/Room.svelte');
  expect(names).toContain('../../main.ts');
  const hits = names.flatMap((f) =>
    files[f].split('\n').flatMap((line, i) => RULES.filter((r) => r.test(line)).map((r) => `${f}:${i + 1}: ${line.match(r)![0]}`)),
  );
  expect(hits).toEqual([]);
});
